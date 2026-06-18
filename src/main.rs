use std::io::{self, IsTerminal};

use ipl_playoff_calculator::{
    Algorithm, AppError, AutoSimulator, Colors, DfsSimulator, DpSimulator, Reporter,
    SimulationRunner, Terminal, calibrate_dp, parse_args, parse_inputs, read_matches_file,
};

fn run() -> Result<(), AppError> {
    let term = Terminal::new(io::stdout().is_terminal());
    let cli = match parse_args(std::env::args(), &term)? {
        Some(c) => c,
        None => return Ok(()),
    };
    let matches_input = read_matches_file(&cli.file_path)?;
    let parsed = parse_inputs(&matches_input)?;

    if cli.calibrate_dp {
        let dp = DpSimulator::new(&parsed, cli.allow_no_results);
        calibrate_dp(&dp, parsed.initial_state);
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
