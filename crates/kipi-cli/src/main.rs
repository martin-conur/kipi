//! The `kipi` binary: clap commands and output formatting over `kipi-core`.

mod cli;
mod commands;
mod complete;
mod error;
mod output;

use std::io::{self, Write};
use std::process::ExitCode;

use clap::Parser;

use cli::Cli;
use error::{EXIT_OK, EXIT_USER, exit};
use output::Output;
use output::color::{Painter, no_color_env};

fn main() -> ExitCode {
    // A completion request (`KIPI_COMPLETE=<shell>`) is answered here and exits.
    complete::handle_request();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            // `--help` and `--version` are not errors. Usage errors exit 1 (a user error),
            // not clap's default 2, which kipi reserves for doctor findings.
            let code = if e.use_stderr() { EXIT_USER } else { EXIT_OK };
            let _ = e.print();
            return exit(code);
        }
    };
    let no_color = cli.global.no_color || no_color_env(std::env::var_os("NO_COLOR"));
    let out = Output {
        json: cli.global.json,
        paint: Painter::detect(&io::stdout(), no_color),
        paint_err: Painter::detect(&io::stderr(), no_color),
        verbosity: if cli.global.quiet {
            -1
        } else {
            i8::try_from(cli.global.verbose).unwrap_or(i8::MAX)
        },
        home: kipi_core::config::Env::from_process().ok().map(|e| e.home),
    };
    match commands::run(&cli.command, &cli.global, &out) {
        Ok(()) => exit(EXIT_OK),
        Err(failure) => {
            let _ = io::stderr().write_all(failure.render(&out.paint_err).as_bytes());
            exit(failure.exit_code())
        }
    }
}
