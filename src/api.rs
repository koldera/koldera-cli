//! The HTTP client for the koldera-core API (`contracts/openapi.yaml`) used by remote mode.

mod error;

use std::fmt;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use ureq::Agent;
use ureq::http::Response;

pub use error::{
    ARTIFACT_MISMATCH, ApiError, FILE_TOO_LARGE, FieldError, MALFORMED_RESPONSE, TRANSPORT_ERROR,
};

use crate::args::UsageError;
use crate::model::{Action, Artifact, Finding, Items, Job};
use crate::output::sha256_hex;

pub const USER_AGENT: &str = concat!("koldera-cli/", env!("CARGO_PKG_VERSION"));
/// `Prefer: wait=N` on every job creation.
pub const WAIT_SECONDS: u64 = 60;
pub const API_TIMEOUT: Duration = Duration::from_secs(30);
pub const STORAGE_TIMEOUT: Duration = Duration::from_secs(600);
/// Repeats after a transport failure (creations, uploads, reads) or a 429/503 (reads).
pub const RETRIES: u32 = 3;
pub const PROFILE: &str = "ua1";

const MAX_JSON_BYTES: u64 = 16 << 20;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);
const FIRST_POLL: Duration = Duration::from_secs(1);
const MAX_POLL: Duration = Duration::from_secs(5);
const RETRY_STEP: Duration = Duration::from_millis(250);
const ALREADY_COMPLETED: &str = "UPLOAD_ALREADY_COMPLETED";

type Sleep = Arc<dyn Fn(Duration) + Send + Sync>;
type Warn = Arc<dyn Fn(&str) + Send + Sync>;

/// A signed upload target. `upload_url` is a credential: it is never serialized or printed.
#[derive(Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ticket {
    pub upload_id: String,
    pub upload_url: String,
    #[serde(default)]
    pub expires_at: String,
    #[serde(default)]
    pub max_bytes: Option<u64>,
    /// The content type declared for the upload; the PUT sends it.
    #[serde(skip)]
    pub content_type: String,
    /// The server replayed the ticket of an earlier attempt (`Idempotent-Replayed: true`).
    #[serde(skip)]
    pub replayed: bool,
}

impl fmt::Debug for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ticket")
            .field("upload_id", &self.upload_id)
            .field("upload_url", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .field("max_bytes", &self.max_bytes)
            .field("content_type", &self.content_type)
            .field("replayed", &self.replayed)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfJobRequest {
    pub upload_id: String,
    pub mode: String,
    pub profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explicit_language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explicit_title: Option<String>,
}

impl PdfJobRequest {
    pub fn new(upload_id: &str, mode: &str) -> PdfJobRequest {
        PdfJobRequest {
            upload_id: upload_id.to_owned(),
            mode: mode.to_owned(),
            profile: PROFILE.to_owned(),
            explicit_language: None,
            explicit_title: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HtmlJobRequest {
    pub upload_id: String,
    pub mode: String,
    pub profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_selector: Option<String>,
}

impl HtmlJobRequest {
    pub fn new(upload_id: &str, mode: &str) -> HtmlJobRequest {
        HtmlJobRequest {
            upload_id: upload_id.to_owned(),
            mode: mode.to_owned(),
            profile: PROFILE.to_owned(),
            content_selector: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WebJobRequest {
    pub url: String,
}

impl WebJobRequest {
    pub fn new(url: &str) -> WebJobRequest {
        WebJobRequest {
            url: url.to_owned(),
        }
    }
}

/// A created job; `finished` only for a `200` whose job is no longer queued or running.
#[derive(Debug, Clone, PartialEq)]
pub struct Created {
    pub job: Job,
    pub finished: bool,
}

pub fn is_terminal(state: &str) -> bool {
    !matches!(state, "queued" | "running")
}

struct Answer {
    status: u16,
    body: Vec<u8>,
    retry_after: Option<Duration>,
    replayed: bool,
    /// The body was longer than `MAX_JSON_BYTES` and was not read.
    too_large: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Call {
    Read,
    /// `POST /v1/uploads`: idempotent, not held.
    Declare,
    /// A job creation: idempotent and held by `Prefer: wait`.
    Job,
}

pub struct Api {
    base: String,
    token: Option<String>,
    api: Agent,
    storage: Agent,
    held_timeout: Duration,
    sleep: Sleep,
    warn: Warn,
}

impl fmt::Debug for Api {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Api")
            .field("base", &self.base)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

fn agent(timeout: Duration) -> Agent {
    Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .max_redirects(0)
        .user_agent(USER_AGENT)
        .build()
        .into()
}

fn usage(message: &str) -> UsageError {
    UsageError {
        message: message.to_owned(),
        command: None,
    }
}

fn valid_base(base: &str) -> Option<String> {
    let trimmed = base.trim_end_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    let rest = if lower.starts_with("https://") {
        &trimmed[8..]
    } else if lower.starts_with("http://") {
        &trimmed[7..]
    } else {
        return None;
    };
    let authority = rest.split('/').next().unwrap_or("");
    let bad = |c: char| c.is_whitespace() || c.is_control() || matches!(c, '?' | '#');
    if authority.is_empty() || authority.contains('@') || trimmed.chars().any(bad) {
        return None;
    }
    Some(trimmed.to_owned())
}

fn escape(segment: &str) -> String {
    segment
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn retry_after<B>(response: &Response<B>) -> Option<Duration> {
    let value = response.headers().get("retry-after")?.to_str().ok()?;
    let seconds = value.trim().parse::<f64>().ok()?;
    (seconds.is_finite() && seconds >= 0.0)
        .then(|| Duration::from_secs_f64(seconds.min(MAX_RETRY_AFTER.as_secs_f64())))
}

fn decode<T: DeserializeOwned>(answer: &Answer, what: &str) -> Result<T, ApiError> {
    serde_json::from_slice(&answer.body).map_err(|_| ApiError::malformed(what))
}

impl Api {
    /// `base_url` must be an absolute http(s) URL without credentials, query or fragment.
    pub fn new(base_url: &str, token: Option<String>) -> Result<Api, UsageError> {
        let base = valid_base(base_url).ok_or_else(|| {
            usage("the server URL must be an absolute http:// or https:// URL without credentials, query or fragment")
        })?;
        let token = token.filter(|t| !t.is_empty());
        if let Some(t) = &token {
            if !t.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
                return Err(usage(
                    "KOLDERA_TOKEN contains characters that cannot be sent in an HTTP header",
                ));
            }
        }
        Ok(Api {
            base,
            token,
            api: agent(API_TIMEOUT),
            storage: agent(STORAGE_TIMEOUT),
            held_timeout: API_TIMEOUT + Duration::from_secs(WAIT_SECONDS),
            sleep: Arc::new(std::thread::sleep),
            warn: Arc::new(|line| eprintln!("{line}")),
        })
    }

    /// Replaces the timeouts of API requests, held job creations and storage transfers (tests use short ones).
    pub fn with_timeouts(mut self, api: Duration, held: Duration, storage: Duration) -> Api {
        self.api = agent(api);
        self.storage = agent(storage);
        self.held_timeout = held;
        self
    }

    /// Replaces the pause between retries and polls (tests record it instead of sleeping).
    /// Replaces where waiting notices go (stderr by default; tests record them).
    pub fn with_warn(mut self, warn: impl Fn(&str) + Send + Sync + 'static) -> Api {
        self.warn = Arc::new(warn);
        self
    }

    pub fn with_sleep(mut self, sleep: impl Fn(Duration) + Send + Sync + 'static) -> Api {
        self.sleep = Arc::new(sleep);
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    fn secrets<'a>(&'a self, extra: &'a str) -> Vec<&'a str> {
        let mut s = vec![extra];
        if let Some((_, query)) = extra.split_once('?') {
            s.push(query);
        }
        s.extend(self.token.as_deref());
        s
    }

    fn exchange(
        &self,
        path: &str,
        body: Option<&[u8]>,
        key: Option<&str>,
        held: bool,
    ) -> Result<Answer, ureq::Error> {
        let url = format!("{}{path}", self.base);
        let auth = self.token.as_ref().map(|t| format!("Bearer {t}"));
        let mut response = match body {
            None => {
                let mut request = self.api.get(&url).header("Accept", "application/json");
                if let Some(auth) = &auth {
                    request = request.header("Authorization", auth);
                }
                request.call()?
            }
            Some(body) => {
                let mut request = self
                    .api
                    .post(&url)
                    .header("Accept", "application/json")
                    .header("Content-Type", "application/json");
                if let Some(auth) = &auth {
                    request = request.header("Authorization", auth);
                }
                if let Some(key) = key {
                    request = request.header("Idempotency-Key", key);
                }
                if held {
                    request
                        .header("Prefer", format!("wait={WAIT_SECONDS}"))
                        .config()
                        .timeout_global(Some(self.held_timeout))
                        .build()
                        .send(body)?
                } else {
                    request.send(body)?
                }
            }
        };
        let status = response.status().as_u16();
        let retry_after = retry_after(&response);
        let replayed = response
            .headers()
            .get("idempotent-replayed")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("true"));
        let (body, too_large) = match response
            .body_mut()
            .with_config()
            .limit(MAX_JSON_BYTES)
            .read_to_vec()
        {
            Ok(body) => (body, false),
            Err(ureq::Error::BodyExceedsLimit(_)) => (Vec::new(), true),
            Err(e) => return Err(e),
        };
        Ok(Answer {
            status,
            body,
            retry_after,
            replayed,
            too_large,
        })
    }

    fn call(&self, path: &str, body: Option<&[u8]>, kind: Call) -> Result<Answer, ApiError> {
        let key = (kind != Call::Read).then(|| uuid::Uuid::new_v4().to_string());
        let what = format!("the request to {path}");
        let (mut transport, mut busy) = (0, 0);
        loop {
            match self.exchange(path, body, key.as_deref(), kind == Call::Job) {
                Err(e) => {
                    let held = kind == Call::Job && matches!(e, ureq::Error::Timeout(_));
                    if held || transport >= RETRIES {
                        return Err(ApiError::transport(&what, &e));
                    }
                    transport += 1;
                    (self.sleep)(RETRY_STEP * transport);
                }
                Ok(a) if a.too_large && (200..300).contains(&a.status) => {
                    return Err(ApiError::malformed(path));
                }
                Ok(a) if (200..300).contains(&a.status) => return Ok(a),
                Ok(a) if kind == Call::Read && matches!(a.status, 429 | 503) && busy < RETRIES => {
                    busy += 1;
                    (self.sleep)(a.retry_after.unwrap_or(FIRST_POLL));
                }
                Ok(a) => {
                    return Err(ApiError::problem(
                        a.status,
                        &a.body,
                        "the server",
                        &self.secrets(""),
                    ));
                }
            }
        }
    }

    fn create<B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
        kind: Call,
    ) -> Result<Answer, ApiError> {
        let payload = serde_json::to_vec(body).map_err(|_| {
            ApiError::local(
                MALFORMED_RESPONSE,
                "the request body cannot be encoded".into(),
            )
        })?;
        self.call(path, Some(&payload), kind)
    }

    fn create_job<B: Serialize + ?Sized>(&self, path: &str, body: &B) -> Result<Created, ApiError> {
        let answer = self.create(path, body, Call::Job)?;
        let job: Job = decode(&answer, path)?;
        let finished = answer.status == 200 && is_terminal(&job.state);
        Ok(Created { job, finished })
    }

    fn read<T: DeserializeOwned>(&self, path: &str) -> Result<(T, Answer), ApiError> {
        let answer = self.call(path, None, Call::Read)?;
        Ok((decode(&answer, path)?, answer))
    }

    /// Declares an upload of `bytes` (size and SHA-256 are computed here).
    pub fn create_upload(
        &self,
        file_name: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<Ticket, ApiError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            file_name: &'a str,
            size_bytes: usize,
            sha256: String,
            content_type: &'a str,
        }
        let body = Body {
            file_name,
            size_bytes: bytes.len(),
            sha256: sha256_hex(bytes),
            content_type,
        };
        let answer = self.create("/v1/uploads", &body, Call::Declare)?;
        let mut ticket: Ticket = decode(&answer, "/v1/uploads")?;
        if ticket.upload_id.is_empty() || ticket.upload_url.is_empty() {
            return Err(ApiError::malformed("/v1/uploads"));
        }
        ticket.content_type = content_type.to_owned();
        ticket.replayed = answer.replayed;
        Ok(ticket)
    }

    /// Sends the bytes to the signed URL without any credential. `409 UPLOAD_ALREADY_COMPLETED` counts as stored when
    /// the ticket was replayed or this PUT is a repeat, since both mean an earlier attempt stored these exact bytes.
    pub fn put_upload(&self, ticket: &Ticket, bytes: &[u8]) -> Result<(), ApiError> {
        if let Some(max) = ticket.max_bytes {
            if bytes.len() as u64 > max {
                return Err(ApiError::local(
                    FILE_TOO_LARGE,
                    format!(
                        "the file ({} bytes) exceeds the upload limit of {max} bytes",
                        bytes.len()
                    ),
                ));
            }
        }
        let mut attempt = 0;
        loop {
            let sent = self
                .storage
                .put(&ticket.upload_url)
                .header("Content-Type", &ticket.content_type)
                .send(bytes);
            let mut response = match sent {
                Ok(r) => r,
                Err(e) if attempt < RETRIES && !matches!(e, ureq::Error::Timeout(_)) => {
                    attempt += 1;
                    (self.sleep)(RETRY_STEP * attempt);
                    continue;
                }
                Err(e) => return Err(ApiError::transport("the upload to storage", &e)),
            };
            let status = response.status().as_u16();
            if (200..300).contains(&status) {
                return Ok(());
            }
            let body = read_limited(response.body_mut().as_reader(), 1 << 20);
            let err = ApiError::problem(
                status,
                &body,
                "the storage",
                &self.secrets(&ticket.upload_url),
            );
            let repeat = ticket.replayed || attempt > 0;
            if status == 409 && err.code == ALREADY_COMPLETED && repeat {
                return Ok(());
            }
            return Err(err);
        }
    }

    pub fn create_pdf_job(&self, body: &PdfJobRequest) -> Result<Created, ApiError> {
        self.create_job("/v1/pdf/jobs", body)
    }

    pub fn create_html_job(&self, body: &HtmlJobRequest) -> Result<Created, ApiError> {
        self.create_job("/v1/html-to-pdf/jobs", body)
    }

    pub fn create_web_job(&self, body: &WebJobRequest) -> Result<Created, ApiError> {
        self.create_job("/v1/web/jobs", body)
    }

    /// `document` is the structured document with every image's `file` already replaced by its `uploadId`.
    pub fn create_document_job(&self, document: &serde_json::Value) -> Result<Created, ApiError> {
        self.create_job("/v1/documents/jobs", document)
    }

    /// Polls the job until it is no longer queued or running, honouring `Retry-After` (else 1 s doubling to 5 s).
    /// Busy answers (429, 502-504) and transport failures keep it polling, with one notice per outage; any other
    /// non-2xx status ends it.
    pub fn wait_job(&self, id: &str) -> Result<Job, ApiError> {
        let path = format!("/v1/jobs/{}", escape(id));
        let mut delay = FIRST_POLL;
        let mut outage = false;
        loop {
            let answer = self.exchange(&path, None, None, false);
            let down = match &answer {
                Err(_) => true,
                Ok(a) => matches!(a.status, 502..=504),
            };
            if down && !outage {
                (self.warn)(&format!(
                    "koldera: the server is not answering; still waiting for job {id} (Ctrl+C to stop)"
                ));
            }
            if down {
                outage = true;
            }
            let pause = match answer {
                Err(_) => delay,
                Ok(a) if a.too_large && (200..300).contains(&a.status) => {
                    return Err(ApiError::malformed(&path));
                }
                Ok(a) if (200..300).contains(&a.status) => {
                    outage = false;
                    let job: Job = decode(&a, &path)?;
                    if is_terminal(&job.state) {
                        return Ok(job);
                    }
                    a.retry_after.unwrap_or(delay)
                }
                Ok(a) if matches!(a.status, 429 | 502..=504) => a.retry_after.unwrap_or(delay),
                Ok(a) => {
                    return Err(ApiError::problem(
                        a.status,
                        &a.body,
                        "the server",
                        &self.secrets(""),
                    ));
                }
            };
            (self.sleep)(pause.max(FIRST_POLL));
            delay = (delay * 2).min(MAX_POLL);
        }
    }

    pub fn findings(&self, id: &str) -> Result<Vec<Finding>, ApiError> {
        self.items(id, "findings")
    }

    pub fn actions(&self, id: &str) -> Result<Vec<Action>, ApiError> {
        self.items(id, "actions")
    }

    pub fn artifacts(&self, id: &str) -> Result<Vec<Artifact>, ApiError> {
        self.items(id, "artifacts")
    }

    fn items<T: DeserializeOwned>(&self, id: &str, what: &str) -> Result<Vec<T>, ApiError> {
        let (items, _): (Items<T>, _) = self.read(&format!("/v1/jobs/{}/{what}", escape(id)))?;
        Ok(items.items)
    }

    /// Fetches an artifact through its signed URL without any credential; reads at most `size + 1` bytes and returns
    /// them only when size and SHA-256 match the record.
    pub fn download(&self, artifact: &Artifact) -> Result<Vec<u8>, ApiError> {
        let what = format!("the download of the {} artifact", artifact.kind);
        let mut response = self
            .storage
            .get(&artifact.download_url)
            .call()
            .map_err(|e| ApiError::transport(&what, &e))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let body = read_limited(response.body_mut().as_reader(), 1 << 20);
            return Err(ApiError::problem(
                status,
                &body,
                "the storage",
                &self.secrets(&artifact.download_url),
            ));
        }
        let mut bytes = Vec::new();
        response
            .into_body()
            .into_reader()
            .take(artifact.size_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|e| ApiError::io(&what, &e))?;
        let matches = bytes.len() as u64 == artifact.size_bytes
            && sha256_hex(&bytes).eq_ignore_ascii_case(&artifact.sha256);
        if !matches {
            return Err(ApiError::local(
                ARTIFACT_MISMATCH,
                format!(
                    "the {} artifact does not match its declared size and SHA-256",
                    artifact.kind
                ),
            ));
        }
        Ok(bytes)
    }
}

fn read_limited(reader: impl Read, limit: u64) -> Vec<u8> {
    let mut body = Vec::new();
    let _ = reader.take(limit).read_to_end(&mut body);
    body
}
