use std::process::ExitCode;

use koldera_cli::api::Api;
use koldera_cli::args::{self, Command};
use koldera_cli::{exit, remote};

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let invocation = match args::parse(&argv) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("koldera: {e}");
            eprintln!("{}", e.usage());
            return code(exit::USAGE);
        }
    };
    match invocation.command {
        Command::Help(command) => {
            println!("{}", args::help_of(command.as_deref().unwrap_or("")));
            code(exit::OK)
        }
        Command::Version => {
            println!("{}", args::version_line());
            code(exit::OK)
        }
        command => {
            let server = invocation
                .global
                .server
                .clone()
                .or_else(|| std::env::var("KOLDERA_URL").ok())
                .filter(|s| !s.is_empty());
            if invocation.global.local || invocation.global.image.is_some() {
                eprintln!("koldera: local Docker mode is not implemented; use --server or KOLDERA_URL");
                return code(exit::USAGE);
            }
            match server {
                Some(server) => code(run_remote(&server, &command)),
                None => {
                    eprintln!("koldera: set --server or KOLDERA_URL to connect to koldera-core");
                    code(exit::USAGE)
                }
            }
        }
    }
}

fn run_remote(server: &str, command: &Command) -> i32 {
    let token = std::env::var("KOLDERA_TOKEN").ok();
    let api = match Api::new(server, token) {
        Ok(api) => api,
        Err(e) => {
            eprintln!("koldera: {e}");
            eprintln!("{}", e.usage());
            return exit::USAGE;
        }
    };
    if let Some(warning) = remote::clear_text_warning(api.base_url()) {
        eprintln!("{warning}");
    }
    remote::run(
        &api,
        command,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    )
}

fn code(c: i32) -> ExitCode {
    ExitCode::from(c as u8)
}
