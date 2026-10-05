//! Remote mode: runs one command against a koldera-core server through the API client.

use std::collections::HashMap;
use std::io::Write;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

use crate::api::{Api, ApiError, Created, HtmlJobRequest, PdfJobRequest, WebJobRequest};
use crate::args::{Command, usage_of};
use crate::document::{self, DocumentError};
use crate::model::Job;
use crate::output::{self, safe_name};
use crate::{exit, summary};

const CLEAR_TEXT_WARNING: &str =
    "koldera: warning: the server URL is not HTTPS; the token is sent in clear text";

/// The one-line warning for a plain `http://` server that is not on this machine.
pub fn clear_text_warning(base: &str) -> Option<&'static str> {
    if !base.to_ascii_lowercase().starts_with("http://") {
        return None;
    }
    let host = host_of(base)?;
    let loopback = host == "localhost"
        || host.parse::<Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
        || host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .and_then(|h| h.parse::<Ipv6Addr>().ok())
            .is_some_and(|ip| ip.is_loopback());
    (!loopback).then_some(CLEAR_TEXT_WARNING)
}

/// The lower-case host of an absolute URL (an IPv6 address keeps its brackets), like .NET's `Uri.Host`.
pub fn host_of(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = if host_port.starts_with('[') {
        &host_port[..=host_port.find(']')?]
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    (!host.is_empty()).then(|| host.to_lowercase())
}

/// Where a failed exchange happened, which decides its exit code and hint.
#[derive(Clone, Copy, PartialEq)]
enum Stage {
    /// Declaring an upload: a 400/422 is a problem with the request itself.
    Declare,
    /// Creating the job: a 400/422 is a problem with the request itself (for a document, with its content).
    Request,
    /// Any other API call.
    Api,
    /// Signed storage URLs (upload and download): never a credential problem.
    Storage,
}

enum Failure {
    /// Exit code with the lines already written to stderr.
    Exit(i32),
    Api(ApiError, Stage),
}

impl From<i32> for Failure {
    fn from(code: i32) -> Failure {
        Failure::Exit(code)
    }
}

fn at(stage: Stage) -> impl Fn(ApiError) -> Failure {
    move |e| Failure::Api(e, stage)
}

/// Runs `command` (pdf, html, scan or document) on the server; returns the exit code.
pub fn run(api: &Api, command: &Command, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let document_file = match command {
        Command::Document { file, .. } => Some(file.as_str()),
        _ => None,
    };
    match execute(api, command, stdout, stderr) {
        Ok(code) | Err(Failure::Exit(code)) => code,
        Err(Failure::Api(e, stage)) => report(&e, stage, document_file, stderr),
    }
}

fn execute(
    api: &Api,
    command: &Command,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<i32, Failure> {
    let (created, stem, out, json) = match command {
        Command::Pdf {
            file,
            fix,
            language,
            title,
            out,
            json,
        } => {
            let upload_id = upload_input(api, "pdf", file, "application/pdf", stderr)?;
            let mut body = PdfJobRequest::new(&upload_id, if *fix { "fix" } else { "analyze" });
            body.explicit_language = language.clone();
            body.explicit_title = title.clone();
            let created = api.create_pdf_job(&body).map_err(at(Stage::Request))?;
            (created, file_stem(file), out, *json)
        }
        Command::Html {
            file,
            analyze_only,
            content_selector,
            out,
            json,
        } => {
            let upload_id = upload_input(api, "html", file, "text/html", stderr)?;
            let mode = if *analyze_only {
                "generate_analyze"
            } else {
                "generate_fix"
            };
            let mut body = HtmlJobRequest::new(&upload_id, mode);
            body.content_selector = content_selector.clone();
            let created = api.create_html_job(&body).map_err(at(Stage::Request))?;
            (created, file_stem(file), out, *json)
        }
        Command::Scan { url, out, json } => {
            let created = api
                .create_web_job(&WebJobRequest::new(url))
                .map_err(at(Stage::Request))?;
            let host = created
                .job
                .target_url
                .as_deref()
                .and_then(host_of)
                .or_else(|| host_of(url))
                .unwrap_or_default();
            (created, safe_name(&host), out, *json)
        }
        Command::Document { file, out, json } => {
            let created = run_document(api, file, stderr)?;
            (created, file_stem(file), out, *json)
        }
        Command::Help(_) | Command::Version => return Ok(exit::OK),
    };
    let job = finish(api, created)?;
    let findings = api.findings(&job.id).map_err(at(Stage::Api))?;
    let actions = api.actions(&job.id).map_err(at(Stage::Api))?;
    let artifacts = api.artifacts(&job.id).map_err(at(Stage::Api))?;
    let mut items = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        let bytes = api.download(&artifact).map_err(at(Stage::Storage))?;
        items.push((artifact, bytes));
    }
    let out_dir = match out {
        Some(dir) => PathBuf::from(dir),
        None => std::env::current_dir().unwrap_or_default(),
    };
    let written = match output::write_artifacts(&out_dir, &stem, items) {
        Ok(w) => w,
        Err(e) => {
            let _ = writeln!(stderr, "koldera: could not write the output: {e}");
            return Ok(exit::JOB_FAILED);
        }
    };
    let text = if json {
        summary::json(&job, &findings, &actions, &written) + "\n"
    } else {
        summary::text(&job, &findings, &actions, &written)
    };
    let _ = stdout.write_all(text.as_bytes());
    let _ = stdout.flush();
    Ok(exit::code_for(&job))
}

fn finish(api: &Api, created: Created) -> Result<Job, Failure> {
    if created.finished {
        return Ok(created.job);
    }
    api.wait_job(&created.job.id).map_err(at(Stage::Api))
}

/// The input file name without its extension, as core names the outputs.
fn file_stem(file: &str) -> String {
    let stem = Path::new(file)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    safe_name(&stem)
}

fn refuse(stderr: &mut dyn Write, command: &str, message: &str) -> Failure {
    let _ = writeln!(stderr, "koldera: {message}");
    let _ = writeln!(stderr, "{}", usage_of(command));
    Failure::Exit(exit::USAGE)
}

fn read_input(command: &str, file: &str, stderr: &mut dyn Write) -> Result<Vec<u8>, Failure> {
    if !Path::new(file).is_file() {
        return Err(refuse(stderr, command, &format!("file not found: {file}")));
    }
    std::fs::read(file).map_err(|e| refuse(stderr, command, &format!("cannot read {file}: {e}")))
}

fn upload_input(
    api: &Api,
    command: &str,
    file: &str,
    content_type: &str,
    stderr: &mut dyn Write,
) -> Result<String, Failure> {
    let bytes = read_input(command, file, stderr)?;
    let name = Path::new(file)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    upload(api, &name, content_type, &bytes)
}

fn upload(api: &Api, name: &str, content_type: &str, bytes: &[u8]) -> Result<String, Failure> {
    let ticket = api
        .create_upload(name, content_type, bytes)
        .map_err(at(Stage::Declare))?;
    api.put_upload(&ticket, bytes).map_err(at(Stage::Storage))?;
    Ok(ticket.upload_id)
}

fn run_document(api: &Api, file: &str, stderr: &mut dyn Write) -> Result<Created, Failure> {
    let (mut doc, images) = match document::prepare(Path::new(file)) {
        Ok(prepared) => prepared,
        Err(DocumentError::NotFound) => {
            return Err(refuse(
                stderr,
                "document",
                &format!("file not found: {file}"),
            ));
        }
        Err(DocumentError::Unreadable(e)) => {
            return Err(refuse(
                stderr,
                "document",
                &format!("cannot read {file}: {e}"),
            ));
        }
        Err(DocumentError::NotJson(e)) => {
            let _ = writeln!(stderr, "koldera: {file} is not valid JSON: {e}");
            return Err(exit::USAGE.into());
        }
        Err(DocumentError::Invalid(errors)) => {
            let _ = stderr.write_all(document::invalid_text(file, &errors).as_bytes());
            return Err(exit::USAGE.into());
        }
    };
    let mut uploaded: HashMap<String, String> = HashMap::new();
    for image in &images {
        // Paths compare case-insensitively on Windows, like core's upload map.
        let mut key = image.path.to_string_lossy().into_owned();
        if cfg!(windows) {
            key = key.to_lowercase();
        }
        let id = match uploaded.get(&key) {
            Some(id) => id.clone(),
            None => {
                let shown = image.path.display().to_string();
                let bytes = std::fs::read(&image.path).map_err(|e| {
                    refuse(stderr, "document", &format!("cannot read {shown}: {e}"))
                })?;
                let name = image
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let id = upload(api, &name, image.content_type, &bytes)?;
                uploaded.insert(key, id.clone());
                id
            }
        };
        document::use_upload(&mut doc, &image.pointer, &id);
    }
    api.create_document_job(&doc).map_err(at(Stage::Request))
}

/// Prints a failed exchange and returns its exit code: 64 for a request the server found invalid, else 3.
fn report(e: &ApiError, stage: Stage, document_file: Option<&str>, stderr: &mut dyn Write) -> i32 {
    let invalid_request =
        matches!(stage, Stage::Request | Stage::Declare) && matches!(e.status, Some(400 | 422));
    if invalid_request && stage == Stage::Request {
        if let (Some(file), false) = (document_file, e.errors.is_empty()) {
            let _ = stderr.write_all(document::invalid_text(file, &e.errors).as_bytes());
            return exit::USAGE;
        }
    }
    let _ = writeln!(stderr, "koldera: {}: {}", e.code, e.message);
    for f in &e.errors {
        let path = if f.path.is_empty() {
            "(document)"
        } else {
            &f.path
        };
        let _ = writeln!(stderr, "  {path}  {}: {}", f.code, f.message);
    }
    if invalid_request {
        return exit::USAGE;
    }
    if stage != Stage::Storage && matches!(e.status, Some(401 | 403)) {
        let _ = writeln!(
            stderr,
            "koldera: the server refused the request's credentials; check KOLDERA_TOKEN"
        );
    }
    exit::JOB_FAILED
}
