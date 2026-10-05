//! Hand-written argument parsing and help texts, mirroring core's command line, plus the global options
//! `--server`, `--local` and `--image` (accepted before or after the command).

use std::fmt;

/// The core image local mode starts when neither `--image` nor `KOLDERA_IMAGE` names one.
pub const DEFAULT_IMAGE: &str = "ghcr.io/koldera/core:0.1.0";

pub const COMMANDS: [&str; 4] = ["pdf", "html", "scan", "document"];

pub const GENERAL_USAGE: &str = "\
usage: koldera pdf <file> [--fix] [--language <tag>] [--title <text>] [--out <dir>] [--json]
       koldera html <file> [--analyze-only] [--content-selector <css>] [--out <dir>] [--json]
       koldera scan <url> [--out <dir>] [--json]
       koldera document <file.json> [--out <dir>] [--json]
       koldera --help | <command> --help | --version";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Global {
    pub server: Option<String>,
    pub local: bool,
    pub image: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// General help (`None`) or the help of one command.
    Help(Option<String>),
    Version,
    Pdf {
        file: String,
        fix: bool,
        language: Option<String>,
        title: Option<String>,
        out: Option<String>,
        json: bool,
    },
    Html {
        file: String,
        analyze_only: bool,
        content_selector: Option<String>,
        out: Option<String>,
        json: bool,
    },
    Scan {
        url: String,
        out: Option<String>,
        json: bool,
    },
    Document {
        file: String,
        out: Option<String>,
        json: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub global: Global,
    pub command: Command,
}

/// What is wrong with the command line, and which command's usage to print after it (`None`: the general usage).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError {
    pub message: String,
    pub command: Option<String>,
}

impl UsageError {
    fn new(message: impl Into<String>, command: Option<&str>) -> Self {
        Self {
            message: message.into(),
            command: command.map(str::to_owned),
        }
    }

    pub fn usage(&self) -> &'static str {
        usage_of(self.command.as_deref().unwrap_or(""))
    }
}

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for UsageError {}

pub fn version_line() -> String {
    format!("koldera-cli {}", env!("CARGO_PKG_VERSION"))
}

pub fn usage_of(command: &str) -> &'static str {
    match command {
        "pdf" => {
            "usage: koldera pdf <file> [--fix] [--language <tag>] [--title <text>] [--out <dir>] [--json]"
        }
        "html" => {
            "usage: koldera html <file> [--analyze-only] [--content-selector <css>] [--out <dir>] [--json]"
        }
        "scan" => "usage: koldera scan <url> [--out <dir>] [--json]",
        "document" => "usage: koldera document <file.json> [--out <dir>] [--json]",
        _ => GENERAL_USAGE,
    }
}

const PDF_HELP: &str = "\
Checks a PDF against the PDF/UA-1 machine rules. With --fix a repaired copy is written to --out
as <name>.remediated.pdf; the input is never modified.
  --fix              write a repaired copy (default: analyze only)
  --language <tag>   BCP 47 language to use when the document has none
  --title <text>     document title to use (1-500 printable characters)
  --out <dir>        where artifacts are written (default: current directory)
  --json             print {job, findings, actions, artifacts} as JSON; nothing else on stdout";

const HTML_HELP: &str = "\
Renders one static HTML file to a tagged PDF and checks it. Nothing is fetched from the network.
  --analyze-only          report only (default: also apply the safe fixes)
  --content-selector <c>  render only the element matching this CSS selector
  --out <dir>             where artifacts are written (default: current directory)
  --json                  print {job, findings, actions, artifacts} as JSON; nothing else on stdout";

const SCAN_HELP: &str = "\
Scans a web page with automated accessibility checks (a subset of WCAG; not a conformance claim).
  --out <dir>   where the report is written (default: current directory)
  --json        print {job, findings, actions, artifacts} as JSON; nothing else on stdout";

const DOCUMENT_HELP: &str = "\
Renders a structured document (JSON) to a tagged PDF and checks it. An image names a \"file\" path,
resolved relative to the JSON file; paths outside that directory are refused.
  --out <dir>   where artifacts are written (default: current directory)
  --json        print {job, findings, actions, artifacts} as JSON; nothing else on stdout";

const GENERAL_HELP: &str = "\
Commands:
  pdf        check or repair a PDF
  html       render HTML to a tagged PDF
  scan       scan a web page
  document   render a structured document to a tagged PDF

Global options (before or after the command):
  --server <url>   run the job on this koldera-core server (default: KOLDERA_URL)
  --local          run the job in the koldera-core Docker image, even when KOLDERA_URL is set
  --image <ref>    Docker image for local mode (default: KOLDERA_IMAGE, else ghcr.io/koldera/core:0.1.0)

Environment:
  KOLDERA_URL      server to use; without it (or with --local) jobs run locally in Docker
  KOLDERA_TOKEN    bearer token for the server (only from the environment, never a flag)
  KOLDERA_IMAGE    Docker image for local mode

Exit codes: 0 machine check passed, 2 completed with failed machine rules,
3 job failed, unsupported or no machine verdict, 64 usage error, 130 interrupted.";

/// The help text of a command, or the general help for any other name (no trailing newline).
pub fn help_of(command: &str) -> String {
    let body = match command {
        "pdf" => PDF_HELP,
        "html" => HTML_HELP,
        "scan" => SCAN_HELP,
        "document" => DOCUMENT_HELP,
        _ => GENERAL_HELP,
    };
    format!("{}\n\n{body}", usage_of(command))
}

struct Flag {
    name: &'static str,
    takes_value: bool,
}

const fn flag(name: &'static str, takes_value: bool) -> Flag {
    Flag { name, takes_value }
}

const GLOBAL_FLAGS: [Flag; 3] = [
    flag("--server", true),
    flag("--local", false),
    flag("--image", true),
];
const PDF_FLAGS: [Flag; 5] = [
    flag("--fix", false),
    flag("--language", true),
    flag("--title", true),
    flag("--out", true),
    flag("--json", false),
];
const HTML_FLAGS: [Flag; 4] = [
    flag("--analyze-only", false),
    flag("--content-selector", true),
    flag("--out", true),
    flag("--json", false),
];
const OTHER_FLAGS: [Flag; 2] = [flag("--out", true), flag("--json", false)];

fn flags_of(command: &str) -> &'static [Flag] {
    match command {
        "pdf" => &PDF_FLAGS,
        "html" => &HTML_FLAGS,
        _ => &OTHER_FLAGS,
    }
}

#[derive(Default)]
struct Options {
    target: Option<String>,
    fix: bool,
    analyze_only: bool,
    json: bool,
    language: Option<String>,
    title: Option<String>,
    selector: Option<String>,
    out: Option<String>,
}

/// Parses the arguments after the program name. The environment is not read here.
pub fn parse(argv: &[String]) -> Result<Invocation, UsageError> {
    let mut global = Global::default();
    let mut i = 0;

    // Global options may come before the command.
    while i < argv.len() {
        let (name, inline) = split(&argv[i]);
        let Some(flag) = GLOBAL_FLAGS.iter().find(|f| f.name == name) else {
            break;
        };
        let value = take_value(flag, inline, argv, &mut i, None)?;
        set_global(&mut global, name, value, None)?;
        i += 1;
    }

    let Some(name) = argv.get(i).map(String::as_str) else {
        return Ok(Invocation {
            global,
            command: Command::Help(None),
        });
    };
    match name {
        "--help" | "-h" | "help" => {
            return Ok(Invocation {
                global,
                command: Command::Help(None),
            });
        }
        "--version" => {
            return Ok(Invocation {
                global,
                command: Command::Version,
            });
        }
        _ if !COMMANDS.contains(&name) => {
            return Err(UsageError::new(format!("unknown command: {name}"), None));
        }
        _ => {}
    }

    let cmd = Some(name);
    let flags = flags_of(name);
    let mut o = Options::default();
    let mut options_ended = false;
    i += 1;
    while i < argv.len() {
        let arg = &argv[i];
        // A standalone help token asks for help; one consumed as a flag value or after "--" does not.
        if !options_ended && (arg == "--help" || arg == "-h") {
            return Ok(Invocation {
                global,
                command: Command::Help(Some(name.to_owned())),
            });
        }
        if !options_ended && arg == "--" {
            options_ended = true;
            i += 1;
            continue;
        }
        if options_ended || !arg.starts_with('-') {
            if o.target.is_some() {
                return Err(UsageError::new(
                    format!("unexpected extra argument: {arg}"),
                    cmd,
                ));
            }
            o.target = Some(arg.clone());
            i += 1;
            continue;
        }
        option(&mut o, &mut global, flags, argv, &mut i, name)?;
        i += 1;
    }

    if global.server.is_some() && global.local {
        return Err(UsageError::new(
            "--server and --local cannot be used together",
            cmd,
        ));
    }
    let Some(target) = o.target else {
        let message = if name == "scan" {
            "a URL is required"
        } else {
            "an input file is required"
        };
        return Err(UsageError::new(message, cmd));
    };
    let command = match name {
        "pdf" => Command::Pdf {
            file: target,
            fix: o.fix,
            language: o.language,
            title: o.title,
            out: o.out,
            json: o.json,
        },
        "html" => Command::Html {
            file: target,
            analyze_only: o.analyze_only,
            content_selector: o.selector,
            out: o.out,
            json: o.json,
        },
        "scan" => Command::Scan {
            url: target,
            out: o.out,
            json: o.json,
        },
        _ => Command::Document {
            file: target,
            out: o.out,
            json: o.json,
        },
    };
    Ok(Invocation { global, command })
}

/// One option of a command (its own flag or a global one) at `argv[*i]`; advances `i` past a separate value.
fn option(
    o: &mut Options,
    global: &mut Global,
    flags: &[Flag],
    argv: &[String],
    i: &mut usize,
    name: &str,
) -> Result<(), UsageError> {
    let cmd = Some(name);
    let (flag_name, inline) = split(&argv[*i]);
    if let Some(flag) = GLOBAL_FLAGS.iter().find(|f| f.name == flag_name) {
        let value = take_value(flag, inline, argv, i, cmd)?;
        set_global(global, flag_name, value, cmd)?;
        return Ok(());
    }
    let Some(flag) = flags.iter().find(|f| f.name == flag_name) else {
        return Err(UsageError::new(
            format!("unknown option for {name}: {flag_name}"),
            cmd,
        ));
    };
    let value = take_value(flag, inline, argv, i, cmd)?;
    match flag_name {
        "--fix" => o.fix = true,
        "--analyze-only" => o.analyze_only = true,
        "--json" => o.json = true,
        "--language" => o.language = value,
        "--title" => o.title = value,
        "--content-selector" => o.selector = value,
        "--out" => {
            o.out = Some(non_empty(value, "--out needs a non-empty directory", cmd)?);
        }
        _ => {}
    }
    Ok(())
}

/// `--name=value` into its name and inline value (an `=` at position 0 is not a separator, as in core).
fn split(arg: &str) -> (&str, Option<&str>) {
    match arg.find('=') {
        Some(eq) if eq > 0 => (&arg[..eq], Some(&arg[eq + 1..])),
        _ => (arg, None),
    }
}

fn take_value(
    flag: &Flag,
    inline: Option<&str>,
    argv: &[String],
    i: &mut usize,
    cmd: Option<&str>,
) -> Result<Option<String>, UsageError> {
    if !flag.takes_value {
        return match inline {
            Some(_) => Err(UsageError::new(
                format!("{} takes no value", flag.name),
                cmd,
            )),
            None => Ok(None),
        };
    }
    if let Some(v) = inline {
        return Ok(Some(v.to_owned()));
    }
    if *i + 1 < argv.len() {
        *i += 1;
        return Ok(Some(argv[*i].clone()));
    }
    Err(UsageError::new(format!("{} needs a value", flag.name), cmd))
}

fn set_global(
    global: &mut Global,
    name: &str,
    value: Option<String>,
    cmd: Option<&str>,
) -> Result<(), UsageError> {
    match name {
        "--server" => {
            global.server = Some(non_empty(value, "--server needs a non-empty URL", cmd)?);
        }
        "--image" => {
            global.image = Some(non_empty(
                value,
                "--image needs a non-empty reference",
                cmd,
            )?);
        }
        _ => global.local = true,
    }
    Ok(())
}

fn non_empty(
    value: Option<String>,
    message: &str,
    cmd: Option<&str>,
) -> Result<String, UsageError> {
    match value {
        Some(v) if !v.trim().is_empty() => Ok(v),
        _ => Err(UsageError::new(message, cmd)),
    }
}
