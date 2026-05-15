mod cli;
mod models;
mod parser;
mod ranking;
mod reporter;
mod simulate;
mod terminal;
mod utils;

use std::io::{self, IsTerminal};

use crate::cli::parse_args;
use crate::models::{Algorithm, AppError};
use crate::parser::{parse_inputs, read_matches_file};
use crate::reporter::Reporter;
use crate::simulate::{SimulationRunner, auto::AutoSimulator, dfs::DfsSimulator, dp::DpSimulator};
use crate::terminal::{Colors, Terminal};

fn run() -> Result<(), AppError> {
    let term = Terminal::new(io::stdout().is_terminal());
    let cli = parse_args(&term)?;
    let matches_input = read_matches_file(&cli.file_path)?;
    let parsed = parse_inputs(&matches_input)?;

    if cli.calibrate_dp {
        let dp = DpSimulator::new(&parsed, cli.allow_no_results);
        dp.run_calibration(parsed.initial_state, &Terminal::new(false));
        return Ok(());
    }

    let reporter = Reporter::new(&parsed, &term, cli.allow_no_results);
    let runner = match cli.algorithm {
        Algorithm::Dfs => SimulationRunner::Dfs(DfsSimulator::new(&parsed, cli.allow_no_results)),
        Algorithm::Dp => SimulationRunner::Dp(DpSimulator::new(&parsed, cli.allow_no_results)),
        Algorithm::Auto => {
            SimulationRunner::Auto(AutoSimulator::new(&parsed, cli.allow_no_results))
        }
    };
    runner.run_simulation(&parsed.initial_state, &reporter, &term)?;
    Ok(())
}

fn main() {
    let colors = Colors::new(io::stderr().is_terminal());
    if let Err(e) = run() {
        eprintln!("{}Error:{} {}", colors.yellow, colors.reset, e);
        std::process::exit(1);
    }
}
