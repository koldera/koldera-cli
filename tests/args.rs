use koldera_cli::args::{self, Command, Global, Invocation, UsageError};

fn parse(line: &[&str]) -> Result<Invocation, UsageError> {
    let argv: Vec<String> = line.iter().map(|s| s.to_string()).collect();
    args::parse(&argv)
}

fn command(line: &[&str]) -> Command {
    parse(line).expect("parses").command
}

fn error(line: &[&str]) -> UsageError {
    parse(line).expect_err("usage error")
}

fn local() -> Global {
    Global::default()
}

#[test]
fn pdf_with_every_flag() {
    let inv = parse(&[
        "pdf",
        "in.pdf",
        "--fix",
        "--language",
        "de",
        "--title",
        "Report",
        "--out",
        "o",
        "--json",
    ])
    .unwrap();
    assert_eq!(inv.global, local());
    assert_eq!(
        inv.command,
        Command::Pdf {
            file: "in.pdf".into(),
            fix: true,
            language: Some("de".into()),
            title: Some("Report".into()),
            out: Some("o".into()),
            json: true,
        }
    );
}

#[test]
fn pdf_defaults() {
    assert_eq!(
        command(&["pdf", "in.pdf"]),
        Command::Pdf {
            file: "in.pdf".into(),
            fix: false,
            language: None,
            title: None,
            out: None,
            json: false,
        }
    );
}

#[test]
fn html_scan_and_document() {
    assert_eq!(
        command(&[
            "html",
            "--analyze-only",
            "page.html",
            "--content-selector=main",
            "--out=o"
        ]),
        Command::Html {
            file: "page.html".into(),
            analyze_only: true,
            content_selector: Some("main".into()),
            out: Some("o".into()),
            json: false,
        }
    );
    assert_eq!(
        command(&["scan", "https://example.com/", "--json"]),
        Command::Scan {
            url: "https://example.com/".into(),
            out: None,
            json: true,
        }
    );
    assert_eq!(
        command(&["document", "doc.json", "--out", "o"]),
        Command::Document {
            file: "doc.json".into(),
            out: Some("o".into()),
            json: false,
        }
    );
}

#[test]
fn flag_equals_value_keeps_further_equals_signs() {
    match command(&["pdf", "a.pdf", "--title=a=b"]) {
        Command::Pdf { title, .. } => assert_eq!(title.as_deref(), Some("a=b")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_value_that_looks_like_a_flag_is_taken_as_the_value() {
    match command(&["pdf", "a.pdf", "--title", "--help"]) {
        Command::Pdf { title, .. } => assert_eq!(title.as_deref(), Some("--help")),
        other => panic!("{other:?}"),
    }
    match command(&["pdf", "a.pdf", "--title", "--local"]) {
        Command::Pdf { title, .. } => assert_eq!(title.as_deref(), Some("--local")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn double_dash_ends_options() {
    match command(&["pdf", "--", "-odd.pdf"]) {
        Command::Pdf { file, .. } => assert_eq!(file, "-odd.pdf"),
        other => panic!("{other:?}"),
    }
    match command(&["pdf", "--", "--help"]) {
        Command::Pdf { file, .. } => assert_eq!(file, "--help"),
        other => panic!("{other:?}"),
    }
    let inv = parse(&["pdf", "--", "--local"]).unwrap();
    assert_eq!(inv.global, local());
    assert!(matches!(inv.command, Command::Pdf { ref file, .. } if file == "--local"));
}

#[test]
fn help_and_version() {
    assert_eq!(command(&[]), Command::Help(None));
    assert_eq!(command(&["--help"]), Command::Help(None));
    assert_eq!(command(&["-h"]), Command::Help(None));
    assert_eq!(command(&["help"]), Command::Help(None));
    assert_eq!(command(&["--version"]), Command::Version);
    assert_eq!(command(&["--local", "--version"]), Command::Version);
    assert_eq!(
        command(&["pdf", "--help"]),
        Command::Help(Some("pdf".into()))
    );
    assert_eq!(command(&["scan", "-h"]), Command::Help(Some("scan".into())));
    // A standalone help token wins over a missing target and later errors.
    assert_eq!(
        command(&["document", "--help", "--nope"]),
        Command::Help(Some("document".into()))
    );
}

#[test]
fn global_options_before_and_after_the_command() {
    let inv = parse(&[
        "--server",
        "https://core.example.com",
        "pdf",
        "a.pdf",
        "--image=img:1",
    ])
    .unwrap();
    assert_eq!(
        inv.global,
        Global {
            server: Some("https://core.example.com".into()),
            local: false,
            image: Some("img:1".into()),
        }
    );
    assert!(matches!(inv.command, Command::Pdf { .. }));

    let inv = parse(&["scan", "https://example.com/", "--local"]).unwrap();
    assert!(inv.global.local);
    let inv = parse(&["--server=http://127.0.0.1:8080", "--help"]).unwrap();
    assert_eq!(inv.command, Command::Help(None));
    assert_eq!(inv.global.server.as_deref(), Some("http://127.0.0.1:8080"));
}

#[test]
fn server_and_local_together_is_a_usage_error() {
    let e = error(&[
        "--server",
        "https://core.example.com",
        "pdf",
        "a.pdf",
        "--local",
    ]);
    assert_eq!(e.message, "--server and --local cannot be used together");
    assert_eq!(e.command.as_deref(), Some("pdf"));
    let e = error(&[
        "--local",
        "--server=https://core.example.com",
        "scan",
        "https://example.com/",
    ]);
    assert_eq!(e.message, "--server and --local cannot be used together");
}

#[test]
fn core_parse_errors() {
    let cases: &[(&[&str], &str, Option<&str>)] = &[
        (
            &["pdf", "a.pdf", "--x"],
            "unknown option for pdf: --x",
            Some("pdf"),
        ),
        (
            &["pdf", "a.pdf", "--x=1"],
            "unknown option for pdf: --x",
            Some("pdf"),
        ),
        (
            &["scan", "u", "--fix"],
            "unknown option for scan: --fix",
            Some("scan"),
        ),
        (
            &["html", "a.html", "--fix"],
            "unknown option for html: --fix",
            Some("html"),
        ),
        (
            &["pdf", "a.pdf", "--out"],
            "--out needs a value",
            Some("pdf"),
        ),
        (
            &["pdf", "a.pdf", "--language"],
            "--language needs a value",
            Some("pdf"),
        ),
        (
            &["pdf", "a.pdf", "--out="],
            "--out needs a non-empty directory",
            Some("pdf"),
        ),
        (
            &["pdf", "a.pdf", "--out", " "],
            "--out needs a non-empty directory",
            Some("pdf"),
        ),
        (
            &["pdf", "a.pdf", "--json=yes"],
            "--json takes no value",
            Some("pdf"),
        ),
        (&["pdf"], "an input file is required", Some("pdf")),
        (
            &["html", "--json"],
            "an input file is required",
            Some("html"),
        ),
        (&["document"], "an input file is required", Some("document")),
        (&["scan"], "a URL is required", Some("scan")),
        (
            &["pdf", "a.pdf", "b.pdf"],
            "unexpected extra argument: b.pdf",
            Some("pdf"),
        ),
        (
            &["pdf", "a.pdf", "--", "b.pdf"],
            "unexpected extra argument: b.pdf",
            Some("pdf"),
        ),
        (&["frobnicate"], "unknown command: frobnicate", None),
        (&["serve"], "unknown command: serve", None),
        (&["--nope"], "unknown command: --nope", None),
        (
            &["--local", "--nope", "pdf"],
            "unknown command: --nope",
            None,
        ),
        (&["--server"], "--server needs a value", None),
        (
            &["pdf", "a.pdf", "--image"],
            "--image needs a value",
            Some("pdf"),
        ),
        (&["--server="], "--server needs a non-empty URL", None),
        (
            &["pdf", "a.pdf", "--image= "],
            "--image needs a non-empty reference",
            Some("pdf"),
        ),
        (
            &["pdf", "a.pdf", "--local=1"],
            "--local takes no value",
            Some("pdf"),
        ),
        (
            &["pdf", "a.pdf", "--version"],
            "unknown option for pdf: --version",
            Some("pdf"),
        ),
    ];
    for (line, message, cmd) in cases {
        let e = error(line);
        assert_eq!(&e.message, message, "for {line:?}");
        assert_eq!(e.command.as_deref(), *cmd, "for {line:?}");
    }
}

#[test]
fn usage_error_display_is_the_message_and_usage_names_the_command() {
    let e = error(&["pdf"]);
    assert_eq!(e.to_string(), "an input file is required");
    assert_eq!(e.usage(), args::usage_of("pdf"));
    assert_eq!(error(&["nope"]).usage(), args::GENERAL_USAGE);
}

#[test]
fn usage_texts_match_core() {
    assert_eq!(
        args::usage_of("pdf"),
        "usage: koldera pdf <file> [--fix] [--language <tag>] [--title <text>] [--out <dir>] [--json]"
    );
    assert_eq!(
        args::usage_of("html"),
        "usage: koldera html <file> [--analyze-only] [--content-selector <css>] [--out <dir>] [--json]"
    );
    assert_eq!(
        args::usage_of("scan"),
        "usage: koldera scan <url> [--out <dir>] [--json]"
    );
    assert_eq!(
        args::usage_of("document"),
        "usage: koldera document <file.json> [--out <dir>] [--json]"
    );
    assert_eq!(
        args::GENERAL_USAGE,
        "usage: koldera pdf <file> [--fix] [--language <tag>] [--title <text>] [--out <dir>] [--json]\n\
         \x20      koldera html <file> [--analyze-only] [--content-selector <css>] [--out <dir>] [--json]\n\
         \x20      koldera scan <url> [--out <dir>] [--json]\n\
         \x20      koldera document <file.json> [--out <dir>] [--json]\n\
         \x20      koldera --help | <command> --help | --version"
    );
    assert_eq!(args::usage_of("other"), args::GENERAL_USAGE);
}

#[test]
fn command_help_texts_match_core() {
    assert_eq!(
        args::help_of("pdf"),
        format!(
            "{}\n\n{}",
            args::usage_of("pdf"),
            "Checks a PDF against the PDF/UA-1 machine rules. With --fix a repaired copy is written to --out\n\
             as <name>.remediated.pdf; the input is never modified.\n\
             \x20 --fix              write a repaired copy (default: analyze only)\n\
             \x20 --language <tag>   BCP 47 language to use when the document has none\n\
             \x20 --title <text>     document title to use (1-500 printable characters)\n\
             \x20 --out <dir>        where artifacts are written (default: current directory)\n\
             \x20 --json             print {job, findings, actions, artifacts} as JSON; nothing else on stdout"
        )
    );
    assert_eq!(
        args::help_of("html"),
        format!(
            "{}\n\n{}",
            args::usage_of("html"),
            "Renders one static HTML file to a tagged PDF and checks it. Nothing is fetched from the network.\n\
             \x20 --analyze-only          report only (default: also apply the safe fixes)\n\
             \x20 --content-selector <c>  render only the element matching this CSS selector\n\
             \x20 --out <dir>             where artifacts are written (default: current directory)\n\
             \x20 --json                  print {job, findings, actions, artifacts} as JSON; nothing else on stdout"
        )
    );
    assert_eq!(
        args::help_of("scan"),
        format!(
            "{}\n\n{}",
            args::usage_of("scan"),
            "Scans a web page with automated accessibility checks (a subset of WCAG; not a conformance claim).\n\
             \x20 --out <dir>   where the report is written (default: current directory)\n\
             \x20 --json        print {job, findings, actions, artifacts} as JSON; nothing else on stdout"
        )
    );
    assert_eq!(
        args::help_of("document"),
        format!(
            "{}\n\n{}",
            args::usage_of("document"),
            "Renders a structured document (JSON) to a tagged PDF and checks it. An image names a \"file\" path,\n\
             resolved relative to the JSON file; paths outside that directory are refused.\n\
             \x20 --out <dir>   where artifacts are written (default: current directory)\n\
             \x20 --json        print {job, findings, actions, artifacts} as JSON; nothing else on stdout"
        )
    );
}

#[test]
fn general_help_lists_commands_global_options_environment_and_exit_codes() {
    let help = args::help_of("");
    assert!(help.starts_with(args::GENERAL_USAGE));
    for needle in [
        "Commands:",
        "  pdf        check or repair a PDF",
        "  html       render HTML to a tagged PDF",
        "  scan       scan a web page",
        "  document   render a structured document to a tagged PDF",
        "Global options (before or after the command):",
        "--server <url>",
        "--local",
        "--image <ref>",
        "KOLDERA_URL",
        "KOLDERA_TOKEN",
        "KOLDERA_IMAGE",
        "Exit codes: 0 machine check passed or declaration_withheld (only the PDF/UA identifier\n\
         is missing, pending human review), 2 completed with failed machine rules,\n\
         3 job failed, unsupported or no machine verdict, 64 usage error, 130 interrupted.",
    ] {
        assert!(help.contains(needle), "missing {needle:?} in\n{help}");
    }
    assert!(!help.contains("  serve ") && !help.contains("[serve]"));
}

#[test]
fn version_line() {
    assert_eq!(
        args::version_line(),
        format!("koldera-cli {}", env!("CARGO_PKG_VERSION"))
    );
}
