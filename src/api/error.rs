use std::fmt;

use serde::Deserialize;

/// One entry of a problem's `errors[]` (a `422` for a structured document).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FieldError {
    pub path: String,
    pub code: String,
    pub message: String,
}

/// A failed exchange. `status` is `None` when no HTTP answer arrived. No field ever holds the token or a signed URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: Option<u16>,
    pub code: String,
    pub message: String,
    pub errors: Vec<FieldError>,
}

pub const TRANSPORT_ERROR: &str = "TRANSPORT_ERROR";
pub const MALFORMED_RESPONSE: &str = "MALFORMED_RESPONSE";
pub const ARTIFACT_MISMATCH: &str = "ARTIFACT_MISMATCH";
pub const FILE_TOO_LARGE: &str = "FILE_TOO_LARGE";

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            Some(status) => write!(f, "HTTP {status} {}: {}", self.code, self.message),
            None => write!(f, "{} ({})", self.message, self.code),
        }
    }
}

impl std::error::Error for ApiError {}

#[derive(Deserialize)]
struct Problem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    detail: String,
    #[serde(default)]
    errors: Vec<serde_json::Value>,
}

impl ApiError {
    pub(crate) fn local(code: &str, message: String) -> ApiError {
        ApiError {
            status: None,
            code: code.to_owned(),
            message,
            errors: Vec::new(),
        }
    }

    pub(crate) fn transport(what: &str, error: &ureq::Error) -> ApiError {
        ApiError::local(
            TRANSPORT_ERROR,
            format!("{what} failed: {}", describe(error)),
        )
    }

    pub(crate) fn io(what: &str, error: &std::io::Error) -> ApiError {
        ApiError::local(TRANSPORT_ERROR, format!("{what} failed: {}", error.kind()))
    }

    pub(crate) fn malformed(what: &str) -> ApiError {
        ApiError::local(
            MALFORMED_RESPONSE,
            format!("the answer to {what} could not be read"),
        )
    }

    /// Maps an RFC 9457 problem body; `secrets` are removed from the server's texts.
    pub(crate) fn problem(status: u16, body: &[u8], who: &str, secrets: &[&str]) -> ApiError {
        let fallback = if status >= 500 {
            "INTERNAL_ERROR"
        } else {
            "UNKNOWN"
        };
        let parsed = serde_json::from_slice::<Problem>(body)
            .ok()
            .filter(|p| !p.code.is_empty());
        let Some(p) = parsed else {
            return ApiError {
                status: Some(status),
                code: fallback.to_owned(),
                message: format!("{who} answered HTTP {status}"),
                errors: Vec::new(),
            };
        };
        let text = if p.detail.trim().is_empty() {
            p.title
        } else {
            p.detail
        };
        let message = if text.trim().is_empty() {
            format!("{who} answered HTTP {status}")
        } else {
            redact(&text, secrets)
        };
        let errors = p
            .errors
            .into_iter()
            .filter_map(|e| serde_json::from_value::<FieldError>(e).ok())
            .map(|e| FieldError {
                message: redact(&e.message, secrets),
                ..e
            })
            .collect();
        ApiError {
            status: Some(status),
            code: p.code,
            message,
            errors,
        }
    }
}

fn redact(text: &str, secrets: &[&str]) -> String {
    secrets
        .iter()
        .filter(|s| s.len() >= 4)
        .fold(text.to_owned(), |t, s| t.replace(s, "***"))
}

/// A description of a transport failure built only from its kind, so no URL or header value can leak into it.
fn describe(error: &ureq::Error) -> String {
    use ureq::Error as E;
    match error {
        E::Timeout(_) => "the request timed out".to_owned(),
        E::HostNotFound => "the host was not found".to_owned(),
        E::ConnectionFailed => "the connection failed".to_owned(),
        E::Io(e) => e.kind().to_string(),
        E::Tls(_) | E::Pem(_) | E::Rustls(_) | E::TlsRequired => {
            "the TLS connection failed".to_owned()
        }
        E::BodyExceedsLimit(n) => format!("the answer is larger than {n} bytes"),
        E::BadUri(_) | E::Http(_) => "the request could not be built".to_owned(),
        E::LargeResponseHeader(..) => "the answer's headers are too large".to_owned(),
        _ => "the answer could not be read".to_owned(),
    }
}
