//! Exit codes, the same as core's command line.

use crate::model::Job;

pub const OK: i32 = 0;
pub const FAILED_RULES: i32 = 2;
pub const JOB_FAILED: i32 = 3;
pub const USAGE: i32 = 64;
pub const INTERRUPTED: i32 = 130;

/// The job error code of a structured document whose generated PDF failed the machine check.
pub const DOCUMENT_VALIDATION_FAILED: &str = "DOCUMENT_VALIDATION_FAILED";

/// 0 only for the verdict "passed"; 2 for "failed" or any failed rule (and a document that failed validation);
/// 3 for a job that did not complete and for a completed one without a verdict (not_run, error, missing).
pub fn code_for(job: &Job) -> i32 {
    if job.state != "completed" {
        let document_failed = job
            .error
            .as_ref()
            .is_some_and(|e| e.code == DOCUMENT_VALIDATION_FAILED);
        return if document_failed {
            FAILED_RULES
        } else {
            JOB_FAILED
        };
    }
    match &job.validation {
        None => JOB_FAILED,
        Some(v) if v.status == "failed" || !v.failed_rules.is_empty() => FAILED_RULES,
        Some(v) if v.status == "passed" => OK,
        Some(_) => JOB_FAILED,
    }
}
