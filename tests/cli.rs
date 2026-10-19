use std::process::{Command, Output};

fn koldera(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_koldera"))
        .args(args)
        .env_remove("KOLDERA_URL")
        .env_remove("KOLDERA_TOKEN")
        .env_remove("KOLDERA_IMAGE")
        .output()
        .unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8(o.stderr.clone()).unwrap()
}

#[test]
fn version() {
    let o = koldera(&["--version"]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(
        stdout(&o),
        format!("koldera-cli {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn help_goes_to_stdout_with_exit_0() {
    for args in [&[][..], &["--help"], &["-h"], &["help"]] {
        let o = koldera(args);
        assert_eq!(o.status.code(), Some(0));
        assert_eq!(stdout(&o), koldera_cli::args::help_of("") + "\n");
        assert!(stderr(&o).is_empty());
    }
    let o = koldera(&["pdf", "--help"]);
    assert_eq!(stdout(&o), koldera_cli::args::help_of("pdf") + "\n");
}

#[test]
fn usage_errors_go_to_stderr_with_exit_64() {
    let o = koldera(&["pdf", "a.pdf", "--x"]);
    assert_eq!(o.status.code(), Some(64));
    assert!(stdout(&o).is_empty());
    assert_eq!(
        stderr(&o),
        format!(
            "koldera: unknown option for pdf: --x\n{}\n",
            koldera_cli::args::usage_of("pdf")
        )
    );
    let o = koldera(&["nope"]);
    assert_eq!(o.status.code(), Some(64));
    assert_eq!(
        stderr(&o),
        format!(
            "koldera: unknown command: nope\n{}\n",
            koldera_cli::args::GENERAL_USAGE
        )
    );
}

mod support;

use serde_json::json;
use support::server::{Reply, TestServer, fixture};

fn koldera_with(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_koldera"));
    c.args(args)
        .env_remove("KOLDERA_URL")
        .env_remove("KOLDERA_TOKEN")
        .env_remove("KOLDERA_IMAGE");
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().unwrap()
}

#[test]
fn a_server_is_required_and_local_mode_is_rejected() {
    let o = koldera(&["scan", "https://example.com/"]);
    assert_eq!(o.status.code(), Some(64));
    assert_eq!(
        stderr(&o),
        "koldera: set --server or KOLDERA_URL to connect to koldera-core\n"
    );
    let o = koldera_with(
        &["--local", "scan", "https://example.com/"],
        &[("KOLDERA_URL", "http://127.0.0.1:9")],
    );
    assert_eq!(o.status.code(), Some(64));
    assert_eq!(
        stderr(&o),
        "koldera: local Docker mode is not implemented; use --server or KOLDERA_URL\n"
    );
    let o = koldera_with(
        &["--image", "koldera-core:0.1.0", "scan", "https://example.com/"],
        &[("KOLDERA_URL", "http://127.0.0.1:9")],
    );
    assert_eq!(o.status.code(), Some(64));
    assert_eq!(
        stderr(&o),
        "koldera: local Docker mode is not implemented; use --server or KOLDERA_URL\n"
    );
}

#[test]
fn koldera_url_selects_remote_mode_and_the_token_comes_from_the_environment() {
    let server = TestServer::start();
    let id = "4d2d5a6e-2b1c-4e4a-9d6b-1f3c5e7a9b01";
    let mut done = fixture("job-completed-fix")["response"]["body"].clone();
    done["kind"] = json!("web");
    done["targetUrl"] = json!("https://example.com/");
    server
        .on("POST /v1/web/jobs", Reply::json(200, &done))
        .on(
            &format!("GET /v1/jobs/{id}/findings"),
            Reply::golden("findings"),
        )
        .on(
            &format!("GET /v1/jobs/{id}/actions"),
            Reply::golden("actions"),
        )
        .on(
            &format!("GET /v1/jobs/{id}/artifacts"),
            Reply::json(200, &json!({"items": []})),
        );
    let o = koldera_with(
        &["scan", "https://example.com/", "--json"],
        &[
            ("KOLDERA_URL", &server.base),
            ("KOLDERA_TOKEN", "env-token-value"),
        ],
    );
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert_eq!(stderr(&o), "");
    let post = &server.requests_to("POST /v1/web/jobs")[0];
    assert_eq!(post.header("authorization"), Some("Bearer env-token-value"));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["job"]["id"], id);
}

#[test]
fn a_plain_http_remote_server_gets_one_warning_line() {
    let o = koldera_with(
        &[
            "--server",
            "http://core.invalid",
            "scan",
            "https://example.com/",
        ],
        &[("KOLDERA_TOKEN", "env-token-value")],
    );
    assert_eq!(o.status.code(), Some(3));
    let err = stderr(&o);
    let mut lines = err.lines();
    assert_eq!(
        lines.next(),
        Some("koldera: warning: the server URL is not HTTPS; the token is sent in clear text")
    );
    assert!(lines.next().unwrap().starts_with("koldera: "), "{err}");
    assert!(!err.contains("env-token-value"), "{err}");
}

#[test]
fn an_invalid_server_url_is_a_usage_error() {
    let o = koldera_with(
        &["scan", "https://example.com/"],
        &[("KOLDERA_URL", "core.example.com")],
    );
    assert_eq!(o.status.code(), Some(64));
    assert!(
        stderr(&o)
            .starts_with("koldera: the server URL must be an absolute http:// or https:// URL"),
        "{}",
        stderr(&o)
    );
}
