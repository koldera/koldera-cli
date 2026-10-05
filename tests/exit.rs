mod common;

use koldera_cli::exit::{self, code_for};
use koldera_cli::model::{FailedRule, Job, JobError, Validation};

fn with_validation(status: &str, failed: usize) -> Job {
    let mut job = common::job("job-completed");
    let v = Validation {
        profile: "ua1".into(),
        status: status.into(),
        failed_rules: (0..failed)
            .map(|i| FailedRule {
                rule_id: format!("7.1-{i}"),
                count: 1,
            })
            .collect(),
    };
    job.validation = Some(v);
    job
}

#[test]
fn constants() {
    assert_eq!(
        (
            exit::OK,
            exit::FAILED_RULES,
            exit::JOB_FAILED,
            exit::USAGE,
            exit::INTERRUPTED
        ),
        (0, 2, 3, 64, 130)
    );
}

#[test]
fn verdicts_of_completed_jobs() {
    assert_eq!(code_for(&with_validation("passed", 0)), 0);
    assert_eq!(code_for(&with_validation("failed", 0)), 2);
    assert_eq!(code_for(&with_validation("failed", 1)), 2);
    // Failed rules count even when the status claims passed.
    assert_eq!(code_for(&with_validation("passed", 1)), 2);
    assert_eq!(code_for(&with_validation("not_run", 0)), 3);
    assert_eq!(code_for(&with_validation("error", 0)), 3);
    assert_eq!(code_for(&with_validation("something_new", 0)), 3);
    let mut missing = common::job("job-completed");
    missing.validation = None;
    assert_eq!(code_for(&missing), 3);
}

#[test]
fn golden_jobs() {
    assert_eq!(code_for(&common::job("job-completed")), 2);
    assert_eq!(code_for(&common::job("job-completed-fix")), 0);
    assert_eq!(code_for(&common::job("job-failed")), 3);
    assert_eq!(
        code_for(&common::job("document-job-prefer-wait-finished")),
        0
    );
}

#[test]
fn unfinished_and_failed_jobs() {
    let mut job = common::job("job-failed");
    job.error = Some(JobError {
        code: "DOCUMENT_VALIDATION_FAILED".into(),
        message: "The generated PDF failed the machine check.".into(),
    });
    assert_eq!(code_for(&job), 2);
    job.error = None;
    assert_eq!(code_for(&job), 3);
    // Only a completed job has a verdict, whatever its validation says.
    let mut running = with_validation("passed", 0);
    running.state = "running".into();
    assert_eq!(code_for(&running), 3);
}
