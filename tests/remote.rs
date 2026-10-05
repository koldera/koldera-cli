mod support;

use std::fs;
use std::path::{Path, PathBuf};

use koldera_cli::api::Api;
use koldera_cli::args::{self, Command};
use koldera_cli::output::sha256_hex;
use koldera_cli::remote;
use serde_json::{Value, json};
use support::server::{Reply, TestServer, fixture};

const TOKEN: &str = "test-server-token";
const PDF: &[u8] = b"%PDF-1.4\n1 0 obj<<>>endobj\ntrailer<<>>\n%%EOF\n";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
const JOB_ID: &str = "4d2d5a6e-2b1c-4e4a-9d6b-1f3c5e7a9b01";
const UPLOAD_ID: &str = "11111111-1111-4111-8111-111111111111";

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(server: &TestServer, argv: &[&str]) -> Run {
    let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let command = args::parse(&argv).unwrap().command;
    let api = Api::new(&server.base, Some(TOKEN.to_owned()))
        .unwrap()
        .with_sleep(|_| {});
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = remote::run(&api, &command, &mut out, &mut err);
    let run = Run {
        code,
        stdout: String::from_utf8(out).unwrap(),
        stderr: String::from_utf8(err).unwrap(),
    };
    for text in [&run.stdout, &run.stderr] {
        assert!(!text.contains(TOKEN), "{text}");
        assert!(!text.contains("sig="), "{text}");
    }
    run
}

fn job(name: &str, id: &str) -> Value {
    let mut body = fixture(name)["response"]["body"].clone();
    body["id"] = json!(id);
    body
}

fn upload(server: &TestServer, id: &str) {
    let ticket = json!({
        "uploadId": id, "method": "PUT",
        "uploadUrl": format!("{{{{baseUrl}}}}/put/{id}?exp=1790000000&sig=c2lnbmF0dXJl"),
        "expiresAt": "2026-10-05T10:10:00Z", "maxBytes": 26214400
    });
    server
        .on("POST /v1/uploads", Reply::json(201, &ticket))
        .on(&format!("PUT /put/{id}"), Reply::status(204));
}

/// Findings, actions and the given artifacts (with their bytes behind signed URLs) for job `id`.
fn results(server: &TestServer, id: &str, artifacts: &[(&str, &[u8])]) {
    let items: Vec<Value> = artifacts
        .iter()
        .enumerate()
        .map(|(n, (kind, bytes))| {
            json!({
                "id": format!("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa{n}"), "kind": kind,
                "sha256": sha256_hex(bytes), "sizeBytes": bytes.len(),
                "downloadUrl": format!("{{{{baseUrl}}}}/dl/{n}?exp=1790000000&sig=c2lnbmF0dXJl"),
                "expiresAt": "2026-10-05T10:10:00Z"
            })
        })
        .collect();
    for (n, (_, bytes)) in artifacts.iter().enumerate() {
        server.on(
            &format!("GET /dl/{n}"),
            Reply::status(200).body(bytes.to_vec()),
        );
    }
    server
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
            Reply::json(200, &json!({"items": items})),
        );
}

fn input(dir: &Path, name: &str, bytes: &[u8]) -> String {
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

fn out_dir(dir: &Path) -> String {
    dir.join("out").to_string_lossy().into_owned()
}

fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|r| {
            r.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn pdf_analyze_uploads_creates_the_job_and_writes_the_artifacts() {
    let server = TestServer::start();
    let tmp = tempfile::tempdir().unwrap();
    let file = input(tmp.path(), "sample.pdf", PDF);
    upload(&server, UPLOAD_ID);
    server.on(
        "POST /v1/pdf/jobs",
        Reply::json(200, &job("job-completed", JOB_ID)),
    );
    results(&server, JOB_ID, &[("original", PDF)]);
    let out = out_dir(tmp.path());
    let r = run(&server, &["pdf", &file, "--out", &out]);

    assert_eq!(r.code, 2, "{}", r.stderr);
    assert_eq!(r.stderr, "");
    let declared = server.requests_to("POST /v1/uploads")[0].json();
    assert_eq!(
        declared,
        json!({"fileName": "sample.pdf", "sizeBytes": PDF.len(), "sha256": sha256_hex(PDF),
               "contentType": "application/pdf"})
    );
    assert_eq!(
        server.requests_to(&format!("PUT /put/{UPLOAD_ID}"))[0].body,
        PDF
    );
    assert_eq!(
        server.requests_to("POST /v1/pdf/jobs")[0].json(),
        json!({"uploadId": UPLOAD_ID, "mode": "analyze", "profile": "ua1"})
    );
    let written = Path::new(&out).join("sample.original.pdf");
    assert_eq!(fs::read(&written).unwrap(), PDF);
    assert!(
        r.stdout.starts_with(&format!("Job {JOB_ID}: completed\n")),
        "{}",
        r.stdout
    );
    let wrote = format!(
        "Wrote original: {}\n",
        std::path::absolute(&written).unwrap().display()
    );
    assert!(r.stdout.contains(&wrote), "{}", r.stdout);
}

#[test]
fn pdf_fix_sends_mode_language_and_title_and_polls_a_202() {
    let server = TestServer::start();
    let tmp = tempfile::tempdir().unwrap();
    let file = input(tmp.path(), "report.pdf", PDF);
    upload(&server, UPLOAD_ID);
    server
        .on(
            "POST /v1/pdf/jobs",
            Reply::json(202, &job("pdf-job-accepted-fix", JOB_ID)),
        )
        .on(
            &format!("GET /v1/jobs/{JOB_ID}"),
            Reply::json(200, &job("job-running", JOB_ID)),
        )
        .on(
            &format!("GET /v1/jobs/{JOB_ID}"),
            Reply::json(200, &job("job-completed-fix", JOB_ID)),
        );
    results(
        &server,
        JOB_ID,
        &[("original", PDF), ("remediated", b"%PDF-fixed")],
    );
    let out = out_dir(tmp.path());
    let r = run(
        &server,
        &[
            "pdf",
            &file,
            "--fix",
            "--language",
            "de",
            "--title",
            "Bericht",
            "--out",
            &out,
        ],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        server.requests_to("POST /v1/pdf/jobs")[0].json(),
        json!({"uploadId": UPLOAD_ID, "mode": "fix", "profile": "ua1",
               "explicitLanguage": "de", "explicitTitle": "Bericht"})
    );
    assert_eq!(
        server.requests_to(&format!("GET /v1/jobs/{JOB_ID}")).len(),
        2
    );
    assert_eq!(
        files(Path::new(&out)),
        ["report.original.pdf", "report.remediated.pdf"]
    );
}

#[test]
fn html_modes_and_content_selector() {
    for (flags, mode, selector) in [
        (&[][..], "generate_fix", None),
        (
            &["--analyze-only", "--content-selector", "main"][..],
            "generate_analyze",
            Some("main"),
        ),
    ] {
        let server = TestServer::start();
        let tmp = tempfile::tempdir().unwrap();
        let file = input(tmp.path(), "page.html", b"<!doctype html><title>T</title>");
        upload(&server, UPLOAD_ID);
        server.on(
            "POST /v1/html-to-pdf/jobs",
            Reply::json(200, &job("job-completed-fix", JOB_ID)),
        );
        results(&server, JOB_ID, &[("generated_pdf", PDF)]);
        let out = out_dir(tmp.path());
        let mut argv = vec!["html", file.as_str(), "--out", out.as_str()];
        argv.extend(flags);
        let r = run(&server, &argv);
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(
            server.requests_to("POST /v1/uploads")[0].json()["contentType"],
            "text/html"
        );
        let mut expected = json!({"uploadId": UPLOAD_ID, "mode": mode, "profile": "ua1"});
        if let Some(s) = selector {
            expected["contentSelector"] = json!(s);
        }
        assert_eq!(
            server.requests_to("POST /v1/html-to-pdf/jobs")[0].json(),
            expected
        );
        assert_eq!(files(Path::new(&out)), ["page.generated_pdf.pdf"]);
    }
}

#[test]
fn scan_posts_the_url_and_names_the_output_after_the_host() {
    let server = TestServer::start();
    let tmp = tempfile::tempdir().unwrap();
    let mut done = job("job-completed", JOB_ID);
    done["kind"] = json!("web");
    done["targetUrl"] = json!("https://example.com/page");
    server.on("POST /v1/web/jobs", Reply::json(200, &done));
    results(&server, JOB_ID, &[("web_report", b"{}")]);
    let out = out_dir(tmp.path());
    let r = run(
        &server,
        &["scan", "https://Example.com:8443/page?q=1", "--out", &out],
    );
    assert_eq!(r.code, 2, "{}", r.stderr);
    assert_eq!(
        server.requests_to("POST /v1/web/jobs")[0].json(),
        json!({"url": "https://Example.com:8443/page?q=1"})
    );
    assert!(server.requests_to("POST /v1/uploads").is_empty());
    assert_eq!(files(Path::new(&out)), ["example.com.web_report.json"]);
}

#[test]
fn host_names_follow_core() {
    for (url, host) in [
        ("https://Example.COM/a", Some("example.com")),
        ("http://user:pw@example.com:8080/?x#y", Some("example.com")),
        ("https://[2001:db8::1]:443/", Some("[2001:db8::1]")),
        ("example.com", None),
        ("https:///path", None),
    ] {
        assert_eq!(remote::host_of(url).as_deref(), host, "{url}");
    }
}

fn document_dir(tmp: &Path) -> PathBuf {
    let dir = tmp.join("doc");
    fs::create_dir_all(dir.join("img")).unwrap();
    fs::write(dir.join("img").join("logo.png"), PNG).unwrap();
    dir
}

fn document(dir: &Path, images: &[&str]) -> String {
    let mut body = vec![json!({"type": "heading", "level": 1, "text": "Invoice"})];
    for file in images {
        body.push(json!({"type": "image", "file": file, "alt": "Company logo"}));
    }
    input(
        dir,
        "invoice.json",
        &serde_json::to_vec(&json!({"title": "Invoice", "language": "en", "body": body})).unwrap(),
    )
}

#[test]
fn document_uploads_each_image_file_once_and_replaces_every_reference() {
    let server = TestServer::start();
    let tmp = tempfile::tempdir().unwrap();
    let dir = document_dir(tmp.path());
    let file = document(&dir, &["img/logo.png", "./img/../img/logo.png"]);
    upload(&server, "33333333-3333-4333-8333-333333333333");
    server.on(
        "POST /v1/documents/jobs",
        Reply::json(200, &job("document-job-prefer-wait-finished", JOB_ID)),
    );
    results(&server, JOB_ID, &[("generated_pdf", PDF)]);
    let out = out_dir(tmp.path());
    let r = run(&server, &["document", &file, "--out", &out]);
    assert_eq!(r.code, 0, "{}", r.stderr);

    let uploads = server.requests_to("POST /v1/uploads");
    assert_eq!(uploads.len(), 1);
    assert_eq!(
        uploads[0].json(),
        json!({"fileName": "logo.png", "sizeBytes": PNG.len(), "sha256": sha256_hex(PNG),
               "contentType": "image/png"})
    );
    assert_eq!(
        server
            .requests_to("PUT /put/33333333-3333-4333-8333-333333333333")
            .len(),
        1
    );
    let sent = server.requests_to("POST /v1/documents/jobs")[0].json();
    let image = json!({"type": "image", "alt": "Company logo",
                       "uploadId": "33333333-3333-4333-8333-333333333333"});
    assert_eq!(sent["body"][1], image);
    assert_eq!(sent["body"][2], image);
    assert_eq!(sent["title"], "Invoice");
    assert_eq!(files(Path::new(&out)), ["invoice.generated_pdf.pdf"]);
}

#[test]
fn refused_images_print_core_invalid_document_output_and_exit_64() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = document_dir(tmp.path());
    fs::write(tmp.path().join("secret.png"), PNG).unwrap();
    fs::write(dir.join("logo.gif"), b"GIF89a").unwrap();
    let absolute = dir
        .join("img")
        .join("logo.png")
        .to_string_lossy()
        .into_owned();
    let file = document(&dir, &[&absolute, "../secret.png", "logo.gif"]);
    let server = TestServer::start();
    let r = run(&server, &["document", &file, "--out", &out_dir(tmp.path())]);
    assert_eq!(r.code, 64);
    assert_eq!(r.stdout, "");
    assert_eq!(
        r.stderr,
        format!(
            "koldera: invalid document {file}\n\
             \x20 /body/1  FILE_IMAGE_INVALID: an image file path must be relative to the JSON file.\n\
             \x20 /body/2  FILE_IMAGE_INVALID: the image file is outside the directory of the JSON file.\n\
             \x20 /body/3  FILE_IMAGE_INVALID: an image file must end in .png, .jpg, .jpeg or .svg.\n"
        )
    );
    assert!(server.requests().is_empty());
}

#[test]
fn an_upload_id_in_the_file_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let doc = json!({"title": "T", "language": "en", "body": [
        {"type": "image", "uploadId": "33333333-3333-4333-8333-333333333333", "alt": "x"}]});
    let file = input(tmp.path(), "d.json", doc.to_string().as_bytes());
    let server = TestServer::start();
    let r = run(&server, &["document", &file]);
    assert_eq!(r.code, 64);
    assert!(
        r.stderr
            .contains("  /body/0  UPLOAD_ID_NOT_ALLOWED: uploadId is only available over HTTP"),
        "{}",
        r.stderr
    );
    assert!(server.requests().is_empty());
}

#[test]
fn a_server_422_for_the_document_is_printed_like_core_and_exits_64() {
    let server = TestServer::start();
    let tmp = tempfile::tempdir().unwrap();
    let dir = document_dir(tmp.path());
    let file = document(&dir, &[]);
    server.on(
        "POST /v1/documents/jobs",
        Reply::golden("document-job-validation-errors"),
    );
    let r = run(&server, &["document", &file]);
    assert_eq!(r.code, 64);
    assert_eq!(
        r.stderr,
        format!(
            "koldera: invalid document {file}\n\
             \x20 /body/0/level  HEADING_LEVEL_SKIPPED: The first heading must be level 1 or 2.\n\
             \x20 /body/1/alt  IMAGE_ALT_MISSING: An image needs a description.\n"
        )
    );
}

#[test]
fn bad_json_and_missing_inputs_are_usage_errors() {
    let server = TestServer::start();
    let tmp = tempfile::tempdir().unwrap();
    let file = input(tmp.path(), "d.json", b"{\"title\": ");
    let r = run(&server, &["document", &file]);
    assert_eq!(r.code, 64);
    assert!(
        r.stderr
            .starts_with(&format!("koldera: {file} is not valid JSON: ")),
        "{}",
        r.stderr
    );
    for (command, name) in [
        ("pdf", "none.pdf"),
        ("html", "none.html"),
        ("document", "none.json"),
    ] {
        let missing = tmp.path().join(name).to_string_lossy().into_owned();
        let r = run(&server, &[command, &missing]);
        assert_eq!(r.code, 64);
        assert_eq!(
            r.stderr,
            format!(
                "koldera: file not found: {missing}\n{}\n",
                args::usage_of(command)
            )
        );
    }
    let directory = tmp.path().to_string_lossy().into_owned();
    assert_eq!(run(&server, &["pdf", &directory]).code, 64);
    assert!(server.requests().is_empty());
}

#[test]
fn a_422_for_the_request_prints_code_message_and_field_errors_and_exits_64() {
    let server = TestServer::start();
    let tmp = tempfile::tempdir().unwrap();
    let file = input(tmp.path(), "sample.pdf", PDF);
    upload(&server, UPLOAD_ID);
    server.on(
        "POST /v1/pdf/jobs",
        Reply::golden("pdf-job-validation-error"),
    );
    let r = run(&server, &["pdf", &file, "--title", "x"]);
    assert_eq!(r.code, 64);
    assert_eq!(
        r.stderr,
        "koldera: VALIDATION_ERROR: The upload is not complete.\n"
    );

    let server = TestServer::start();
    server.on("POST /v1/web/jobs", Reply::golden("web-job-url-rejected"));
    let r = run(&server, &["scan", "https://internal.invalid/"]);
    assert_eq!(r.code, 64);
    assert_eq!(
        r.stderr,
        "koldera: VALIDATION_ERROR: url rejected: private destination\n"
    );

    let server = TestServer::start();
    server.on("POST /v1/uploads", Reply::golden("upload-ticket-too-large"));
    let r = run(&server, &["pdf", &file]);
    assert_eq!(r.code, 64);
    assert_eq!(
        r.stderr,
        "koldera: FILE_TOO_LARGE: The configured limit is 26214400 bytes.\n"
    );
}

#[test]
fn refused_credentials_exit_3_with_a_hint() {
    for name in ["pdf-job-unauthorized", "pdf-job-insufficient-scope"] {
        let server = TestServer::start();
        let tmp = tempfile::tempdir().unwrap();
        let file = input(tmp.path(), "sample.pdf", PDF);
        upload(&server, UPLOAD_ID);
        server.on("POST /v1/pdf/jobs", Reply::golden(name));
        let r = run(&server, &["pdf", &file]);
        assert_eq!(r.code, 3);
        let code = fixture(name)["response"]["body"]["code"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            r.stderr.starts_with(&format!("koldera: {code}: ")),
            "{}",
            r.stderr
        );
        assert!(r.stderr.contains("KOLDERA_TOKEN"), "{}", r.stderr);
    }
}

#[test]
fn other_server_failures_exit_3() {
    let server = TestServer::start();
    server.on("POST /v1/web/jobs", Reply::golden("pdf-job-queue-full"));
    let r = run(&server, &["scan", "https://example.com/"]);
    assert_eq!(r.code, 3);
    assert!(r.stderr.contains("QUEUE_FULL"), "{}", r.stderr);
    assert!(!r.stderr.contains("KOLDERA_TOKEN"), "{}", r.stderr);
}

#[test]
fn a_network_failure_exits_3_without_the_server_url() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let dead = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let api = Api::new(&dead, Some(TOKEN.into()))
        .unwrap()
        .with_sleep(|_| {});
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let command = Command::Scan {
        url: "https://example.com/".into(),
        out: None,
        json: false,
    };
    let code = remote::run(&api, &command, &mut out, &mut err);
    let stderr = String::from_utf8(err).unwrap();
    assert_eq!(code, 3);
    assert!(stderr.starts_with("koldera: "), "{stderr}");
    assert!(
        !stderr.contains(&dead) && !stderr.contains(TOKEN),
        "{stderr}"
    );
    assert!(out.is_empty());
}

#[test]
fn a_failed_job_still_prints_its_summary_and_exits_3() {
    let server = TestServer::start();
    let mut failed = job("job-failed", JOB_ID);
    failed["kind"] = json!("web");
    server.on("POST /v1/web/jobs", Reply::json(200, &failed));
    results(&server, JOB_ID, &[]);
    let r = run(&server, &["scan", "https://example.com/"]);
    assert_eq!(r.code, 3);
    assert!(
        r.stdout.starts_with(&format!("Job {JOB_ID}: failed")),
        "{}",
        r.stdout
    );
}

#[test]
fn json_output_has_host_paths_and_nothing_else() {
    let server = TestServer::start();
    let tmp = tempfile::tempdir().unwrap();
    let file = input(tmp.path(), "sample.pdf", PDF);
    upload(&server, UPLOAD_ID);
    server.on(
        "POST /v1/pdf/jobs",
        Reply::json(200, &job("job-completed-fix", JOB_ID)),
    );
    results(
        &server,
        JOB_ID,
        &[("original", PDF), ("remediated", b"%PDF-fixed")],
    );
    fs::create_dir_all(tmp.path().join("out")).unwrap();
    fs::write(tmp.path().join("out").join("sample.original.pdf"), b"keep").unwrap();
    let out = out_dir(tmp.path());
    let r = run(&server, &["pdf", &file, "--fix", "--out", &out, "--json"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.ends_with('\n') && r.stdout.lines().count() == 1);
    let v: Value = serde_json::from_str(&r.stdout).unwrap();
    let paths: Vec<PathBuf> = v["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| PathBuf::from(a["path"].as_str().unwrap()))
        .collect();
    let out_abs = std::path::absolute(&out).unwrap();
    assert_eq!(
        paths,
        [
            out_abs.join("sample.original-1.pdf"),
            out_abs.join("sample.remediated.pdf")
        ]
    );
    assert_eq!(fs::read(&paths[0]).unwrap(), PDF);
    assert_eq!(
        fs::read(out_abs.join("sample.original.pdf")).unwrap(),
        b"keep"
    );
    assert!(!r.stdout.contains("downloadUrl"));
    assert_eq!(v["job"]["id"], JOB_ID);
}

#[test]
fn a_download_that_does_not_match_writes_nothing_and_exits_3() {
    let server = TestServer::start();
    let tmp = tempfile::tempdir().unwrap();
    let file = input(tmp.path(), "sample.pdf", PDF);
    upload(&server, UPLOAD_ID);
    server.on(
        "POST /v1/pdf/jobs",
        Reply::json(200, &job("job-completed-fix", JOB_ID)),
    );
    // Queued first, so the one download of the remediated copy gets these bytes.
    server.on("GET /dl/1", Reply::status(200).body(b"%PDF-other".to_vec()));
    results(
        &server,
        JOB_ID,
        &[("original", PDF), ("remediated", b"%PDF-fixed")],
    );
    let out = out_dir(tmp.path());
    let r = run(&server, &["pdf", &file, "--out", &out]);
    assert_eq!(r.code, 3);
    assert!(r.stderr.contains("ARTIFACT_MISMATCH"), "{}", r.stderr);
    assert!(files(Path::new(&out)).is_empty());
    assert_eq!(r.stdout, "");
}

#[test]
fn only_plain_http_to_a_remote_host_warns() {
    let warning = "koldera: warning: the server URL is not HTTPS; the token is sent in clear text";
    for (base, warns) in [
        ("http://core.example.com", true),
        ("HTTP://10.0.0.5:8080", true),
        ("http://localhost:8080", false),
        ("http://127.0.0.1:8080", false),
        ("http://127.1.2.3", false),
        ("http://[::1]:8080", false),
        ("https://core.example.com", false),
    ] {
        assert_eq!(
            remote::clear_text_warning(base),
            warns.then_some(warning),
            "{base}"
        );
    }
}

#[cfg(windows)]
#[test]
fn file_names_differing_only_in_case_are_one_upload_on_windows() {
    let server = TestServer::start();
    let tmp = tempfile::tempdir().unwrap();
    let dir = document_dir(tmp.path());
    let file = document(&dir, &["img/LOGO.png", "img/logo.png"]);
    upload(&server, "33333333-3333-4333-8333-333333333333");
    server.on(
        "POST /v1/documents/jobs",
        Reply::json(200, &job("document-job-prefer-wait-finished", JOB_ID)),
    );
    results(&server, JOB_ID, &[]);
    let r = run(&server, &["document", &file]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(server.requests_to("POST /v1/uploads").len(), 1);
    let sent = server.requests_to("POST /v1/documents/jobs")[0].json();
    assert_eq!(sent["body"][1]["uploadId"], sent["body"][2]["uploadId"]);
}

#[test]
fn an_upload_declaration_refused_for_a_document_image_is_not_an_invalid_document() {
    let server = TestServer::start();
    let tmp = tempfile::tempdir().unwrap();
    let dir = document_dir(tmp.path());
    let file = document(&dir, &["img/logo.png"]);
    let problem = json!({"type": "about:blank", "title": "Invalid upload", "status": 422,
        "code": "VALIDATION_ERROR", "detail": "contentType is not allowed.",
        "errors": [{"path": "/contentType", "code": "UNSUPPORTED", "message": "Not allowed."}]});
    server.on("POST /v1/uploads", Reply::json(422, &problem));
    let r = run(&server, &["document", &file]);
    assert_eq!(r.code, 64);
    assert_eq!(
        r.stderr,
        "koldera: VALIDATION_ERROR: contentType is not allowed.\n  /contentType  UNSUPPORTED: Not allowed.\n"
    );
    assert!(server.requests_to("POST /v1/documents/jobs").is_empty());
}
