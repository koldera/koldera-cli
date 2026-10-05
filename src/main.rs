use std::process::ExitCode;

use koldera_cli::args::{self, Command};
use koldera_cli::exit;

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
        // Running jobs (remote and local) comes with the next parts of the client.
        _ => {
            eprintln!("koldera: running jobs is not available in this build yet");
            code(exit::JOB_FAILED)
        }
    }
}

fn code(c: i32) -> ExitCode {
    ExitCode::from(c as u8)
}
