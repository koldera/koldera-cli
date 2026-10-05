//! An in-process HTTP/1.1 server on `std::net::TcpListener` that answers scripted replies and records requests.
#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    /// Path including the query, as sent.
    pub target: String,
    /// Header names in lower case.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Read the request, then close the connection without answering.
    pub drop: bool,
    /// Read the request, then hold the connection open without answering.
    pub hang: bool,
}

impl Reply {
    pub fn status(status: u16) -> Reply {
        Reply {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            drop: false,
            hang: false,
        }
    }

    pub fn json(status: u16, body: &Value) -> Reply {
        Reply::status(status)
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(body).unwrap())
    }

    pub fn dropped() -> Reply {
        Reply {
            drop: true,
            ..Reply::status(0)
        }
    }

    pub fn hang() -> Reply {
        Reply {
            hang: true,
            ..Reply::status(0)
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Reply {
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn without_header(mut self, name: &str) -> Reply {
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Reply {
        self.body = body.into();
        self
    }

    /// The `response` of `contracts/golden/<name>.json`; `{{baseUrl}}` is replaced when it is sent.
    pub fn golden(name: &str) -> Reply {
        let response = fixture(name)["response"].clone();
        let mut reply = Reply::status(response["status"].as_u64().unwrap() as u16);
        if let Some(headers) = response["headers"].as_object() {
            for (n, v) in headers {
                if !n.eq_ignore_ascii_case("Content-Length") {
                    reply = reply.header(n, v.as_str().unwrap());
                }
            }
        }
        if let Some(raw) = response["rawBody"].as_str() {
            reply.body = raw.chars().map(|c| c as u32 as u8).collect();
        } else if !response["body"].is_null() {
            reply.body = serde_json::to_vec(&response["body"]).unwrap();
        }
        reply
    }
}

pub fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/contracts/golden/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap()
}

type Routes = HashMap<String, VecDeque<Reply>>;

/// Replies are queued per `"METHOD /path"` (query ignored); the last reply of a route repeats.
pub struct TestServer {
    pub base: String,
    routes: Arc<Mutex<Routes>>,
    requests: Arc<Mutex<Vec<Recorded>>>,
}

impl TestServer {
    pub fn start() -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes: Arc<Mutex<Routes>> = Arc::default();
        let requests: Arc<Mutex<Vec<Recorded>>> = Arc::default();
        let (r, q, b) = (routes.clone(), requests.clone(), base.clone());
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (r, q, b) = (r.clone(), q.clone(), b.clone());
                thread::spawn(move || serve(stream, &r, &q, &b));
            }
        });
        TestServer {
            base,
            routes,
            requests,
        }
    }

    pub fn on(&self, route: &str, reply: Reply) -> &TestServer {
        self.routes
            .lock()
            .unwrap()
            .entry(route.to_owned())
            .or_default()
            .push_back(reply);
        self
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }

    pub fn requests_to(&self, route: &str) -> Vec<Recorded> {
        self.requests()
            .into_iter()
            .filter(|r| format!("{} {}", r.method, r.path()) == route)
            .collect()
    }

    /// A JSON value with `{{baseUrl}}` replaced by this server's origin.
    pub fn with_base(&self, value: &Value) -> Value {
        let text = serde_json::to_string(value)
            .unwrap()
            .replace("{{baseUrl}}", &self.base);
        serde_json::from_str(&text).unwrap()
    }
}

fn serve(stream: TcpStream, routes: &Mutex<Routes>, requests: &Mutex<Vec<Recorded>>, base: &str) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let Some(request) = read_request(&mut reader) else {
        return;
    };
    let route = format!("{} {}", request.method, request.path());
    requests.lock().unwrap().push(request);
    let reply = {
        let mut routes = routes.lock().unwrap();
        match routes.get_mut(&route) {
            Some(queue) if queue.len() > 1 => queue.pop_front().unwrap(),
            Some(queue) if !queue.is_empty() => queue[0].clone(),
            _ => Reply::status(599).body(format!("no reply scripted for {route}")),
        }
    };
    let mut stream = stream;
    if reply.hang {
        thread::sleep(std::time::Duration::from_secs(20));
        return;
    }
    if reply.drop {
        let _ = stream.shutdown(Shutdown::Both);
        return;
    }
    let body = replace_bytes(&reply.body, b"{{baseUrl}}", base.as_bytes());
    let mut head = format!("HTTP/1.1 {} Scripted\r\n", reply.status);
    for (n, v) in &reply.headers {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    head.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
}

fn read_request(reader: &mut BufReader<TcpStream>) -> Option<Recorded> {
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let target = parts.next()?.to_owned();
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).ok()?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        let (n, v) = h.split_once(':')?;
        headers.push((n.trim().to_ascii_lowercase(), v.trim().to_owned()));
    }
    let length = headers
        .iter()
        .find(|(n, _)| n == "content-length")
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(Recorded {
        method,
        target,
        headers,
        body,
    })
}

fn replace_bytes(haystack: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(haystack.len());
    let mut i = 0;
    while i < haystack.len() {
        if haystack[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(haystack[i]);
            i += 1;
        }
    }
    out
}
