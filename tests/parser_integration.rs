use std::fs;

use ipl_playoff_calculator::parse_inputs;

const FIXTURES_DIR: &str = "tests/fixtures";

#[test]
fn test_valid_9_remaining() {
    let input = fs::read_to_string(format!("{}/valid_9_remaining.txt", FIXTURES_DIR)).unwrap();
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(parsed.team_count, 10);
    assert_eq!(parsed.matches.len(), 9);
    assert_eq!(parsed.completed_matches, 61);
}

#[test]
fn test_valid_15_remaining() {
    let input = fs::read_to_string(format!("{}/valid_15_remaining.txt", FIXTURES_DIR)).unwrap();
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(parsed.team_count, 10);
    assert_eq!(parsed.matches.len(), 15);
    assert_eq!(parsed.completed_matches, 55);
}

#[test]
fn test_valid_25_remaining() {
    let input = fs::read_to_string(format!("{}/valid_25_remaining.txt", FIXTURES_DIR)).unwrap();
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(parsed.team_count, 10);
    assert_eq!(parsed.matches.len(), 25);
    assert_eq!(parsed.completed_matches, 45);
}

#[test]
fn test_valid_40_remaining() {
    let input = fs::read_to_string(format!("{}/valid_40_remaining.txt", FIXTURES_DIR)).unwrap();
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(parsed.team_count, 10);
    assert_eq!(parsed.matches.len(), 40);
    assert_eq!(parsed.completed_matches, 30);
}

#[test]
fn test_invalid_team_vs_itself() {
    let input = fs::read_to_string(format!("{}/invalid_team_vs_itself.txt", FIXTURES_DIR)).unwrap();
    let result = parse_inputs(&input);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("cannot play against itself")
    );
}

#[test]
fn test_invalid_outcome_not_in_match() {
    let input =
        fs::read_to_string(format!("{}/invalid_outcome_not_in_match.txt", FIXTURES_DIR)).unwrap();
    let result = parse_inputs(&input);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Invalid outcome"));
}

#[test]
fn test_invalid_malformed_line() {
    let input = fs::read_to_string(format!("{}/invalid_malformed_line.txt", FIXTURES_DIR)).unwrap();
    let result = parse_inputs(&input);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Invalid match line")
    );
}

#[test]
fn test_invalid_too_many_teams() {
    let input = fs::read_to_string(format!("{}/invalid_too_many_teams.txt", FIXTURES_DIR)).unwrap();
    let result = parse_inputs(&input);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Too many teams"));
}

#[test]
fn test_invalid_empty() {
    let input = fs::read_to_string(format!("{}/invalid_empty.txt", FIXTURES_DIR)).unwrap();
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(parsed.team_count, 0);
    assert_eq!(parsed.matches.len(), 0);
}
