//! The `kipi` binary: clap commands and output formatting over `kipi-core`.

mod commands;
mod output;

use clap::Parser;

/// Give LLM coding agents a scoped map of your local repos and shared skills.
#[derive(Debug, Parser)]
#[command(name = "kipi", version, about, long_about = None)]
struct Cli {}

fn main() {
    let Cli {} = Cli::parse();
}
