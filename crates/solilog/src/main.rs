mod cli;
mod ui;

use std::process::ExitCode;

use clap::Parser as _;

fn main() -> ExitCode {
    let mut cli = cli::Cli::parse();
    let json = cli.json;
    let result = match cli.command.take() {
        Some(command) => cli::run(cli, command),
        None => solilog_core::Db::open_default().and_then(ui::run),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            cli::report_error(json, &err);
            ExitCode::FAILURE
        }
    }
}
