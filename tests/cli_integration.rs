use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_ipl_playoff_calculator")
}

fn fixture(name: &str) -> String {
    format!("tests/fixtures/{}", name)
}

#[test]
fn test_help_exit_code_and_output() {
    let output = Command::new(binary()).arg("--help").output().unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("IPL Playoff Calculator"));
    assert!(stderr.contains("Usage:"));
}

#[test]
fn test_help_h_flag() {
    let output = Command::new(binary()).arg("-h").output().unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("IPL Playoff Calculator"));
}

#[test]
fn test_valid_file() {
    let output = Command::new(binary())
        .arg(fixture("valid_9_remaining.txt"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Probabilities"));
}

#[test]
fn test_missing_file_error() {
    let output = Command::new(binary())
        .arg("nonexistent.txt")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Error reading file"));
}

#[test]
fn test_malformed_input_error() {
    let output = Command::new(binary())
        .arg(fixture("invalid_malformed_line.txt"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid match line"));
}

#[test]
fn test_unknown_flag_error() {
    let output = Command::new(binary())
        .args(["--bogus", &fixture("valid_9_remaining.txt")])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Unknown flag"));
}

#[test]
fn test_no_args_error() {
    let output = Command::new(binary()).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Missing matches file path"));
}

#[test]
fn test_allow_no_results_flag() {
    let output = Command::new(binary())
        .args(["--allow-no-results", &fixture("valid_9_remaining.txt")])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("3 per match"));
}

#[test]
fn test_duplicate_file_path_error() {
    let output = Command::new(binary())
        .args([
            &fixture("valid_9_remaining.txt"),
            &fixture("valid_15_remaining.txt"),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("exactly one"));
}
