mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use koldera_cli::api::{
    Api, ApiError, HtmlJobRequest, PdfJobRequest, Ticket, WAIT_SECONDS, WebJobRequest,
};
use koldera_cli::model::{Artifact, Items, Validation};
use serde_json::{Value, json};
use support::server::{Reply, TestServer, fixture};

const TOKEN: &str = "test-server-token";
const PDF: &[u8] = b"%PDF-1.4\n1 0 obj<<>>endobj\ntrailer<<>>\n%%EOF\n";
const SIG: &str = "sig=ZmFrZS1zaWduYXR1cmUtZm9yLXRlc3Rz";
const JOB_ID: &str = "4d2d5a6e-2b1c-4e4a-9d6b-1f3c5e7a9b01";

type Sleeps = Arc<Mutex<Vec<Duration>>>;

fn api(server: &TestServer) -> (Api, Sleeps) {
    let sleeps: Sleeps = Arc::default();
    let record = sleeps.clone();
    let api = Api::new(&server.base, Some(TOKEN.to_owned()))
        .unwrap()
        .with_sleep(move |d| record.lock().unwrap().push(d));
    (api, sleeps)
}

fn slept(sleeps: &Sleeps) -> Vec<u64> {
    sleeps.lock().unwrap().iter().map(|d| d.as_secs()).collect()
}

fn artifacts(server: &TestServer) -> Vec<Artifact> {
    let body = server.with_base(&fixture("artifacts")["response"]["body"]);
    serde_json::from_value::<Items<Artifact>>(body)
        .unwrap()
        .items
}

fn pdf_request() -> PdfJobRequest {
    PdfJobRequest::new("11111111-1111-4111-8111-111111111111", "fix")
}

fn assert_secret_free(server: &TestServer, err: &ApiError) {
    for text in [err.to_string(), format!("{err:?}")] {
        assert!(!text.contains(TOKEN), "{text}");
        assert!(!text.contains("sig="), "{text}");
        assert!(!text.contains("exp="), "{text}");
        assert!(!text.contains(&server.base), "{text}");
    }
}

#[test]
fn new_accepts_absolute_http_urls_and_trims_the_trailing_slash() {
    let a = Api::new("https://core.example.com/", None).unwrap();
    assert_eq!(a.base_url(), "https://core.example.com");
    let a = Api::new("http://127.0.0.1:8080/base//", Some(TOKEN.into())).unwrap();
    assert_eq!(a.base_url(), "http://127.0.0.1:8080/base");
    for bad in [
        "",
        "core.example.com",
        "/v1",
        "ftp://core.example.com",
        "https://",
        "https://user:pass@core.example.com",
        "https://core.example.com/?a=b",
        "https://core.example.com/#x",
        "https://core example.com",
    ] {
        let err = Api::new(bad, None).unwrap_err();
        assert!(err.message.contains("http"), "{bad}: {}", err.message);
    }
}

#[test]
fn new_refuses_a_token_that_cannot_be_sent_in_a_header() {
    let err = Api::new("https://core.example.com", Some("bad\ntoken".into())).unwrap_err();
    assert!(!err.message.contains("bad"), "{}", err.message);
}

#[test]
fn the_token_never_appears_in_debug_output() {
    let a = Api::new("https://core.example.com", Some(TOKEN.into())).unwrap();
    assert!(!format!("{a:?}").contains(TOKEN));
}

#[test]
fn upload_sends_the_bearer_to_the_api_and_nothing_to_storage() {
    let server = TestServer::start();
    server
        .on("POST /v1/uploads", Reply::golden("upload-ticket-pdf"))
        .on(
            "PUT /v1/uploads/11111111-1111-4111-8111-111111111111/content",
            Reply::golden("upload-put-pdf"),
        );
    let (api, _) = api(&server);
    let ticket = api
        .create_upload("sample.pdf", "application/pdf", PDF)
        .unwrap();
    assert_eq!(ticket.upload_id, "11111111-1111-4111-8111-111111111111");
    assert_eq!(ticket.max_bytes, Some(26214400));
    assert!(!format!("{ticket:?}").contains(SIG));
    api.put_upload(&ticket, PDF).unwrap();

    let golden = fixture("upload-ticket-pdf");
    let post = &server.requests_to("POST /v1/uploads")[0];
    assert_eq!(
        post.header("authorization"),
        Some("Bearer test-server-token")
    );
    assert_eq!(post.header("content-type"), Some("application/json"));
    assert!(post.header("idempotency-key").is_some());
    assert_eq!(post.json(), golden["request"]["body"]);

    let put = &server.requests()[1];
    assert_eq!(put.method, "PUT");
    assert!(put.target.ends_with(SIG));
    assert_eq!(put.header("authorization"), None);
    assert_eq!(put.header("content-type"), Some("application/pdf"));
    assert_eq!(put.body, PDF);
}

#[test]
fn no_token_means_no_authorization_header() {
    let server = TestServer::start();
    server.on("GET /v1/jobs/x/findings", Reply::golden("findings"));
    let api = Api::new(&server.base, None).unwrap();
    api.findings("x").unwrap();
    assert_eq!(server.requests()[0].header("authorization"), None);
}

#[test]
fn upload_bigger_than_the_ticket_limit_is_not_sent() {
    let server = TestServer::start();
    let (api, _) = api(&server);
    let ticket = Ticket {
        max_bytes: Some(4),
        ..ticket(&server, false)
    };
    let err = api.put_upload(&ticket, PDF).unwrap_err();
    assert_eq!(err.code, "FILE_TOO_LARGE");
    assert!(server.requests().is_empty());
}

fn ticket(server: &TestServer, replayed: bool) -> Ticket {
    let golden = server.with_base(&fixture("upload-ticket-pdf")["response"]["body"]);
    let mut t: Ticket = serde_json::from_value(golden).unwrap();
    t.content_type = "application/pdf".into();
    t.replayed = replayed;
    t
}

const PUT_ROUTE: &str = "PUT /v1/uploads/11111111-1111-4111-8111-111111111111/content";

#[test]
fn already_completed_upload_fails_on_a_first_attempt() {
    let server = TestServer::start();
    server.on(PUT_ROUTE, Reply::golden("upload-put-already-completed"));
    let (api, _) = api(&server);
    let err = api.put_upload(&ticket(&server, false), PDF).unwrap_err();
    assert_eq!(err.status, Some(409));
    assert_eq!(err.code, "UPLOAD_ALREADY_COMPLETED");
    assert_secret_free(&server, &err);
}

#[test]
fn already_completed_upload_counts_as_stored_after_a_replayed_ticket() {
    let server = TestServer::start();
    server
        .on(
            "POST /v1/uploads",
            Reply::golden("upload-ticket-pdf").header("Idempotent-Replayed", "true"),
        )
        .on(PUT_ROUTE, Reply::golden("upload-put-already-completed"));
    let (api, _) = api(&server);
    let ticket = api
        .create_upload("sample.pdf", "application/pdf", PDF)
        .unwrap();
    assert!(ticket.replayed);
    api.put_upload(&ticket, PDF).unwrap();
}

#[test]
fn already_completed_upload_counts_as_stored_after_a_retried_put() {
    let server = TestServer::start();
    server
        .on(PUT_ROUTE, Reply::dropped())
        .on(PUT_ROUTE, Reply::golden("upload-put-already-completed"));
    let (api, _) = api(&server);
    api.put_upload(&ticket(&server, false), PDF).unwrap();
    assert_eq!(server.requests_to(PUT_ROUTE).len(), 2);
}

#[test]
fn storage_problems_are_mapped_without_the_signed_url() {
    for (name, status, code) in [
        ("upload-put-signature-invalid", 403, "SIGNATURE_INVALID"),
        ("upload-put-expired", 410, "UPLOAD_EXPIRED"),
        ("upload-put-checksum-mismatch", 422, "CHECKSUM_MISMATCH"),
    ] {
        let server = TestServer::start();
        server.on(PUT_ROUTE, Reply::golden(name));
        let (api, _) = api(&server);
        let err = api.put_upload(&ticket(&server, true), PDF).unwrap_err();
        assert_eq!((err.status, err.code.as_str()), (Some(status), code));
        assert_secret_free(&server, &err);
    }
}

#[test]
fn storage_redirects_are_not_followed() {
    let server = TestServer::start();
    let elsewhere = format!("{}/elsewhere", server.base);
    server.on(PUT_ROUTE, Reply::status(307).header("Location", &elsewhere));
    let (api, _) = api(&server);
    let err = api.put_upload(&ticket(&server, false), PDF).unwrap_err();
    assert_eq!(err.status, Some(307));
    assert!(server.requests_to("PUT /elsewhere").is_empty());
    assert_secret_free(&server, &err);
}

#[test]
fn storage_transport_failure_is_reported_without_the_url() {
    let server = TestServer::start();
    server.on(PUT_ROUTE, Reply::dropped());
    let (api, _) = api(&server);
    let err = api.put_upload(&ticket(&server, false), PDF).unwrap_err();
    assert_eq!(err.status, None);
    assert_eq!(server.requests_to(PUT_ROUTE).len(), 4);
    assert_secret_free(&server, &err);
}

#[test]
fn creation_waits_and_reports_a_200_as_finished() {
    let server = TestServer::start();
    server.on(
        "POST /v1/pdf/jobs",
        Reply::golden("pdf-job-prefer-wait-finished"),
    );
    let (api, _) = api(&server);
    let created = api.create_pdf_job(&pdf_request()).unwrap();
    assert!(created.finished);
    assert_eq!(created.job.state, "completed");
    let post = &server.requests()[0];
    assert_eq!(post.header("prefer"), Some("wait=60"));
    assert_eq!(WAIT_SECONDS, 60);
    assert_eq!(
        post.json(),
        fixture("pdf-job-prefer-wait-finished")["request"]["body"]
    );
}

#[test]
fn a_202_is_not_finished_and_is_polled_until_terminal() {
    let server = TestServer::start();
    server
        .on(
            "POST /v1/pdf/jobs",
            Reply::golden("pdf-job-prefer-wait-timeout"),
        )
        .on(
            &format!("GET /v1/jobs/{JOB_ID}"),
            Reply::golden("job-queued-retry-after").header("Retry-After", "3"),
        )
        .on(
            &format!("GET /v1/jobs/{JOB_ID}"),
            Reply::golden("job-running"),
        )
        .on(
            &format!("GET /v1/jobs/{JOB_ID}"),
            Reply::golden("job-completed"),
        );
    let (api, sleeps) = api(&server);
    let created = api.create_pdf_job(&pdf_request()).unwrap();
    assert!(!created.finished);
    assert_eq!(created.job.state, "running");
    let job = api.wait_job(&created.job.id).unwrap();
    assert_eq!(job.state, "completed");
    assert_eq!(slept(&sleeps), vec![3, 2]);
    let gets = server.requests_to(&format!("GET /v1/jobs/{JOB_ID}"));
    assert_eq!(gets.len(), 3);
    assert_eq!(
        gets[0].header("authorization"),
        Some("Bearer test-server-token")
    );
}

#[test]
fn a_200_that_is_not_terminal_is_not_finished() {
    let server = TestServer::start();
    let running = fixture("job-running")["response"]["body"].clone();
    server.on("POST /v1/pdf/jobs", Reply::json(200, &running));
    let (api, _) = api(&server);
    assert!(!api.create_pdf_job(&pdf_request()).unwrap().finished);
}

#[test]
fn a_202_is_never_finished_even_with_a_terminal_body() {
    let server = TestServer::start();
    let completed = fixture("job-completed")["response"]["body"].clone();
    server.on("POST /v1/pdf/jobs", Reply::json(202, &completed));
    let (api, _) = api(&server);
    assert!(!api.create_pdf_job(&pdf_request()).unwrap().finished);
}

#[test]
fn polling_without_retry_after_backs_off_from_one_to_five_seconds() {
    let server = TestServer::start();
    let route = format!("GET /v1/jobs/{JOB_ID}");
    let running = Reply::golden("job-running").without_header("Retry-After");
    for _ in 0..5 {
        server.on(&route, running.clone());
    }
    server.on(&route, Reply::golden("job-failed"));
    let (api, sleeps) = api(&server);
    let job = api.wait_job(JOB_ID).unwrap();
    assert_eq!(job.state, "failed");
    assert_eq!(slept(&sleeps), vec![1, 2, 4, 5, 5]);
}

#[test]
fn reads_retry_429_and_503_honouring_retry_after() {
    let server = TestServer::start();
    let busy = |status: u16, after: &str| {
        Reply::json(
            status,
            &json!({"type": "about:blank", "title": "Busy", "status": status, "code": "BUSY"}),
        )
        .header("Retry-After", after)
    };
    server
        .on("GET /v1/jobs/x/findings", busy(503, "3"))
        .on("GET /v1/jobs/x/findings", busy(429, "7"))
        .on("GET /v1/jobs/x/findings", Reply::golden("findings"));
    server
        .on("GET /v1/jobs/x/actions", busy(503, "1"))
        .on("GET /v1/jobs/x/actions", Reply::golden("actions"));
    server.on("GET /v1/jobs/x/artifacts", Reply::golden("artifacts"));
    let (api, sleeps) = api(&server);
    assert!(!api.findings("x").unwrap().is_empty());
    assert!(!api.actions("x").unwrap().is_empty());
    assert_eq!(api.artifacts("x").unwrap().len(), 2);
    assert_eq!(slept(&sleeps), vec![3, 7, 1]);
}

#[test]
fn reads_give_up_after_three_busy_retries() {
    let server = TestServer::start();
    server.on(
        "GET /v1/jobs/x/findings",
        Reply::golden("pdf-job-queue-full").header("Retry-After", "1"),
    );
    let (api, _) = api(&server);
    let err = api.findings("x").unwrap_err();
    assert_eq!((err.status, err.code.as_str()), (Some(503), "QUEUE_FULL"));
    assert_eq!(server.requests().len(), 4);
}

#[test]
fn creation_is_retried_with_the_same_key_after_a_dropped_connection() {
    let server = TestServer::start();
    server
        .on("POST /v1/pdf/jobs", Reply::dropped())
        .on("POST /v1/pdf/jobs", Reply::dropped())
        .on(
            "POST /v1/pdf/jobs",
            Reply::golden("pdf-job-idempotent-replay"),
        );
    let (api, _) = api(&server);
    let created = api.create_pdf_job(&pdf_request()).unwrap();
    assert!(!created.finished);
    let posts = server.requests_to("POST /v1/pdf/jobs");
    assert_eq!(posts.len(), 3);
    let key = posts[0].header("idempotency-key").unwrap().to_owned();
    assert!(uuid_like(&key), "{key}");
    assert!(
        posts
            .iter()
            .all(|p| p.header("idempotency-key") == Some(key.as_str()))
    );
    assert!(posts.iter().all(|p| p.body == posts[0].body));

    api.create_pdf_job(&pdf_request()).unwrap();
    let next = server.requests_to("POST /v1/pdf/jobs");
    assert_ne!(next[3].header("idempotency-key"), Some(key.as_str()));
}

fn uuid_like(key: &str) -> bool {
    key.len() == 36 && key.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

#[test]
fn creation_gives_up_after_three_transport_retries() {
    let server = TestServer::start();
    server.on("POST /v1/web/jobs", Reply::dropped());
    let (api, _) = api(&server);
    let err = api
        .create_web_job(&WebJobRequest::new("https://example.com/"))
        .unwrap_err();
    assert_eq!(err.status, None);
    assert_eq!(err.code, "TRANSPORT_ERROR");
    assert_eq!(server.requests().len(), 4);
    assert_secret_free(&server, &err);
}

#[test]
fn html_web_and_document_jobs_post_their_bodies() {
    let server = TestServer::start();
    server
        .on(
            "POST /v1/html-to-pdf/jobs",
            Reply::golden("html-job-analyze"),
        )
        .on("POST /v1/web/jobs", Reply::golden("web-job-accepted"))
        .on(
            "POST /v1/documents/jobs",
            Reply::golden("document-job-prefer-wait-finished"),
        );
    let (api, _) = api(&server);
    let mut html = HtmlJobRequest::new("22222222-2222-4222-8222-222222222222", "generate_analyze");
    html.content_selector = Some("main".into());
    api.create_html_job(&html).unwrap();
    api.create_web_job(&WebJobRequest::new("https://example.com/"))
        .unwrap();
    let document = fixture("document-job-prefer-wait-finished")["request"]["body"].clone();
    assert!(api.create_document_job(&document).unwrap().finished);

    let r = server.requests();
    assert_eq!(
        r[0].json(),
        json!({"uploadId": "22222222-2222-4222-8222-222222222222", "mode": "generate_analyze",
               "profile": "ua1", "contentSelector": "main"})
    );
    assert_eq!(r[1].json(), json!({"url": "https://example.com/"}));
    assert_eq!(r[2].json(), document);
    assert!(r.iter().all(|q| q.header("prefer") == Some("wait=60")));
}

#[test]
fn pdf_request_omits_unset_options() {
    let mut r = pdf_request();
    assert_eq!(
        serde_json::to_value(&r).unwrap(),
        json!({"uploadId": "11111111-1111-4111-8111-111111111111", "mode": "fix", "profile": "ua1"})
    );
    r.explicit_language = Some("de".into());
    r.explicit_title = Some("T".into());
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["explicitLanguage"], "de");
    assert_eq!(v["explicitTitle"], "T");
}

#[test]
fn problems_are_mapped_to_status_and_code() {
    let cases: [(&str, &str, u16, &str); 8] = [
        (
            "POST /v1/pdf/jobs",
            "pdf-job-unauthorized",
            401,
            "UNAUTHORIZED",
        ),
        (
            "POST /v1/pdf/jobs",
            "pdf-job-insufficient-scope",
            403,
            "INSUFFICIENT_SCOPE",
        ),
        ("POST /v1/pdf/jobs", "pdf-job-feature-off", 404, "NOT_FOUND"),
        (
            "POST /v1/pdf/jobs",
            "pdf-job-idempotency-key-reused",
            409,
            "IDEMPOTENCY_KEY_REUSED",
        ),
        (
            "POST /v1/pdf/jobs",
            "pdf-job-unsupported-input",
            415,
            "UNSUPPORTED_INPUT",
        ),
        ("POST /v1/pdf/jobs", "pdf-job-queue-full", 503, "QUEUE_FULL"),
        (
            "POST /v1/web/jobs",
            "web-job-host-not-allowed",
            421,
            "HOST_NOT_ALLOWED",
        ),
        (
            "POST /v1/web/jobs",
            "web-job-url-rejected",
            422,
            "VALIDATION_ERROR",
        ),
    ];
    for (route, name, status, code) in cases {
        let server = TestServer::start();
        server.on(route, Reply::golden(name));
        let (api, sleeps) = api(&server);
        let err = if route.contains("web") {
            api.create_web_job(&WebJobRequest::new("https://example.com/"))
        } else {
            api.create_pdf_job(&pdf_request())
        }
        .unwrap_err();
        assert_eq!(
            (err.status, err.code.as_str()),
            (Some(status), code),
            "{name}"
        );
        let detail = fixture(name)["response"]["body"]["detail"].clone();
        if let Some(detail) = detail.as_str() {
            assert_eq!(err.message, detail);
        }
        assert!(err.to_string().contains(code), "{err}");
        assert_eq!(server.requests().len(), 1, "{name}: no retry");
        assert!(slept(&sleeps).is_empty());
        assert_secret_free(&server, &err);
    }
}

#[test]
fn job_read_problems_are_mapped() {
    for (name, status, code) in [
        ("job-unauthorized", 401, "UNAUTHORIZED"),
        ("job-insufficient-scope", 403, "INSUFFICIENT_SCOPE"),
        ("job-not-found", 404, "NOT_FOUND"),
    ] {
        let server = TestServer::start();
        server.on(&format!("GET /v1/jobs/{JOB_ID}"), Reply::golden(name));
        let (api, _) = api(&server);
        let err = api.wait_job(JOB_ID).unwrap_err();
        assert_eq!((err.status, err.code.as_str()), (Some(status), code));
    }
}

#[test]
fn a_422_carries_the_field_errors() {
    let server = TestServer::start();
    server.on(
        "POST /v1/documents/jobs",
        Reply::golden("document-job-validation-errors"),
    );
    let (api, _) = api(&server);
    let body = fixture("document-job-validation-errors")["request"]["body"].clone();
    let err = api.create_document_job(&body).unwrap_err();
    assert_eq!(
        (err.status, err.code.as_str()),
        (Some(422), "VALIDATION_ERROR")
    );
    assert_eq!(err.errors.len(), 2);
    assert_eq!(err.errors[0].path, "/body/0/level");
    assert_eq!(err.errors[0].code, "HEADING_LEVEL_SKIPPED");
    assert_eq!(err.errors[1].message, "An image needs a description.");
}

#[test]
fn a_problem_without_a_json_body_gets_a_fallback_code() {
    let server = TestServer::start();
    server
        .on("GET /v1/jobs/a/findings", Reply::status(500).body("oops"))
        .on("GET /v1/jobs/b/findings", Reply::status(418).body("<html>"));
    let (api, _) = api(&server);
    let err = api.findings("a").unwrap_err();
    assert_eq!(
        (err.status, err.code.as_str()),
        (Some(500), "INTERNAL_ERROR")
    );
    let err = api.findings("b").unwrap_err();
    assert_eq!((err.status, err.code.as_str()), (Some(418), "UNKNOWN"));
}

#[test]
fn a_malformed_success_body_is_an_error() {
    let server = TestServer::start();
    server
        .on(
            "GET /v1/jobs/x/findings",
            Reply::json(200, &json!({"no": "items"})),
        )
        .on("POST /v1/pdf/jobs", Reply::status(202).body("not json"));
    let (api, _) = api(&server);
    assert_eq!(api.findings("x").unwrap_err().code, "MALFORMED_RESPONSE");
    assert_eq!(
        api.create_pdf_job(&pdf_request()).unwrap_err().code,
        "MALFORMED_RESPONSE"
    );
}

#[test]
fn job_ids_are_escaped_in_paths() {
    let server = TestServer::start();
    server.on(
        "GET /v1/jobs/a%2F..%3Fb/findings",
        Reply::golden("findings"),
    );
    let (api, _) = api(&server);
    api.findings("a/..?b").unwrap();
}

const DOWNLOAD_ROUTE: &str = "GET /v1/artifacts/aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaa1/content";

#[test]
fn download_verifies_and_sends_no_credential() {
    let server = TestServer::start();
    server.on(DOWNLOAD_ROUTE, Reply::golden("artifact-download-original"));
    let (api, _) = api(&server);
    let bytes = api.download(&artifacts(&server)[0]).unwrap();
    assert_eq!(bytes, PDF);
    let get = &server.requests()[0];
    assert!(get.target.ends_with(SIG));
    assert_eq!(get.header("authorization"), None);
}

#[test]
fn download_with_a_wrong_size_or_hash_returns_no_bytes() {
    let mut longer = PDF.to_vec();
    longer.push(b'\n');
    let mut flipped = PDF.to_vec();
    flipped[0] = b'#';
    for body in [longer, PDF[..10].to_vec(), flipped, vec![b'x'; 1 << 20]] {
        let server = TestServer::start();
        server.on(
            DOWNLOAD_ROUTE,
            Reply::golden("artifact-download-original").body(body),
        );
        let (api, _) = api(&server);
        let err = api.download(&artifacts(&server)[0]).unwrap_err();
        assert_eq!(err.code, "ARTIFACT_MISMATCH");
        assert!(err.to_string().contains("original"), "{err}");
        assert_secret_free(&server, &err);
    }
}

#[test]
fn download_problems_are_mapped_without_the_signed_url() {
    for (name, status, code) in [
        (
            "artifact-download-signature-invalid",
            403,
            "SIGNATURE_INVALID",
        ),
        ("artifact-download-not-found", 404, "NOT_FOUND"),
    ] {
        let server = TestServer::start();
        server.on(DOWNLOAD_ROUTE, Reply::golden(name));
        let (api, _) = api(&server);
        let err = api.download(&artifacts(&server)[0]).unwrap_err();
        assert_eq!((err.status, err.code.as_str()), (Some(status), code));
        assert_secret_free(&server, &err);
    }
}

#[test]
fn download_from_an_unreachable_host_hides_the_url() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let dead = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let server = TestServer::start();
    let (api, _) = api(&server);
    let mut artifact = artifacts(&server)[0].clone();
    artifact.download_url = artifact.download_url.replace(&server.base, &dead);
    let err = api.download(&artifact).unwrap_err();
    assert_eq!(err.status, None);
    for text in [err.to_string(), format!("{err:?}")] {
        assert!(!text.contains("sig=") && !text.contains(&dead), "{text}");
    }
}

#[test]
fn artifacts_never_serialize_the_download_url() {
    let server = TestServer::start();
    let a = &artifacts(&server)[0];
    let text = serde_json::to_string(a).unwrap();
    assert!(
        !text.contains("downloadUrl") && !text.contains(SIG),
        "{text}"
    );
    assert!(!format!("{a:?}").contains(SIG));
}

#[test]
fn validation_profile_and_status_default_when_missing() {
    let v: Validation = serde_json::from_value(json!({"failedRules": []})).unwrap();
    assert_eq!((v.profile.as_str(), v.status.as_str()), ("", ""));
    let _: Value = serde_json::to_value(&v).unwrap();
}
