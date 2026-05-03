use std::collections::HashMap;
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::ops::AddAssign;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

// ================================================================
// TERMINAL COLORS
// ================================================================

const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";
const GREEN: &str = "\x1b[32m";
const MAGENTA: &str = "\x1b[35m";
const NO_COLOR: &str = "";

#[derive(Clone, Copy)]
struct Colors {
    bold: &'static str,
    reset: &'static str,
    cyan: &'static str,
    yellow: &'static str,
    green: &'static str,
    magenta: &'static str,
}

impl Colors {
    fn new(enabled: bool) -> Self {
        if enabled {
            Self {
                bold: BOLD,
                reset: RESET,
                cyan: CYAN,
                yellow: YELLOW,
                green: GREEN,
                magenta: MAGENTA,
            }
        } else {
            Self {
                bold: NO_COLOR,
                reset: NO_COLOR,
                cyan: NO_COLOR,
                yellow: NO_COLOR,
                green: NO_COLOR,
                magenta: NO_COLOR,
            }
        }
    }
}

// ================================================================
// CONSTANTS
// ================================================================

// Internal max capacity to maintain stack allocation performance
const MAX_TEAMS: usize = 16;

/// Task multiplier: generate at least this many tasks per thread for good load balancing.
const TASKS_PER_THREAD_TARGET: u64 = 512;
const PROGRESS_POLL_INTERVAL_MS: u64 = 100;

// ================================================================
// ERROR TYPE
// ================================================================

#[derive(Debug)]
enum AppError {
    Io(std::io::Error),
    Parse(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Io(e) => write!(f, "I/O error: {}", e),
            AppError::Parse(msg) => write!(f, "Parse error: {}", msg),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e)
    }
}

// ================================================================
// MATH HELPERS
// ================================================================

const fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

const fn lcm_u64(a: u64, b: u64) -> u64 {
    if a == 0 || b == 0 {
        0
    } else {
        a / gcd_u64(a, b) * b
    }
}

const fn seat_scale_for_team_count(team_count: usize) -> u64 {
    let mut scale = 1u64;
    let mut x = 1usize;
    while x <= team_count {
        scale = lcm_u64(scale, x as u64);
        x += 1;
    }
    scale
}

fn pow_u64(base: u64, exp: usize) -> Result<u64, AppError> {
    (0..exp).try_fold(1u64, |acc, _| {
        acc.checked_mul(base)
            .ok_or_else(|| AppError::Parse("Scenario count overflowed u64".to_string()))
    })
}

// ================================================================
// CLI
// ================================================================

struct CliArgs {
    file_path: String,
    allow_no_results: bool,
}

fn parse_args() -> Result<CliArgs, AppError> {
    let mut args = env::args();
    let program_name = args
        .next()
        .unwrap_or_else(|| "ipl-playoff-calculator".to_string());

    let mut file_path: Option<String> = None;
    let mut allow_no_results = false;

    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => {
                print_usage(&program_name);
                std::process::exit(0);
            }
            "--allow-no-results" => {
                allow_no_results = true;
            }
            _ if arg.starts_with('-') => {
                return Err(AppError::Parse(format!("Unknown flag: {}", arg)));
            }
            _ => {
                if file_path.is_some() {
                    return Err(AppError::Parse(
                        "Expected exactly one matches file path".to_string(),
                    ));
                }
                file_path = Some(arg);
            }
        }
    }

    let file_path = file_path.ok_or_else(|| {
        AppError::Parse("Missing matches file path. Use --help for usage.".to_string())
    })?;

    Ok(CliArgs {
        file_path,
        allow_no_results,
    })
}

fn print_usage(program_name: &str) {
    eprintln!("{BOLD}{CYAN}IPL Playoff Calculator{RESET}\n");
    eprintln!(
        "{BOLD}{YELLOW}Usage:{RESET} {} [--allow-no-results] <matches-file>",
        program_name
    );
    eprintln!("\n{BOLD}Arguments:{RESET}");
    eprintln!("  <matches-file> Path to the text file containing the schedule.");
    eprintln!(
        "  --allow-no-results (Optional) Include ties/washouts (1 pt each) in future outcomes."
    );
    eprintln!("\n{BOLD}Matches File Format Instructions:{RESET}");
    eprintln!("  - Provide one match per line. Lines starting with '#' are ignored.");
    eprintln!("  - {BOLD}Upcoming match:{RESET} Team A vs Team B");
    eprintln!("  - {BOLD}Completed match:{RESET} Team A vs Team B : Winner Team");
    eprintln!("  - {BOLD}No Result / Tie:{RESET} Team A vs Team B : NR");
    eprintln!("\n{BOLD}Example:{RESET}");
    eprintln!("  CSK vs RCB : CSK");
    eprintln!("  MI vs DC\n");
}

// ================================================================
// DATA MODELS
// ================================================================

#[derive(Clone, Copy, Default, Debug)]
struct StandingState {
    points: [u8; MAX_TEAMS],
    wins: [u8; MAX_TEAMS],
}

impl StandingState {
    fn record_win(&mut self, team: usize) {
        self.points[team] += 2;
        self.wins[team] += 1;
    }

    fn undo_win(&mut self, team: usize) {
        self.wins[team] -= 1;
        self.points[team] -= 2;
    }

    fn record_no_result(&mut self, a: usize, b: usize) {
        self.points[a] += 1;
        self.points[b] += 1;
    }

    fn undo_no_result(&mut self, a: usize, b: usize) {
        self.points[b] -= 1;
        self.points[a] -= 1;
    }
}

#[derive(Clone, Copy, Default, Debug)]
struct Counts {
    top2_pts: [u64; MAX_TEAMS],
    top2_good_nrr_units: [u64; MAX_TEAMS],
    top4_pts: [u64; MAX_TEAMS],
    top4_good_nrr_units: [u64; MAX_TEAMS],
}

impl AddAssign<&Counts> for Counts {
    fn add_assign(&mut self, other: &Counts) {
        for i in 0..MAX_TEAMS {
            self.top2_pts[i] += other.top2_pts[i];
            self.top2_good_nrr_units[i] += other.top2_good_nrr_units[i];
            self.top4_pts[i] += other.top4_pts[i];
            self.top4_good_nrr_units[i] += other.top4_good_nrr_units[i];
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Task {
    next_match: usize,
    state: StandingState,
}

#[derive(Clone, Copy, Default, Debug)]
struct Group {
    points: u8,
    wins: u8,
    members: [usize; MAX_TEAMS],
    len: usize,
}

#[derive(Clone, Debug)]
struct ParsedInput {
    team_names: Vec<String>,
    team_count: usize,
    seat_scale: u64,
    initial_state: StandingState,
    matches_played: [u8; MAX_TEAMS],
    losses: [u8; MAX_TEAMS],
    no_results: [u8; MAX_TEAMS],
    matches: Vec<(usize, usize)>,
    completed_matches: usize,
}

#[derive(Clone, Debug)]
struct Row {
    team: String,
    top2_pts: u64,
    top2_good_nrr_units: u64,
    top4_pts: u64,
    top4_good_nrr_units: u64,
}

struct TableLayout {
    team_w: usize,
    pos_w: usize,
    stat_w: usize,
    pct_w: usize,
}

impl TableLayout {
    fn from_input(parsed: &ParsedInput) -> Self {
        Self {
            team_w: parsed
                .team_names
                .iter()
                .map(|name| name.len())
                .max()
                .unwrap_or(6)
                .max(6),
            pos_w: 8,
            stat_w: 4,
            pct_w: 20,
        }
    }
}

// ================================================================
// PARSING
// ================================================================

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

fn apply_completed_outcome(
    outcome_str: &str,
    line: &str,
    a_name: &str,
    b_name: &str,
    a: usize,
    b: usize,
    state: &mut StandingState,
    losses: &mut [u8; MAX_TEAMS],
    no_results: &mut [u8; MAX_TEAMS],
) -> Result<(), AppError> {
    let outcome = canonical(outcome_str.trim());

    if outcome == canonical("NR") {
        state.record_no_result(a, b);
        no_results[a] += 1;
        no_results[b] += 1;
        return Ok(());
    }

    if outcome == canonical(a_name) {
        state.record_win(a);
        losses[b] += 1;
        return Ok(());
    }

    if outcome == canonical(b_name) {
        state.record_win(b);
        losses[a] += 1;
        return Ok(());
    }

    Err(AppError::Parse(format!(
        "Invalid outcome '{}' in line '{}'. Expected '{}', '{}', or 'NR'",
        outcome_str.trim(),
        line,
        a_name,
        b_name
    )))
}

fn parse_inputs(matches_input: &str) -> Result<ParsedInput, AppError> {
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

        // splitn(2, ':') safely handles colons inside team names or comments
        let mut parts = line.splitn(2, ':');
        let match_part = parts.next().unwrap_or("").trim();
        let outcome_part = parts.next();

        let (a_name, b_name) = parse_match_line(match_part)?;

        // Validate that a match isn't listed against itself
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
                outcome_str.trim(),
                line,
                &a_name,
                &b_name,
                a,
                b,
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

fn read_matches_file(path: &str) -> Result<String, AppError> {
    fs::read_to_string(path)
        .map_err(|e| AppError::Parse(format!("Error reading file '{}': {}", path, e)))
}

// ================================================================
// RANKING
// ================================================================

struct CutoffCounts<'a> {
    guaranteed: &'a mut [u64; MAX_TEAMS],
    seat_aware_units: &'a mut [u64; MAX_TEAMS],
}

impl<'a> CutoffCounts<'a> {
    fn new(
        guaranteed: &'a mut [u64; MAX_TEAMS],
        seat_aware_units: &'a mut [u64; MAX_TEAMS],
    ) -> Self {
        Self {
            guaranteed,
            seat_aware_units,
        }
    }
}

#[derive(Clone)]
struct Ranker {
    team_count: usize,
    seat_scale: u64,
}

impl Ranker {
    fn new(team_count: usize, seat_scale: u64) -> Self {
        Self {
            team_count,
            seat_scale,
        }
    }

    fn classify(&self, state: &StandingState, counts: &mut Counts) {
        let order = sort_teams(self.team_count, &state.points, &state.wins);
        let (groups, group_count) =
            build_groups(&order, self.team_count, &state.points, &state.wins);

        apply_cutoff(
            &groups,
            group_count,
            2,
            self.seat_scale,
            CutoffCounts::new(&mut counts.top2_pts, &mut counts.top2_good_nrr_units),
        );

        apply_cutoff(
            &groups,
            group_count,
            4,
            self.seat_scale,
            CutoffCounts::new(&mut counts.top4_pts, &mut counts.top4_good_nrr_units),
        );
    }
}

fn sort_teams(
    team_count: usize,
    points: &[u8; MAX_TEAMS],
    wins: &[u8; MAX_TEAMS],
) -> [usize; MAX_TEAMS] {
    let mut order = [0usize; MAX_TEAMS];

    for i in 0..team_count {
        order[i] = i;
    }

    order[..team_count].sort_unstable_by(|&a, &b| {
        points[b]
            .cmp(&points[a])
            .then(wins[b].cmp(&wins[a]))
            .then(a.cmp(&b))
    });

    order
}

fn build_groups(
    order: &[usize; MAX_TEAMS],
    team_count: usize,
    points: &[u8; MAX_TEAMS],
    wins: &[u8; MAX_TEAMS],
) -> ([Group; MAX_TEAMS], usize) {
    let mut groups = [Group::default(); MAX_TEAMS];
    let mut group_count = 0usize;

    for &team in order.iter().take(team_count) {
        if group_count > 0 {
            let last = &mut groups[group_count - 1];
            if last.points == points[team] && last.wins == wins[team] {
                last.members[last.len] = team;
                last.len += 1;
                continue;
            }
        }

        let mut group = Group {
            points: points[team],
            wins: wins[team],
            members: [0usize; MAX_TEAMS],
            len: 1,
        };
        group.members[0] = team;
        groups[group_count] = group;
        group_count += 1;
    }

    (groups, group_count)
}

fn apply_cutoff(
    groups: &[Group; MAX_TEAMS],
    group_count: usize,
    cutoff: usize,
    seat_scale: u64,
    counts: CutoffCounts<'_>,
) {
    let mut placed_above = 0usize;

    for group in groups.iter().take(group_count) {
        let spots_here = if placed_above >= cutoff {
            0usize
        } else {
            (cutoff - placed_above).min(group.len)
        };

        let fully_inside_cutoff = spots_here == group.len;
        let seat_units_per_team = if spots_here == 0 {
            0u64
        } else {
            (spots_here as u64) * seat_scale / (group.len as u64)
        };

        for idx in 0..group.len {
            let team = group.members[idx];

            if seat_units_per_team > 0 {
                counts.seat_aware_units[team] += seat_units_per_team;
            }

            if fully_inside_cutoff {
                counts.guaranteed[team] += 1;
            }
        }

        placed_above += group.len;
    }
}

// ================================================================
// SIMULATION
// ================================================================

#[derive(Clone)]
struct Simulator {
    matches: Arc<Vec<(usize, usize)>>,
    ranker: Ranker,
    allow_no_results: bool,
    base: u64,
}

impl Simulator {
    fn new(parsed: &ParsedInput, allow_no_results: bool) -> Self {
        Self {
            matches: Arc::new(parsed.matches.clone()),
            ranker: Ranker::new(parsed.team_count, parsed.seat_scale),
            allow_no_results,
            base: if allow_no_results { 3 } else { 2 },
        }
    }

    fn remaining_match_count(&self) -> usize {
        self.matches.len()
    }

    fn total_scenarios(&self) -> Result<u64, AppError> {
        pow_u64(self.base, self.remaining_match_count())
    }

    fn choose_split_depth(&self, num_threads: usize) -> Result<usize, AppError> {
        let mut split_depth = 0usize;
        let mut task_count = 1u64;

        while split_depth < self.remaining_match_count()
            && task_count < (num_threads as u64 * TASKS_PER_THREAD_TARGET)
        {
            split_depth += 1;
            task_count = task_count
                .checked_mul(self.base)
                .ok_or_else(|| AppError::Parse("Task count overflowed u64".to_string()))?;
        }

        Ok(split_depth)
    }

    fn task_count_for_depth(&self, split_depth: usize) -> Result<u64, AppError> {
        pow_u64(self.base, split_depth)
    }

    fn scenarios_per_task(&self, split_depth: usize) -> Result<u64, AppError> {
        pow_u64(self.base, self.remaining_match_count() - split_depth)
    }

    fn classify(&self, state: &StandingState, counts: &mut Counts) {
        self.ranker.classify(state, counts);
    }

    fn build_tasks(
        &self,
        split_depth: usize,
        initial_state: StandingState,
    ) -> Result<Vec<Task>, AppError> {
        let capacity = self.task_count_for_depth(split_depth)? as usize;
        let mut tasks = Vec::with_capacity(capacity);
        self.build_tasks_from(0, split_depth, initial_state, &mut tasks);
        Ok(tasks)
    }

    fn build_tasks_from(
        &self,
        match_idx: usize,
        split_depth: usize,
        state: StandingState,
        tasks: &mut Vec<Task>,
    ) {
        if match_idx == split_depth {
            tasks.push(Task {
                next_match: match_idx,
                state,
            });
            return;
        }

        let (a, b) = self.matches[match_idx];
        self.build_a_win_task(match_idx, split_depth, state, a, tasks);
        self.build_b_win_task(match_idx, split_depth, state, b, tasks);

        if self.allow_no_results {
            self.build_no_result_task(match_idx, split_depth, state, a, b, tasks);
        }
    }

    fn build_a_win_task(
        &self,
        match_idx: usize,
        split_depth: usize,
        mut state: StandingState,
        team: usize,
        tasks: &mut Vec<Task>,
    ) {
        state.record_win(team);
        self.build_tasks_from(match_idx + 1, split_depth, state, tasks);
    }

    fn build_b_win_task(
        &self,
        match_idx: usize,
        split_depth: usize,
        mut state: StandingState,
        team: usize,
        tasks: &mut Vec<Task>,
    ) {
        state.record_win(team);
        self.build_tasks_from(match_idx + 1, split_depth, state, tasks);
    }

    fn build_no_result_task(
        &self,
        match_idx: usize,
        split_depth: usize,
        mut state: StandingState,
        a: usize,
        b: usize,
        tasks: &mut Vec<Task>,
    ) {
        state.record_no_result(a, b);
        self.build_tasks_from(match_idx + 1, split_depth, state, tasks);
    }

    fn simulate_task(&self, task: &Task) -> Counts {
        let mut counts = Counts::default();
        let mut state = task.state;
        self.dfs_from(task.next_match, &mut state, &mut counts);
        counts
    }

    fn dfs_from(&self, match_idx: usize, state: &mut StandingState, counts: &mut Counts) {
        if match_idx == self.remaining_match_count() {
            self.classify(state, counts);
            return;
        }

        let (a, b) = self.matches[match_idx];
        self.explore_a_win(match_idx, a, state, counts);
        self.explore_b_win(match_idx, b, state, counts);

        if self.allow_no_results {
            self.explore_no_result(match_idx, a, b, state, counts);
        }
    }

    fn explore_a_win(
        &self,
        match_idx: usize,
        team: usize,
        state: &mut StandingState,
        counts: &mut Counts,
    ) {
        state.record_win(team);
        self.dfs_from(match_idx + 1, state, counts);
        state.undo_win(team);
    }

    fn explore_b_win(
        &self,
        match_idx: usize,
        team: usize,
        state: &mut StandingState,
        counts: &mut Counts,
    ) {
        state.record_win(team);
        self.dfs_from(match_idx + 1, state, counts);
        state.undo_win(team);
    }

    fn explore_no_result(
        &self,
        match_idx: usize,
        a: usize,
        b: usize,
        state: &mut StandingState,
        counts: &mut Counts,
    ) {
        state.record_no_result(a, b);
        self.dfs_from(match_idx + 1, state, counts);
        state.undo_no_result(a, b);
    }
}

// ================================================================
// NEXT MATCH OUTCOME SIMULATION
// ================================================================

#[derive(Clone, Copy)]
enum NextMatchOutcome {
    TeamAWin,
    TeamBWin,
    NoResult,
}

impl NextMatchOutcome {
    fn title(self, a_name: &str, b_name: &str) -> String {
        match self {
            NextMatchOutcome::TeamAWin => format!("If {} beats {}", a_name, b_name),
            NextMatchOutcome::TeamBWin => format!("If {} beats {}", b_name, a_name),
            NextMatchOutcome::NoResult => format!("If {} vs {} ends in NR", a_name, b_name),
        }
    }
}

fn conditioned_input_for_next_match(
    parsed: &ParsedInput,
    outcome: NextMatchOutcome,
) -> Result<ParsedInput, AppError> {
    let (a, b) = parsed
        .matches
        .first()
        .copied()
        .ok_or_else(|| AppError::Parse("No remaining matches available".to_string()))?;

    let mut conditioned = parsed.clone();
    conditioned.matches = parsed.matches[1..].to_vec();
    conditioned.completed_matches += 1;
    conditioned.matches_played[a] += 1;
    conditioned.matches_played[b] += 1;

    match outcome {
        NextMatchOutcome::TeamAWin => {
            conditioned.initial_state.record_win(a);
            conditioned.losses[b] += 1;
        }
        NextMatchOutcome::TeamBWin => {
            conditioned.initial_state.record_win(b);
            conditioned.losses[a] += 1;
        }
        NextMatchOutcome::NoResult => {
            conditioned.initial_state.record_no_result(a, b);
            conditioned.no_results[a] += 1;
            conditioned.no_results[b] += 1;
        }
    }

    Ok(conditioned)
}

fn simulate_next_match_outcome(
    parsed: &ParsedInput,
    allow_no_results: bool,
    outcome: NextMatchOutcome,
    num_threads: usize,
    reporter: &Reporter,
) -> Result<(), AppError> {
    let conditioned = conditioned_input_for_next_match(parsed, outcome)?;
    let simulator = Simulator::new(&conditioned, allow_no_results);
    let total_scenarios = simulator.total_scenarios()?;

    let (a, b) = parsed.matches[0];
    let title = outcome.title(&parsed.team_names[a], &parsed.team_names[b]);

    reporter.print_next_match_scenario_heading(&title);

    let counts = simulate_all(
        &conditioned,
        &simulator,
        num_threads,
        false, // keep the extra tables quiet; set to true if you want progress here too
        reporter.colors(),
        total_scenarios,
    )?;

    reporter.print_results(
        &conditioned,
        &counts,
        total_scenarios,
        conditioned.seat_scale,
    );
    println!();

    Ok(())
}

// ================================================================
// PROGRESS
// ================================================================

struct ProgressTracker {
    total_scenarios: u64,
    scenarios_per_task: u64,
    scenarios_done: Arc<AtomicU64>,
}

impl ProgressTracker {
    fn new(total_scenarios: u64, scenarios_per_task: u64) -> Self {
        Self {
            total_scenarios,
            scenarios_per_task,
            scenarios_done: Arc::new(AtomicU64::new(0)),
        }
    }

    fn counter(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.scenarios_done)
    }

    fn scenarios_per_task(&self) -> u64 {
        self.scenarios_per_task
    }

    fn run_ui_loop(&self, interactive: bool, colors: &Colors, start_time: Instant) {
        if !interactive {
            return;
        }

        let mut last_drawn = u64::MAX;

        loop {
            let done = self
                .scenarios_done
                .load(Ordering::Relaxed)
                .min(self.total_scenarios);

            if done != last_drawn {
                self.draw(done, colors, start_time);
                last_drawn = done;
            }

            if done >= self.total_scenarios {
                break;
            }

            thread::sleep(Duration::from_millis(PROGRESS_POLL_INTERVAL_MS));
        }

        println!("\n");
    }

    fn draw(&self, done: u64, colors: &Colors, start_time: Instant) {
        let elapsed = start_time.elapsed().as_secs_f64();
        let pct = done as f64 / self.total_scenarios as f64;

        let eta = if done > 0 {
            let seconds_per_scenario = elapsed / done as f64;
            seconds_per_scenario * (self.total_scenarios - done) as f64
        } else {
            0.0
        };

        let bar_width = 40usize;
        let filled = ((pct * bar_width as f64) as usize).min(bar_width);

        let bar: String = (0..bar_width)
            .map(|i| {
                if i < filled {
                    '='
                } else if i == filled && done < self.total_scenarios {
                    '>'
                } else {
                    ' '
                }
            })
            .collect();

        print!(
            "\r{}Progress:{} [{}] {}{:>5.1}%{} | {}Scenarios:{} {}/{} | {}Elapsed:{} {:>5.1}s | {}ETA:{} {:>5.1}s ",
            colors.cyan,
            colors.reset,
            bar,
            colors.bold,
            pct * 100.0,
            colors.reset,
            colors.magenta,
            colors.reset,
            format_with_commas(done),
            format_with_commas(self.total_scenarios),
            colors.yellow,
            colors.reset,
            elapsed,
            colors.green,
            colors.reset,
            eta
        );
        io::stdout().flush().unwrap();
    }
}

// ================================================================
// PARALLEL EXECUTION
// ================================================================

struct ParallelSimulator {
    simulator: Simulator,
    num_threads: usize,
}

impl ParallelSimulator {
    fn new(simulator: Simulator, num_threads: usize) -> Self {
        Self {
            simulator,
            num_threads,
        }
    }

    fn run(
        &self,
        tasks: Vec<Task>,
        progress: &ProgressTracker,
        interactive: bool,
        colors: &Colors,
    ) -> Result<Counts, AppError> {
        let tasks = Arc::new(tasks);
        let next_task = Arc::new(AtomicUsize::new(0));
        let start_time = Instant::now();

        let handles = self.spawn_workers(tasks, next_task, progress);
        progress.run_ui_loop(interactive, colors, start_time);
        self.collect_counts(handles)
    }

    fn spawn_workers(
        &self,
        tasks: Arc<Vec<Task>>,
        next_task: Arc<AtomicUsize>,
        progress: &ProgressTracker,
    ) -> Vec<thread::JoinHandle<Counts>> {
        let mut handles = Vec::with_capacity(self.num_threads);

        for _ in 0..self.num_threads {
            handles.push(self.spawn_worker(
                Arc::clone(&tasks),
                Arc::clone(&next_task),
                progress.counter(),
                progress.scenarios_per_task(),
            ));
        }

        handles
    }

    fn spawn_worker(
        &self,
        tasks: Arc<Vec<Task>>,
        next_task: Arc<AtomicUsize>,
        scenarios_done: Arc<AtomicU64>,
        scenarios_per_task: u64,
    ) -> thread::JoinHandle<Counts> {
        let simulator = self.simulator.clone();

        thread::spawn(move || -> Counts {
            let mut local = Counts::default();

            loop {
                let idx = next_task.fetch_add(1, Ordering::Relaxed);
                if idx >= tasks.len() {
                    break;
                }

                let partial = simulator.simulate_task(&tasks[idx]);
                local += &partial;
                scenarios_done.fetch_add(scenarios_per_task, Ordering::Relaxed);
            }

            local
        })
    }

    fn collect_counts(&self, handles: Vec<thread::JoinHandle<Counts>>) -> Result<Counts, AppError> {
        let mut total_counts = Counts::default();

        for handle in handles {
            let local = handle
                .join()
                .map_err(|_| AppError::Parse("A worker thread panicked".to_string()))?;
            total_counts += &local;
        }

        Ok(total_counts)
    }
}

// ================================================================
// REPORTING
// ================================================================

struct Reporter {
    colors: Colors,
    layout: TableLayout,
}

impl Reporter {
    fn new(parsed: &ParsedInput, interactive: bool) -> Self {
        Self {
            colors: Colors::new(interactive),
            layout: TableLayout::from_input(parsed),
        }
    }

    fn colors(&self) -> &Colors {
        &self.colors
    }

    fn standings_table_width(&self) -> usize {
        // pos_w + team_w + 5 stats columns + 6 space separators
        self.layout.pos_w + self.layout.team_w + (5 * self.layout.stat_w) + 6
    }

    fn probabilities_table_width(&self) -> usize {
        // team_w + 4 pct columns + 5 space separators
        self.layout.pos_w + self.layout.team_w + (4 * self.layout.pct_w) + 5
    }

    fn print_current_standings(&self, parsed: &ParsedInput) {
        let heading_text = format!(
            "========= Current Standings after Match {} =========",
            parsed.completed_matches
        );
        let table_width = self.standings_table_width();
        println!(
            "{}{:^width$}{}",
            self.colors.cyan,
            heading_text,
            self.colors.reset,
            width = table_width
        );

        println!(
            "{}{:>pos_w$} {:team_w$} {:>stat_w$} {:>stat_w$} {:>stat_w$} {:>stat_w$} {:>stat_w$}{}",
            self.colors.yellow,
            "",
            "Team",
            "M",
            "W",
            "L",
            "NR",
            "Pts",
            self.colors.reset,
            pos_w = self.layout.pos_w,
            team_w = self.layout.team_w,
            stat_w = self.layout.stat_w,
        );

        let current_order = sort_teams(
            parsed.team_count,
            &parsed.initial_state.points,
            &parsed.initial_state.wins,
        );

        for (idx, &team_idx) in current_order.iter().take(parsed.team_count).enumerate() {
            println!(
                "{:>pos_w$} {}{:team_w$}{} {:>stat_w$} {:>stat_w$} {:>stat_w$} {:>stat_w$} {:>stat_w$}",
                idx + 1,
                self.colors.green,
                parsed.team_names[team_idx],
                self.colors.reset,
                parsed.matches_played[team_idx],
                parsed.initial_state.wins[team_idx],
                parsed.losses[team_idx],
                parsed.no_results[team_idx],
                parsed.initial_state.points[team_idx],
                pos_w = self.layout.pos_w,
                team_w = self.layout.team_w,
                stat_w = self.layout.stat_w,
            );
        }
    }

    fn print_simulation_header(
        &self,
        completed_matches: usize,
        remaining_matches: usize,
        base: u64,
        total_scenarios: u64,
        num_threads: usize,
    ) {
        println!();
        println!(
            "{}========= League Status ========={}",
            self.colors.cyan, self.colors.reset
        );
        println!(
            "    {}Matches Completed :{} {}",
            self.colors.magenta, self.colors.reset, completed_matches
        );
        println!(
            "    {}Matches Remaining:{} {}",
            self.colors.magenta, self.colors.reset, remaining_matches
        );
        println!(
            "    {}Outcome Mode:{} {} per match",
            self.colors.magenta, self.colors.reset, base
        );
        println!(
            "    {}Total Scenarios:{} {}",
            self.colors.magenta,
            self.colors.reset,
            format_with_commas(total_scenarios)
        );
        println!(
            "    {}Threads:{} {}",
            self.colors.magenta, self.colors.reset, num_threads
        );
        println!();
    }

    fn print_current_probabilities_heading(&self, parsed: &ParsedInput) {
        let heading_text = format!(
            "========= Current Probabilities after Match {} =========",
            parsed.completed_matches
        );
        let table_width = self.probabilities_table_width();

        println!(
            "{}{:^width$}{}",
            self.colors.cyan,
            heading_text,
            self.colors.reset,
            width = table_width
        );
    }

    fn print_next_match_impact_heading(&self, parsed: &ParsedInput) {
        if let Some(&(a, b)) = parsed.matches.first() {
            let heading_text = format!(
                "========= Impact of Next Match {}: {} vs {} =========",
                parsed.completed_matches + 1,
                parsed.team_names[a],
                parsed.team_names[b]
            );
            let table_width = self.probabilities_table_width();

            println!(
                "{}{:^width$}{}\n",
                self.colors.magenta,
                heading_text,
                self.colors.reset,
                width = table_width
            );
        }
    }

    fn print_next_match_scenario_heading(&self, title: &str) {
        let heading_text = format!("========= {} =========", title);
        let table_width = self.probabilities_table_width();
        println!(
            "{}{:^width$}{}",
            self.colors.cyan,
            heading_text,
            self.colors.reset,
            width = table_width
        );
    }

    fn print_results(
        &self,
        parsed: &ParsedInput,
        total_counts: &Counts,
        total_scenarios: u64,
        seat_scale: u64,
    ) {
        let mut rows: Vec<Row> = (0..parsed.team_count)
            .map(|i| Row {
                team: parsed.team_names[i].clone(),
                top2_pts: total_counts.top2_pts[i],
                top2_good_nrr_units: total_counts.top2_good_nrr_units[i],
                top4_pts: total_counts.top4_pts[i],
                top4_good_nrr_units: total_counts.top4_good_nrr_units[i],
            })
            .collect();

        rows.sort_by(|a, b| {
            b.top2_pts
                .cmp(&a.top2_pts)
                .then(b.top2_good_nrr_units.cmp(&a.top2_good_nrr_units))
                .then(b.top4_pts.cmp(&a.top4_pts))
                .then(b.top4_good_nrr_units.cmp(&a.top4_good_nrr_units))
                .then(a.team.cmp(&b.team))
        });

        println!(
            "{:>pos_w$} {}{:team_w$} {:>pct_w$} {:>pct_w$} {:>pct_w$} {:>pct_w$}{}",
            "",
            self.colors.yellow,
            "Team",
            "Top 2 Pts",
            "Top 2 Pts+Good NRR",
            "Top 4 Pts",
            "Top 4 Pts+Good NRR",
            self.colors.reset,
            pos_w = self.layout.pos_w,
            team_w = self.layout.team_w,
            pct_w = self.layout.pct_w,
        );

        for row in rows {
            println!(
                "{:>pos_w$} {}{:team_w$}{} {:>pct_w$} {:>pct_w$} {:>pct_w$} {:>pct_w$}",
                "",
                self.colors.green,
                row.team,
                self.colors.reset,
                fmt_pct(row.top2_pts, total_scenarios),
                fmt_scaled_pct(row.top2_good_nrr_units, total_scenarios, seat_scale),
                fmt_pct(row.top4_pts, total_scenarios),
                fmt_scaled_pct(row.top4_good_nrr_units, total_scenarios, seat_scale),
                pos_w = self.layout.pos_w,
                team_w = self.layout.team_w,
                pct_w = self.layout.pct_w,
            );
        }
    }
}

// ================================================================
// FORMATTING HELPERS
// ================================================================

fn format_with_commas(n: u64) -> String {
    let mut s = n.to_string();
    let mut i = s.len();
    while i > 3 {
        i -= 3;
        s.insert(i, ',');
    }
    s
}

fn fmt_pct(numerator: u64, denominator: u64) -> String {
    if numerator == 0 {
        "-".to_string()
    } else {
        format!("{:.2}%", numerator as f64 * 100.0 / denominator as f64)
    }
}

fn fmt_scaled_pct(units: u64, total_scenarios: u64, seat_scale: u64) -> String {
    if units == 0 {
        "-".to_string()
    } else {
        let denom = (total_scenarios as f64) * (seat_scale as f64);
        format!("{:.2}%", (units as f64) * 100.0 / denom)
    }
}

// ================================================================
// ORCHESTRATION
// ================================================================

fn determine_num_threads() -> usize {
    thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4)
}

fn simulate_all(
    parsed: &ParsedInput,
    simulator: &Simulator,
    num_threads: usize,
    interactive: bool,
    colors: &Colors,
    total_scenarios: u64,
) -> Result<Counts, AppError> {
    if simulator.remaining_match_count() == 0 {
        let mut counts = Counts::default();
        simulator.classify(&parsed.initial_state, &mut counts);
        return Ok(counts);
    }

    let split_depth = simulator.choose_split_depth(num_threads)?;
    let tasks = simulator.build_tasks(split_depth, parsed.initial_state)?;
    let scenarios_per_task = simulator.scenarios_per_task(split_depth)?;
    let progress = ProgressTracker::new(total_scenarios, scenarios_per_task);
    let parallel = ParallelSimulator::new(simulator.clone(), num_threads);

    parallel.run(tasks, &progress, interactive, colors)
}

fn run() -> Result<(), AppError> {
    let cli = parse_args()?;
    let interactive = io::stdout().is_terminal();

    let matches_input = read_matches_file(&cli.file_path)?;
    let parsed = parse_inputs(&matches_input)?;
    let reporter = Reporter::new(&parsed, interactive);
    let simulator = Simulator::new(&parsed, cli.allow_no_results);

    let total_scenarios = simulator.total_scenarios()?;
    let num_threads = determine_num_threads();

    reporter.print_current_standings(&parsed);
    reporter.print_simulation_header(
        parsed.completed_matches,
        simulator.remaining_match_count(),
        simulator.base,
        total_scenarios,
        num_threads,
    );

    if simulator.remaining_match_count() == 0 {
        println!("No remaining matches to simulate.");
    }

    let total_counts = simulate_all(
        &parsed,
        &simulator,
        num_threads,
        interactive,
        reporter.colors(),
        total_scenarios,
    )?;

    reporter.print_current_probabilities_heading(&parsed);
    reporter.print_results(&parsed, &total_counts, total_scenarios, parsed.seat_scale);

    if !parsed.matches.is_empty() {
        println!();
        reporter.print_next_match_impact_heading(&parsed);

        simulate_next_match_outcome(
            &parsed,
            cli.allow_no_results,
            NextMatchOutcome::TeamAWin,
            num_threads,
            &reporter,
        )?;

        simulate_next_match_outcome(
            &parsed,
            cli.allow_no_results,
            NextMatchOutcome::TeamBWin,
            num_threads,
            &reporter,
        )?;

        if cli.allow_no_results {
            simulate_next_match_outcome(
                &parsed,
                cli.allow_no_results,
                NextMatchOutcome::NoResult,
                num_threads,
                &reporter,
            )?;
        }
    }

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{}Error:{} {}", YELLOW, RESET, e);
        std::process::exit(1);
    }
}
