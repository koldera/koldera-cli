mod common;

use std::path::PathBuf;

use koldera_cli::model::{Artifact, Finding, Job};
use koldera_cli::output::Written;
use koldera_cli::summary;
use serde_json::{Value, json};

const CLOSING: &str = "Automatic checks only; a person still has to review the result.\n";

fn written(artifacts: &[Artifact], paths: &[&str]) -> Vec<Written> {
    artifacts
        .iter()
        .zip(paths)
        .map(|(a, p)| Written {
            artifact: a.clone(),
            path: PathBuf::from(p),
        })
        .collect()
}

fn finding(rule: &str, review: bool, page: Option<i64>) -> Finding {
    Finding {
        source: "web".into(),
        rule_id: rule.into(),
        message: format!("{rule} message"),
        page_number: page,
        selector: Some("main > img".into()),
        needs_human_review: review,
        resolution: None,
    }
}

#[test]
fn text_for_a_pdf_fix_job() {
    let job = common::job("job-completed-fix");
    let w = written(
        &common::artifacts(),
        &["/out/sample.original.pdf", "/out/sample.remediated.pdf"],
    );
    let text = summary::text(&job, &common::findings(), &common::actions(), &w);
    assert_eq!(
        text,
        format!(
            "Job 4d2d5a6e-2b1c-4e4a-9d6b-1f3c5e7a9b01: completed\n\
             PDF/UA-1 machine check: passed\n\
             \x20 before the fix: failed (1 rule failed: 7.1-3 x2)\n\
             Remediation: applied (1 of 2 steps applied)\n\
             Findings: 2 (1 need human review)\n\
             \x20 - TITLE_FROM_FILE_NAME: The title was taken from the file name; check it.\n\
             Wrote original: {}\n\
             Wrote remediated: {}\n\
             {CLOSING}",
            PathBuf::from("/out/sample.original.pdf").display(),
            PathBuf::from("/out/sample.remediated.pdf").display(),
        )
    );
}

#[test]
fn text_for_an_analyze_job_has_no_before_line_and_no_remediation_line() {
    let job = common::job("job-completed");
    let text = summary::text(&job, &[], &[], &[]);
    assert_eq!(
        text,
        format!(
            "Job 4d2d5a6e-2b1c-4e4a-9d6b-1f3c5e7a9b01: completed\n\
             PDF/UA-1 machine check: failed (1 rule failed: 7.1-3 x2)\n\
             Findings: 0 (0 need human review)\n\
             {CLOSING}"
        )
    );
}

#[test]
fn before_line_only_when_the_verdict_changed() {
    let mut job = common::job("job-completed-fix");
    job.baseline_validation = job.final_validation.clone();
    assert!(!summary::text(&job, &[], &[], &[]).contains("before the fix"));
}

#[test]
fn text_for_a_failed_job() {
    let job = common::job("job-failed");
    assert_eq!(
        summary::text(&job, &[], &[], &[]),
        format!(
            "Job 4d2d5a6e-2b1c-4e4a-9d6b-1f3c5e7a9b01: failed\n\
             Error: WORKER_TIMEOUT: The worker did not finish in time.\n\
             PDF/UA-1 machine check: not run\n\
             Findings: 0 (0 need human review)\n\
             {CLOSING}"
        )
    );
}

#[test]
fn text_for_a_scan_job_lists_the_first_ten_findings_needing_review() {
    let job = common::scan_job("failed", &[("color-contrast", 3), ("image-alt", 1)]);
    let mut findings: Vec<Finding> = (0..11)
        .map(|i| finding(&format!("rule-{i}"), true, None))
        .collect();
    findings.insert(3, finding("auto-only", false, None));
    findings[0].page_number = Some(2);
    let report = Artifact {
        id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa3".into(),
        kind: "web_report".into(),
        sha256: "00".repeat(32),
        size_bytes: 2,
        download_url: "https://core.example.com/v1/artifacts/x/content?sig=secret".into(),
        expires_at: "2026-10-05T10:10:00Z".into(),
    };
    let w = written(&[report], &["/out/scan.web_report.json"]);
    let mut expected = String::from(
        "Job 4d2d5a6e-2b1c-4e4a-9d6b-1f3c5e7a9b02: completed (https://example.com/page)\n\
         WCAG 2.2 AA automated machine check: failed (2 rules failed: color-contrast x3, image-alt x1)\n\
         Findings: 12 (11 need human review)\n\
         \x20 - rule-0: rule-0 message (page 2)\n",
    );
    for i in 1..10 {
        expected.push_str(&format!("  - rule-{i}: rule-{i} message\n"));
    }
    expected.push_str("  ... and 1 more (use --json for all)\n");
    expected.push_str(&format!(
        "Wrote web_report: {}\n",
        PathBuf::from("/out/scan.web_report.json").display()
    ));
    expected.push_str(CLOSING);
    assert_eq!(summary::text(&job, &findings, &[], &w), expected);
}

#[test]
fn more_than_five_failed_rules_are_cut_short() {
    let rules: Vec<(String, i64)> = (1..=6).map(|i| (format!("r{i}"), i)).collect();
    let refs: Vec<(&str, i64)> = rules.iter().map(|(r, c)| (r.as_str(), *c)).collect();
    let job = common::scan_job("failed", &refs);
    assert!(summary::text(&job, &[], &[], &[]).contains(
        "WCAG 2.2 AA automated machine check: failed (6 rules failed: r1 x1, r2 x2, r3 x3, r4 x4, r5 x5, ...)\n"
    ));
}

#[test]
fn labels() {
    let line = |job: &Job| {
        summary::text(job, &[], &[], &[])
            .lines()
            .find(|l| l.contains("machine check"))
            .unwrap()
            .to_owned()
    };
    let mut job = common::job("job-completed");
    for (profile, label) in [
        ("ua1", "PDF/UA-1"),
        ("pdfua1", "PDF/UA-1"),
        ("wcag22aa_automated", "WCAG 2.2 AA automated"),
        ("future_profile", "future_profile"),
    ] {
        job.validation.as_mut().unwrap().profile = profile.into();
        assert!(
            line(&job).starts_with(&format!("{label} machine check:")),
            "{profile}"
        );
    }
    // Without a verdict the label comes from the job: a web job is a WCAG scan, anything else PDF/UA-1.
    let mut web = common::scan_job("passed", &[]);
    web.validation = None;
    assert_eq!(line(&web), "WCAG 2.2 AA automated machine check: not run");
    web.profile = Some("ua1".into());
    assert_eq!(line(&web), "PDF/UA-1 machine check: not run");
}

#[test]
fn json_for_a_pdf_fix_job() {
    let job = common::job("job-completed-fix");
    let w = written(
        &common::artifacts(),
        &["/out/sample.original.pdf", "/out/sample.remediated.pdf"],
    );
    let text = summary::json(&job, &common::findings(), &common::actions(), &w);
    assert!(!text.contains('\n'));
    assert!(text.starts_with(r#"{"job":{"id":"4d2d5a6e-2b1c-4e4a-9d6b-1f3c5e7a9b01","kind":"pdf","mode":"fix","state":"completed","createdAt""#), "{text}");
    assert!(!text.contains("downloadUrl") && !text.contains("sig="));
    let parsed: Value = serde_json::from_str(&text).unwrap();
    let path = |p: &str| PathBuf::from(p).to_string_lossy().into_owned();
    assert_eq!(
        parsed,
        json!({
            "job": common::body("job-completed-fix"),
            "findings": common::body("findings")["items"],
            "actions": common::body("actions")["items"],
            "artifacts": [
                {
                    "id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1",
                    "kind": "original",
                    "sha256": "5a838678058f6de375e8635b5f2fea47a4e5f07cb1a882a44b10f39abc6f34ff",
                    "sizeBytes": 45,
                    "path": path("/out/sample.original.pdf"),
                },
                {
                    "id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa2",
                    "kind": "remediated",
                    "sha256": "fe9a7d44f4318fec2635607dcacc6755fafe3b997406abe640db9cec5f28ab08",
                    "sizeBytes": 45,
                    "path": path("/out/sample.remediated.pdf"),
                },
            ],
        })
    );
}

#[test]
fn json_for_a_scan_job_keeps_nulls_and_has_empty_lists() {
    let job = common::scan_job("passed", &[]);
    let parsed: Value = serde_json::from_str(&summary::json(&job, &[], &[], &[])).unwrap();
    assert_eq!(parsed["job"]["targetUrl"], "https://example.com/page");
    assert_eq!(parsed["job"]["finalValidation"], Value::Null);
    assert_eq!(parsed["job"]["error"], Value::Null);
    assert!(parsed["job"].get("profile").is_none());
    assert_eq!(parsed["findings"], json!([]));
    assert_eq!(parsed["actions"], json!([]));
    assert_eq!(parsed["artifacts"], json!([]));
}

#[test]
fn unknown_fields_are_ignored_and_unknown_values_kept() {
    let mut v = common::body("job-completed");
    v["somethingNew"] = json!({"x": 1});
    v["state"] = "archived".into();
    v["kind"] = "order_pdf".into();
    let job: Job = serde_json::from_value(v).unwrap();
    assert_eq!(
        (job.state.as_str(), job.kind.as_str()),
        ("archived", "order_pdf")
    );
}

#[test]
fn artifact_debug_hides_the_signed_url() {
    let a = &common::artifacts()[0];
    let debug = format!("{a:?}");
    assert!(
        !debug.contains("sig=") && !debug.contains("exp="),
        "{debug}"
    );
    assert!(debug.contains("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1"));
}
