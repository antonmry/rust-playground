mod cli;
mod energy;
mod error;
mod output;
mod runner;
mod util;

use crate::cli::Cli;
use crate::error::EnergyError;
use crate::runner::run_command;
use clap::Parser;

fn main() {
    if let Err(err) = real_main() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<(), EnergyError> {
    let cli = Cli::parse();
    let result = run_command(&cli)?;
    output::print_result(&cli.output, &result)?;

    match result.exit_status.code() {
        Some(code) => std::process::exit(code),
        None => {
            if result.exit_status.success() {
                Ok(())
            } else {
                std::process::exit(1)
            }
        }
    }
}
