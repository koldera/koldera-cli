//! What the command line prints for a finished job: the same text and `--json` document as core's command line.

use std::fmt::Write as _;

use serde::Serialize;

use crate::exit::DECLARATION_WITHHELD;
use crate::model::{Action, Finding, Job, Validation};
use crate::output::Written;

const LISTED_FINDINGS: usize = 10;
const WEB_PROFILE: &str = "wcag22aa_automated";

/// The text summary; every line ends with `\n`.
pub fn text(job: &Job, findings: &[Finding], actions: &[Action], written: &[Written]) -> String {
    let mut s = String::new();
    let target = job
        .target_url
        .as_ref()
        .map(|u| format!(" ({u})"))
        .unwrap_or_default();
    line(
        &mut s,
        format_args!("Job {}: {}{target}", job.id, job.state),
    );
    if let Some(e) = &job.error {
        line(&mut s, format_args!("Error: {}: {}", e.code, e.message));
    }

    let label = label(job);
    match &job.validation {
        Some(v) => {
            line(
                &mut s,
                format_args!("{label} machine check: {}", describe(v)),
            );
            if v.status == DECLARATION_WITHHELD {
                line(
                    &mut s,
                    format_args!(
                        "  The PDF/UA identifier (pdfuaid) was not written: declaring PDF/UA requires a human review. This is not a pass."
                    ),
                );
            }
            // A fix has a before and an after; show the starting point when it differs.
            if let (Some(b), Some(_)) = (&job.baseline_validation, &job.final_validation) {
                if describe(b) != describe(v) {
                    line(&mut s, format_args!("  before the fix: {}", describe(b)));
                }
            }
        }
        None => line(&mut s, format_args!("{label} machine check: not run")),
    }
    if let Some(status) = job
        .remediation_status
        .as_deref()
        .filter(|s| *s != "not_requested")
    {
        let applied = actions.iter().filter(|a| a.outcome == "applied").count();
        line(
            &mut s,
            format_args!(
                "Remediation: {status} ({applied} of {} steps applied)",
                actions.len()
            ),
        );
    }

    let review: Vec<&Finding> = findings.iter().filter(|f| f.needs_human_review).collect();
    line(
        &mut s,
        format_args!(
            "Findings: {} ({} need human review)",
            findings.len(),
            review.len()
        ),
    );
    for f in review.iter().take(LISTED_FINDINGS) {
        let page = f
            .page_number
            .map(|p| format!(" (page {p})"))
            .unwrap_or_default();
        line(
            &mut s,
            format_args!("  - {}: {}{page}", f.rule_id, f.message),
        );
    }
    if review.len() > LISTED_FINDINGS {
        line(
            &mut s,
            format_args!(
                "  ... and {} more (use --json for all)",
                review.len() - LISTED_FINDINGS
            ),
        );
    }

    for w in written {
        line(
            &mut s,
            format_args!("Wrote {}: {}", w.artifact.kind, w.path.display()),
        );
    }
    line(
        &mut s,
        format_args!("Automatic checks only; a person still has to review the result."),
    );
    s
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonArtifact<'a> {
    id: &'a str,
    kind: &'a str,
    sha256: &'a str,
    size_bytes: u64,
    path: String,
}

#[derive(Serialize)]
struct JsonSummary<'a> {
    job: &'a Job,
    findings: &'a [Finding],
    actions: &'a [Action],
    artifacts: Vec<JsonArtifact<'a>>,
}

/// The `--json` document `{job, findings, actions, artifacts}` on one line (no trailing newline); each artifact
/// carries its local `path` instead of the signed download URL.
pub fn json(job: &Job, findings: &[Finding], actions: &[Action], written: &[Written]) -> String {
    let artifacts = written
        .iter()
        .map(|w| JsonArtifact {
            id: &w.artifact.id,
            kind: &w.artifact.kind,
            sha256: &w.artifact.sha256,
            size_bytes: w.artifact.size_bytes,
            path: w.path.to_string_lossy().into_owned(),
        })
        .collect();
    let summary = JsonSummary {
        job,
        findings,
        actions,
        artifacts,
    };
    serde_json::to_string(&summary).expect("the summary serializes")
}

fn line(s: &mut String, args: std::fmt::Arguments<'_>) {
    s.write_fmt(args).expect("writing to a String");
    s.push('\n');
}

fn label(job: &Job) -> String {
    let fallback = || {
        job.profile
            .clone()
            .or_else(|| (job.kind == "web").then(|| WEB_PROFILE.to_owned()))
    };
    let profile = job
        .validation
        .as_ref()
        .map(|v| v.profile.clone())
        .or_else(fallback);
    match profile.as_deref() {
        // "pdfua1" is how the contract fixtures spell the PDF/UA-1 profile; the engine reports "ua1".
        None | Some("ua1") | Some("pdfua1") => "PDF/UA-1".to_owned(),
        Some(WEB_PROFILE) => "WCAG 2.2 AA automated".to_owned(),
        Some(other) => other.to_owned(),
    }
}

fn describe(v: &Validation) -> String {
    let n = v.failed_rules.len();
    if n == 0 {
        return v.status.clone();
    }
    let mut rules: Vec<String> = v
        .failed_rules
        .iter()
        .take(5)
        .map(|f| format!("{} x{}", f.rule_id, f.count))
        .collect();
    if n > 5 {
        rules.push("...".to_owned());
    }
    let noun = if n == 1 { "rule" } else { "rules" };
    format!("{} ({n} {noun} failed: {})", v.status, rules.join(", "))
}
