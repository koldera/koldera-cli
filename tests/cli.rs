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
