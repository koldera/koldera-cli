#![allow(dead_code)]

use koldera_cli::model::{Action, Artifact, Finding, Items, Job};
use serde_json::Value;

/// The `response` object of a golden fixture in `contracts/golden`.
pub fn response(name: &str) -> Value {
    let path = format!(
        "{}/contracts/golden/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let fixture: Value = serde_json::from_str(&text).unwrap();
    fixture["response"].clone()
}

pub fn body(name: &str) -> Value {
    response(name)["body"].clone()
}

pub fn raw_body(name: &str) -> Vec<u8> {
    // The fixtures store bytes as a latin1 string.
    let text = response(name)["rawBody"].as_str().unwrap().to_owned();
    text.chars().map(|c| c as u32 as u8).collect()
}

pub fn job(name: &str) -> Job {
    serde_json::from_value(body(name)).unwrap()
}

pub fn findings() -> Vec<Finding> {
    serde_json::from_value::<Items<Finding>>(body("findings"))
        .unwrap()
        .items
}

pub fn actions() -> Vec<Action> {
    serde_json::from_value::<Items<Action>>(body("actions"))
        .unwrap()
        .items
}

pub fn artifacts() -> Vec<Artifact> {
    serde_json::from_value::<Items<Artifact>>(body("artifacts"))
        .unwrap()
        .items
}

/// A finished scan: the accepted golden web job, completed with a WCAG verdict.
pub fn scan_job(status: &str, failed: &[(&str, i64)]) -> Job {
    let mut v = body("web-job-accepted");
    let rules: Vec<Value> = failed
        .iter()
        .map(|(r, c)| serde_json::json!({"ruleId": r, "count": c}))
        .collect();
    let validation = serde_json::json!({"profile": "wcag22aa_automated", "status": status, "failedRules": rules});
    v["state"] = "completed".into();
    v["startedAt"] = "2026-10-05T10:00:01Z".into();
    v["finishedAt"] = "2026-10-05T10:00:09Z".into();
    v["validation"] = validation.clone();
    v["baselineValidation"] = validation;
    v["remediationStatus"] = "not_requested".into();
    v["engineVersion"] = "0.1.0".into();
    serde_json::from_value(v).unwrap()
}
