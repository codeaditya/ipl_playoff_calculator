use crate::models::{Algorithm, AppError};
use crate::terminal::Terminal;

#[derive(Debug)]
pub struct CliArgs {
    pub file_path: String,
    pub algorithm: Algorithm,
    pub allow_no_results: bool,
    pub calibrate_dp: bool,
}

pub fn parse_args<I, S>(args: I, term: &Terminal) -> Result<Option<CliArgs>, AppError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = args.into_iter();
    let program_name = args
        .next()
        .map(|s| s.as_ref().to_string())
        .unwrap_or_else(|| "ipl-playoff-calculator".to_string());

    let mut file_path: Option<String> = None;
    let mut algorithm = Algorithm::Auto;
    let mut allow_no_results = false;
    let mut calibrate_dp = false;

    for arg in args {
        let arg = arg.as_ref();
        match arg {
            "--help" | "-h" => {
                print_usage(&program_name, term);
                return Ok(None);
            }
            "--allow-no-results" => {
                allow_no_results = true;
            }
            "--algo=dfs" | "--algo=DFS" => {
                algorithm = Algorithm::Dfs;
            }
            "--algo=dp" | "--algo=DP" => {
                algorithm = Algorithm::Dp;
            }
            "--algo=auto" | "--algo=AUTO" => {
                algorithm = Algorithm::Auto;
            }
            "--calibrate-dp" => calibrate_dp = true,
            _ if arg.starts_with('-') => {
                return Err(AppError::Parse(format!("Unknown flag: {}", arg)));
            }
            _ => {
                if file_path.is_some() {
                    return Err(AppError::Parse(
                        "Expected exactly one matches file path".to_string(),
                    ));
                }
                file_path = Some(arg.to_string());
            }
        }
    }

    let file_path = file_path.ok_or_else(|| {
        AppError::Parse("Missing matches file path. Use --help for usage.".to_string())
    })?;

    Ok(Some(CliArgs {
        file_path,
        algorithm,
        allow_no_results,
        calibrate_dp,
    }))
}

fn print_usage(program_name: &str, term: &Terminal) {
    eprintln!(
        "{bold}{cyan}IPL Playoff Calculator{reset}\n",
        bold = term.colors.bold,
        cyan = term.colors.cyan,
        reset = term.colors.reset
    );
    eprintln!(
        "{bold}{yellow}Usage:{reset} {magenta}{prog}{reset} {cyan}[--allow-no-results]{reset} {cyan}[--algo=auto|dfs|dp]{reset} {green}<matches-file>{reset}",
        bold = term.colors.bold,
        yellow = term.colors.yellow,
        reset = term.colors.reset,
        magenta = term.colors.magenta,
        prog = program_name,
        cyan = term.colors.cyan,
        green = term.colors.green,
    );
    eprintln!(
        "\n{bold}{yellow}Arguments:{reset}",
        bold = term.colors.bold,
        yellow = term.colors.yellow,
        reset = term.colors.reset
    );
    eprintln!(
        "  {green}<matches-file>{reset}       Path to the text file containing the schedule.",
        green = term.colors.green,
        reset = term.colors.reset
    );
    eprintln!(
        "  {cyan}--allow-no-results{reset}   (Optional) Include ties/washouts (1 pt each) in future outcomes.",
        cyan = term.colors.cyan,
        reset = term.colors.reset
    );
    eprintln!(
        "  {cyan}--algo=dfs{reset}           DFS simulation: low RAM (~<5 MB), slower for large match counts.",
        cyan = term.colors.cyan,
        reset = term.colors.reset
    );
    eprintln!(
        "  {cyan}--algo=dp{reset}            DP simulation: faster for large match counts, but uses significantly more RAM.",
        cyan = term.colors.cyan,
        reset = term.colors.reset
    );
    eprintln!(
        "  {cyan}--algo=auto{reset}          (Default) Dynamically scales between pure DP and Hybrid DFS-DP based on available system RAM.",
        cyan = term.colors.cyan,
        reset = term.colors.reset
    );
    eprintln!(
        "\n{bold}{yellow}Dev Only Arguments:{reset}",
        bold = term.colors.bold,
        yellow = term.colors.yellow,
        reset = term.colors.reset
    );
    eprintln!(
        "  {cyan}--calibrate-dp{reset}       Use to calibrate the RAM usage and compute time for DP simulation",
        cyan = term.colors.cyan,
        reset = term.colors.reset
    );
    eprintln!(
        "\n{bold}{yellow}Matches File Format:{reset}",
        bold = term.colors.bold,
        yellow = term.colors.yellow,
        reset = term.colors.reset
    );
    eprintln!("  - One match per line. Lines starting with '#' are ignored.");
    eprintln!(
        "  - {magenta}Upcoming:{reset}  Team A vs Team B",
        magenta = term.colors.magenta,
        reset = term.colors.reset,
    );
    eprintln!(
        "  - {magenta}Completed:{reset} Team A vs Team B : Winner",
        magenta = term.colors.magenta,
        reset = term.colors.reset,
    );
    eprintln!(
        "  - {magenta}No Result:{reset} Team A vs Team B : NR",
        magenta = term.colors.magenta,
        reset = term.colors.reset,
    );
    eprintln!(
        "\n{bold}{yellow}Example:{reset}",
        bold = term.colors.bold,
        yellow = term.colors.yellow,
        reset = term.colors.reset
    );
    eprintln!("  CSK vs RCB : CSK");
    eprintln!("  MI vs DC\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_term() -> Terminal {
        Terminal::new(false)
    }

    #[test]
    fn test_minimal_valid() {
        let cli = parse_args(["prog", "matches.txt"], &test_term())
            .unwrap()
            .unwrap();
        assert_eq!(cli.file_path, "matches.txt");
        assert_eq!(cli.algorithm, Algorithm::Auto);
        assert!(!cli.allow_no_results);
        assert!(!cli.calibrate_dp);
    }

    #[test]
    fn test_algo_dfs() {
        let cli = parse_args(["prog", "--algo=dfs", "matches.txt"], &test_term())
            .unwrap()
            .unwrap();
        assert_eq!(cli.algorithm, Algorithm::Dfs);
    }

    #[test]
    fn test_algo_dfs_uppercase() {
        let cli = parse_args(["prog", "--algo=DFS", "matches.txt"], &test_term())
            .unwrap()
            .unwrap();
        assert_eq!(cli.algorithm, Algorithm::Dfs);
    }

    #[test]
    fn test_algo_dp() {
        let cli = parse_args(["prog", "--algo=dp", "matches.txt"], &test_term())
            .unwrap()
            .unwrap();
        assert_eq!(cli.algorithm, Algorithm::Dp);
    }

    #[test]
    fn test_algo_dp_uppercase() {
        let cli = parse_args(["prog", "--algo=DP", "matches.txt"], &test_term())
            .unwrap()
            .unwrap();
        assert_eq!(cli.algorithm, Algorithm::Dp);
    }

    #[test]
    fn test_algo_auto() {
        let cli = parse_args(["prog", "--algo=auto", "matches.txt"], &test_term())
            .unwrap()
            .unwrap();
        assert_eq!(cli.algorithm, Algorithm::Auto);
    }

    #[test]
    fn test_algo_auto_uppercase() {
        let cli = parse_args(["prog", "--algo=AUTO", "matches.txt"], &test_term())
            .unwrap()
            .unwrap();
        assert_eq!(cli.algorithm, Algorithm::Auto);
    }

    #[test]
    fn test_allow_no_results() {
        let cli = parse_args(["prog", "--allow-no-results", "matches.txt"], &test_term())
            .unwrap()
            .unwrap();
        assert!(cli.allow_no_results);
    }

    #[test]
    fn test_calibrate_dp() {
        let cli = parse_args(["prog", "--calibrate-dp", "matches.txt"], &test_term())
            .unwrap()
            .unwrap();
        assert!(cli.calibrate_dp);
    }

    #[test]
    fn test_combined_flags() {
        let cli = parse_args(
            ["prog", "--algo=dfs", "--allow-no-results", "matches.txt"],
            &test_term(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(cli.algorithm, Algorithm::Dfs);
        assert!(cli.allow_no_results);
    }

    #[test]
    fn test_unknown_flag() {
        let err = parse_args(["prog", "--bogus", "matches.txt"], &test_term())
            .unwrap_err()
            .to_string();
        assert!(err.contains("Unknown flag"));
    }

    #[test]
    fn test_missing_file_path() {
        let err = parse_args(["prog"], &test_term()).unwrap_err().to_string();
        assert!(err.contains("Missing matches file path"));
    }

    #[test]
    fn test_duplicate_file_path() {
        let err = parse_args(["prog", "a.txt", "b.txt"], &test_term())
            .unwrap_err()
            .to_string();
        assert!(err.contains("exactly one"));
    }

    #[test]
    fn test_help_flag() {
        let result = parse_args(["prog", "--help", "matches.txt"], &test_term());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_help_h_flag() {
        let result = parse_args(["prog", "-h"], &test_term());
        assert!(result.unwrap().is_none());
    }
}
