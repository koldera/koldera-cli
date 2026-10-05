//! The API shapes the command line reads (`contracts/openapi.yaml`). Enum-like values stay strings so a newer
//! server's values pass through; unknown fields are ignored.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub mode: Option<String>,
    pub state: String,
    #[serde(default)]
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub validation: Option<Validation>,
    pub baseline_validation: Option<Validation>,
    pub final_validation: Option<Validation>,
    pub remediation_status: Option<String>,
    #[serde(default)]
    pub manual_review_required: bool,
    pub engine_version: Option<String>,
    pub error: Option<JobError>,
    pub target_url: Option<String>,
    pub content_selector: Option<String>,
    /// The profile the job was created with (not on the wire); labels the summary when there is no verdict.
    #[serde(default, skip_serializing)]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Validation {
    pub profile: String,
    pub status: String,
    #[serde(default)]
    pub failed_rules: Vec<FailedRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedRule {
    pub rule_id: String,
    pub count: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobError {
    pub code: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub source: String,
    pub rule_id: String,
    pub message: String,
    pub page_number: Option<i64>,
    pub selector: Option<String>,
    #[serde(default)]
    pub needs_human_review: bool,
    pub resolution: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub rule_id: String,
    pub outcome: String,
    #[serde(default)]
    pub reason: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// An artifact record. `download_url` is a signed URL: it is a credential and never printed (see the Debug impl).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub id: String,
    pub kind: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub download_url: String,
    #[serde(default)]
    pub expires_at: String,
}

impl fmt::Debug for Artifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Artifact")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("sha256", &self.sha256)
            .field("size_bytes", &self.size_bytes)
            .field("download_url", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// The `{items: [...]}` envelope of the list endpoints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Items<T> {
    pub items: Vec<T>,
}
