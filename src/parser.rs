use std::collections::HashMap;
use std::fs;

use crate::models::{AppError, MAX_TEAMS, ParsedInput, StandingState};
use crate::utils::seat_scale_for_team_count;

pub fn read_matches_file(path: &str) -> Result<String, AppError> {
    fs::read_to_string(path)
        .map_err(|e| AppError::Parse(format!("Error reading file '{}': {}", path, e)))
}

pub fn parse_inputs(matches_input: &str) -> Result<ParsedInput, AppError> {
    let mut team_names: Vec<String> = Vec::new();
    let mut team_map: HashMap<String, usize> = HashMap::new();
    let mut initial_state = StandingState::default();
    let mut matches_played = [0u8; MAX_TEAMS];
    let mut losses = [0u8; MAX_TEAMS];
    let mut no_results = [0u8; MAX_TEAMS];
    let mut matches = Vec::new();
    let mut completed_matches = 0usize;

    for raw_line in matches_input.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(2, ':');
        let match_part = parts.next().unwrap_or("").trim();
        let outcome_part = parts.next();
        let (a_name, b_name) = parse_match_line(match_part)?;
        if canonical(&a_name) == canonical(&b_name) {
            return Err(AppError::Parse(format!(
                "A team cannot play against itself: '{}'",
                line
            )));
        }
        let a = get_or_insert_team(&a_name, &mut team_names, &mut team_map)?;
        let b = get_or_insert_team(&b_name, &mut team_names, &mut team_map)?;
        if let Some(outcome_str) = outcome_part {
            completed_matches += 1;
            matches_played[a] += 1;
            matches_played[b] += 1;
            apply_completed_outcome(
                CompletedMatchContext {
                    outcome_str: outcome_str.trim(),
                    line,
                    a_name: &a_name,
                    b_name: &b_name,
                    a,
                    b,
                },
                &mut initial_state,
                &mut losses,
                &mut no_results,
            )?;
        } else {
            matches.push((a, b));
        }
    }

    let team_count = team_names.len();
    let seat_scale = seat_scale_for_team_count(team_count);
    Ok(ParsedInput {
        team_names,
        team_count,
        seat_scale,
        initial_state,
        matches_played,
        losses,
        no_results,
        matches,
        completed_matches,
    })
}

fn canonical(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn parse_match_line(line: &str) -> Result<(String, String), AppError> {
    let lower = line.to_ascii_lowercase();
    if let Some(pos) = lower.find(" vs ") {
        return Ok((
            line[..pos].trim().to_string(),
            line[pos + 4..].trim().to_string(),
        ));
    }
    if let Some(pos) = lower.find(" v ") {
        return Ok((
            line[..pos].trim().to_string(),
            line[pos + 3..].trim().to_string(),
        ));
    }
    if let Some(pos) = line.find(',') {
        return Ok((
            line[..pos].trim().to_string(),
            line[pos + 1..].trim().to_string(),
        ));
    }
    Err(AppError::Parse(format!(
        "Invalid match line: '{}'. Expected 'Team A vs Team B'",
        line
    )))
}

fn get_or_insert_team(
    name: &str,
    team_names: &mut Vec<String>,
    team_map: &mut HashMap<String, usize>,
) -> Result<usize, AppError> {
    let key = canonical(name);
    if let Some(&idx) = team_map.get(&key) {
        return Ok(idx);
    }
    let idx = team_names.len();
    if idx >= MAX_TEAMS {
        return Err(AppError::Parse(format!(
            "Too many teams! Max supported is {}",
            MAX_TEAMS
        )));
    }
    team_names.push(name.to_string());
    team_map.insert(key, idx);
    Ok(idx)
}

struct CompletedMatchContext<'a> {
    outcome_str: &'a str,
    line: &'a str,
    a_name: &'a str,
    b_name: &'a str,
    a: usize,
    b: usize,
}

fn apply_completed_outcome(
    ctx: CompletedMatchContext<'_>,
    state: &mut StandingState,
    losses: &mut [u8; MAX_TEAMS],
    no_results: &mut [u8; MAX_TEAMS],
) -> Result<(), AppError> {
    let outcome = canonical(ctx.outcome_str.trim());

    if outcome == canonical("NR") {
        state.record_no_result(ctx.a, ctx.b);
        no_results[ctx.a] += 1;
        no_results[ctx.b] += 1;
        return Ok(());
    }

    if outcome == canonical(ctx.a_name) {
        state.record_win(ctx.a);
        losses[ctx.b] += 1;
        return Ok(());
    }

    if outcome == canonical(ctx.b_name) {
        state.record_win(ctx.b);
        losses[ctx.a] += 1;
        return Ok(());
    }

    Err(AppError::Parse(format!(
        "Invalid outcome '{}' in line '{}'. Expected '{}', '{}', or 'NR'",
        ctx.outcome_str.trim(),
        ctx.line,
        ctx.a_name,
        ctx.b_name
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_completed_and_upcoming() {
        let input = "SRH vs RCB : SRH\nRCB vs MI\n";
        let parsed = parse_inputs(input).unwrap();
        assert_eq!(parsed.team_count, 3);
        assert_eq!(parsed.team_names, vec!["SRH", "RCB", "MI"]);
        assert_eq!(parsed.completed_matches, 1);
        assert_eq!(parsed.matches.len(), 1);
        assert_eq!(parsed.initial_state.points(0), 2);
        assert_eq!(parsed.losses[1], 1);
    }

    #[test]
    fn test_parse_nr_outcome() {
        let input = "SRH vs RCB : NR\n";
        let parsed = parse_inputs(input).unwrap();
        assert_eq!(parsed.completed_matches, 1);
        assert_eq!(parsed.matches.len(), 0);
        assert_eq!(parsed.initial_state.points(0), 1);
        assert_eq!(parsed.initial_state.points(1), 1);
        assert_eq!(parsed.no_results[0], 1);
        assert_eq!(parsed.no_results[1], 1);
    }

    #[test]
    fn test_parse_comments_and_blank_lines_ignored() {
        let input = "# comment\n\nSRH vs RCB : SRH\n# another comment\n\n";
        let parsed = parse_inputs(input).unwrap();
        assert_eq!(parsed.completed_matches, 1);
        assert_eq!(parsed.matches.len(), 0);
    }

    #[test]
    fn test_parse_v_separator() {
        let input = "SRH v RCB : SRH\n";
        let parsed = parse_inputs(input).unwrap();
        assert_eq!(parsed.completed_matches, 1);
    }

    #[test]
    fn test_parse_comma_separator() {
        let input = "SRH, RCB : SRH\n";
        let parsed = parse_inputs(input).unwrap();
        assert_eq!(parsed.completed_matches, 1);
    }

    #[test]
    fn test_parse_team_vs_itself_error() {
        let input = "SRH vs SRH : SRH\n";
        let result = parse_inputs(input);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cannot play against itself"));
    }

    #[test]
    fn test_parse_invalid_outcome_error() {
        let input = "SRH vs RCB : MI\n";
        let result = parse_inputs(input);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid outcome"));
    }

    #[test]
    fn test_parse_malformed_line_error() {
        let input = "SRH RCB\n";
        let result = parse_inputs(input);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid match line"));
    }

    #[test]
    fn test_parse_too_many_teams_error() {
        let input = "T1 vs T2\nT3 vs T4\nT5 vs T6\nT7 vs T8\nT9 vs T10\nT11 vs T1\n";
        let result = parse_inputs(input);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Too many teams"));
    }

    #[test]
    fn test_parse_empty_input() {
        let input = "# only comments\n\n";
        let parsed = parse_inputs(input).unwrap();
        assert_eq!(parsed.team_count, 0);
        assert_eq!(parsed.completed_matches, 0);
        assert_eq!(parsed.matches.len(), 0);
    }

    #[test]
    fn test_parse_multiple_completed_matches() {
        let input = "SRH vs RCB : SRH\nRCB vs MI : MI\nSRH vs MI : NR\n";
        let parsed = parse_inputs(input).unwrap();
        assert_eq!(parsed.team_count, 3);
        assert_eq!(parsed.completed_matches, 3);
        assert_eq!(parsed.matches_played[0], 2);
        assert_eq!(parsed.matches_played[1], 2);
        assert_eq!(parsed.matches_played[2], 2);
        assert_eq!(parsed.initial_state.points(0), 3);
        assert_eq!(parsed.initial_state.points(1), 0);
        assert_eq!(parsed.initial_state.points(2), 3);
    }

    #[test]
    fn test_parse_seat_scale_for_10_teams() {
        let input = "T1 vs T2\nT3 vs T4\nT5 vs T6\nT7 vs T8\nT9 vs T10\n";
        let parsed = parse_inputs(input).unwrap();
        assert_eq!(parsed.team_count, 10);
        assert_eq!(parsed.seat_scale, 2520);
    }

    #[test]
    fn test_canonical_normalization() {
        assert_eq!(canonical("SRH"), "SRH");
        assert_eq!(canonical("srh"), "SRH");
        assert_eq!(canonical("S.R.H."), "SRH");
        assert_eq!(canonical("RCB"), "RCB");
    }

    #[test]
    fn test_parse_case_insensitive_outcome() {
        let input = "SRH vs RCB : srh\n";
        let parsed = parse_inputs(input).unwrap();
        assert_eq!(parsed.initial_state.points(0), 2);
    }

    #[test]
    fn test_parse_case_insensitive_nr() {
        let input = "SRH vs RCB : nr\n";
        let parsed = parse_inputs(input).unwrap();
        assert_eq!(parsed.initial_state.points(0), 1);
        assert_eq!(parsed.initial_state.points(1), 1);
    }
}
