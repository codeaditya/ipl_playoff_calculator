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

#[derive(Clone, Copy)]
struct Colors {
    clear: &'static str,
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
                clear: "\x1b[2K",
                bold: "\x1b[1m",
                reset: "\x1b[0m",
                cyan: "\x1b[36m",
                yellow: "\x1b[33m",
                green: "\x1b[32m",
                magenta: "\x1b[35m",
            }
        } else {
            Self {
                clear: "",
                bold: "",
                reset: "",
                cyan: "",
                yellow: "",
                green: "",
                magenta: "",
            }
        }
    }
}

// ================================================================
// CONSTANTS
// ================================================================

const MAX_TEAMS: usize = 10;
const TASKS_PER_THREAD_TARGET: u64 = 512;
const PROGRESS_POLL_INTERVAL_MS: u64 = 100;

// Slot constants stored in Task::slot — which branch of match 0 this task covers.
// Using u8 keeps Task small and avoids enum padding.
const SLOT_UNSET: u8 = 0; // task starts before match 0 has been branched (split_depth == 0)
const SLOT_A: u8 = 1; // A wins match 0
const SLOT_B: u8 = 2; // B wins match 0
const SLOT_NR: u8 = 3; // no result in match 0

// ================================================================
// ALGORITHM SELECTION
// ================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Algorithm {
    Auto,
    Dfs,
    Dp,
}

impl fmt::Display for Algorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Algorithm::Auto => write!(f, "AUTO"),
            Algorithm::Dfs => write!(f, "DFS"),
            Algorithm::Dp => write!(f, "DP"),
        }
    }
}

// ================================================================
// ERROR TYPE
// ================================================================

#[derive(Debug)]
enum AppError {
    Parse(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Parse(msg) => write!(f, "{}", msg),
        }
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

fn pow_u64(base: u64, exp: usize) -> u64 {
    (0..exp)
        .try_fold(1u64, |acc, _| acc.checked_mul(base))
        .unwrap_or(u64::MAX)
}

// ================================================================
// CLI
// ================================================================

struct CliArgs {
    file_path: String,
    allow_no_results: bool,
    algorithm: Algorithm,
}

fn parse_args() -> Result<CliArgs, AppError> {
    let mut args = env::args();
    let program_name = args
        .next()
        .unwrap_or_else(|| "ipl-playoff-calculator".to_string());

    let interactive = io::stdout().is_terminal();
    let colors = Colors::new(interactive);

    let mut file_path: Option<String> = None;
    let mut allow_no_results = false;
    let mut algorithm = Algorithm::Auto;

    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => {
                print_usage(&program_name, &colors);
                std::process::exit(0);
            }
            "--allow-no-results" => {
                allow_no_results = true;
            }
            "--algo=auto" | "--algo=AUTO" => {
                algorithm = Algorithm::Auto;
            }
            "--algo=dfs" | "--algo=DFS" => {
                algorithm = Algorithm::Dfs;
            }
            "--algo=dp" | "--algo=DP" => {
                algorithm = Algorithm::Dp;
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
        algorithm,
    })
}

fn print_usage(program_name: &str, c: &Colors) {
    eprintln!(
        "{bold}{cyan}IPL Playoff Calculator{reset}\n",
        bold = c.bold,
        cyan = c.cyan,
        reset = c.reset
    );
    eprintln!(
        "{bold}{yellow}Usage:{reset} {magenta}{prog}{reset} {cyan}[--allow-no-results]{reset} {cyan}[--algo=auto|dfs|dp]{reset} {green}<matches-file>{reset}",
        bold = c.bold,
        yellow = c.yellow,
        reset = c.reset,
        magenta = c.magenta,
        prog = program_name,
        cyan = c.cyan,
        green = c.green,
    );
    eprintln!(
        "\n{bold}{yellow}Arguments:{reset}",
        bold = c.bold,
        yellow = c.yellow,
        reset = c.reset
    );
    eprintln!(
        "  {green}<matches-file>{reset}       Path to the text file containing the schedule.",
        green = c.green,
        reset = c.reset
    );
    eprintln!(
        "  {cyan}--allow-no-results{reset}   (Optional) Include ties/washouts (1 pt each) in future outcomes.",
        cyan = c.cyan,
        reset = c.reset
    );
    eprintln!(
        "  {cyan}--algo=auto{reset}          (Default) Dynamically scales between pure DP and Hybrid DFS-DP based on available system RAM.",
        cyan = c.cyan,
        reset = c.reset
    );
    eprintln!(
        "  {cyan}--algo=dfs{reset}           DFS simulation: low RAM (~<5 MB), slower for large match counts.",
        cyan = c.cyan,
        reset = c.reset
    );
    eprintln!(
        "  {cyan}--algo=dp{reset}            DP simulation: faster for large match counts, but uses significantly more RAM.",
        cyan = c.cyan,
        reset = c.reset
    );
    eprintln!(
        "\n{bold}{yellow}Matches File Format:{reset}",
        bold = c.bold,
        yellow = c.yellow,
        reset = c.reset
    );
    eprintln!("  - One match per line. Lines starting with '#' are ignored.");
    eprintln!(
        "  - {magenta}Upcoming:{reset}  Team A vs Team B",
        magenta = c.magenta,
        reset = c.reset,
    );
    eprintln!(
        "  - {magenta}Completed:{reset} Team A vs Team B : Winner",
        magenta = c.magenta,
        reset = c.reset,
    );
    eprintln!(
        "  - {magenta}No Result:{reset} Team A vs Team B : NR",
        magenta = c.magenta,
        reset = c.reset,
    );
    eprintln!(
        "\n{bold}{yellow}Example:{reset}",
        bold = c.bold,
        yellow = c.yellow,
        reset = c.reset
    );
    eprintln!("  CSK vs RCB : CSK");
    eprintln!("  MI vs DC\n");
}

// ================================================================
// DATA MODELS
// ================================================================

const TEAM_BITS: usize = 10;
const TEAM_MASK: u128 = 0x3FF;
const WIN_SCORE_DELTA: u128 = (2 << 4) | 1; // 33 (2 points, 1 win)
const NR_SCORE_DELTA: u128 = (1 << 4) | 0; // 16 (1 point, 0 wins)

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct StandingState {
    score: u128,
}

impl StandingState {
    #[inline]
    fn record_win(&mut self, team: usize) {
        self.score += WIN_SCORE_DELTA << (team * TEAM_BITS);
    }

    #[inline]
    fn undo_win(&mut self, team: usize) {
        self.score -= WIN_SCORE_DELTA << (team * TEAM_BITS);
    }

    #[inline]
    fn record_no_result(&mut self, a: usize, b: usize) {
        self.score += NR_SCORE_DELTA << (a * TEAM_BITS);
        self.score += NR_SCORE_DELTA << (b * TEAM_BITS);
    }

    #[inline]
    fn undo_no_result(&mut self, a: usize, b: usize) {
        self.score -= NR_SCORE_DELTA << (a * TEAM_BITS);
        self.score -= NR_SCORE_DELTA << (b * TEAM_BITS);
    }

    #[inline]
    pub fn points(&self, team: usize) -> u8 {
        (((self.score >> (team * TEAM_BITS)) & TEAM_MASK) >> 4) as u8
    }

    #[inline]
    pub fn wins(&self, team: usize) -> u8 {
        ((self.score >> (team * TEAM_BITS)) & 0xF) as u8
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

#[derive(Clone, Copy, Default, Debug)]
struct AllCounts {
    overall: Counts,
    if_a_wins: Counts,
    if_b_wins: Counts,
    if_nr: Counts,
}

impl AddAssign<&AllCounts> for AllCounts {
    fn add_assign(&mut self, other: &AllCounts) {
        self.overall += &other.overall;
        self.if_a_wins += &other.if_a_wins;
        self.if_b_wins += &other.if_b_wins;
        self.if_nr += &other.if_nr;
    }
}

#[derive(Clone, Copy, Debug)]
struct Task {
    next_match: usize,
    state: StandingState,
    slot: u8, // SLOT_A / SLOT_B / SLOT_NR / SLOT_UNSET
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

struct CompletedMatchContext<'a> {
    outcome_str: &'a str,
    line: &'a str,
    a_name: &'a str,
    b_name: &'a str,
    a: usize,
    b: usize,
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

fn read_matches_file(path: &str) -> Result<String, AppError> {
    fs::read_to_string(path)
        .map_err(|e| AppError::Parse(format!("Error reading file '{}': {}", path, e)))
}

// ================================================================
// RANKING
// ================================================================

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

    #[inline]
    fn classify(&self, state: &StandingState, counts: &mut Counts) {
        // Unpack u128 into a fast array for the sorting and grouping logic
        let mut scores = [0u16; MAX_TEAMS];
        for (i, score) in scores.iter_mut().enumerate().take(self.team_count) {
            *score = ((state.score >> (i * TEAM_BITS)) & TEAM_MASK) as u16;
        }

        let order = sort_teams(self.team_count, &scores);

        let mut start = 0;
        let mut placed_above = 0;

        while start < self.team_count {
            let mut end = start + 1;
            let score_val = scores[order[start]];

            // Group teams with identical points and wins
            while end < self.team_count && scores[order[end]] == score_val {
                end += 1;
            }

            let group_len = end - start;
            let group_len_u64 = group_len as u64;

            // --- TOP 2 Cutoff ---
            let spots_top2 = if placed_above >= 2 {
                0
            } else {
                (2 - placed_above).min(group_len)
            };
            let units_top2 = if spots_top2 == 0 {
                0
            } else {
                (spots_top2 as u64 * self.seat_scale) / group_len_u64
            };

            // --- TOP 4 Cutoff ---
            let spots_top4 = if placed_above >= 4 {
                0
            } else {
                (4 - placed_above).min(group_len)
            };
            let units_top4 = if spots_top4 == 0 {
                0
            } else {
                (spots_top4 as u64 * self.seat_scale) / group_len_u64
            };

            // Assign stats directly to the combined counts struct
            for &team in order.iter().take(end).skip(start) {
                if units_top2 > 0 {
                    counts.top2_good_nrr_units[team] += units_top2;
                }
                if spots_top2 == group_len {
                    counts.top2_pts[team] += 1;
                }
                if units_top4 > 0 {
                    counts.top4_good_nrr_units[team] += units_top4;
                }
                if spots_top4 == group_len {
                    counts.top4_pts[team] += 1;
                }
            }

            start = end;
            placed_above += group_len;
        }
    }
}

fn sort_teams(team_count: usize, scores: &[u16; MAX_TEAMS]) -> [usize; MAX_TEAMS] {
    let mut order = [0usize; MAX_TEAMS];
    for (i, slot) in order.iter_mut().enumerate().take(team_count) {
        *slot = i;
    }
    for i in 1..team_count {
        let key = order[i];
        let mut j = i;
        while j > 0 {
            let prev = order[j - 1];
            if scores[prev] >= scores[key] {
                break;
            }
            order[j] = prev;
            j -= 1;
        }
        order[j] = key;
    }
    order
}

// ================================================================
// DFS SIMULATION
// ================================================================
//
// Key design: the DFS carries exactly ONE Counts, identical to the
// original code. The slot (which branch of match 0 applies to every
// leaf in this sub-tree) is fixed at task-build time and stored in
// Task::slot. simulate_task() runs the standard DFS to get one Counts,
// then routes it into the right AllCounts bucket with two += operations.
// This means:
//   - DFS stack frame size = unchanged (one Counts*)
//   - Per-task merge cost = 2× Counts AddAssign (negligible vs DFS work)
//   - No slot checks inside the hot DFS loop at all

// Low RAM (<5 MB), multi-threaded via work-stealing task queue.
// Runtime roughly doubles (or triples with --allow-no-results) per
// additional remaining match. Shows a real-time progress bar.

#[derive(Clone)]
struct DfsSimulator {
    matches: Arc<Vec<(usize, usize)>>,
    ranker: Ranker,
    allow_no_results: bool,
    pub base: u64,
}

impl DfsSimulator {
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

    fn total_scenarios(&self) -> u64 {
        pow_u64(self.base, self.remaining_match_count())
    }

    fn choose_split_depth(&self, num_threads: usize) -> usize {
        let mut split_depth = 0usize;
        let mut task_count = 1u64;
        while split_depth < self.remaining_match_count()
            && task_count < (num_threads as u64 * TASKS_PER_THREAD_TARGET)
        {
            split_depth += 1;
            task_count = task_count.saturating_mul(self.base);
        }
        split_depth
    }

    fn task_count_for_depth(&self, split_depth: usize) -> u64 {
        pow_u64(self.base, split_depth)
    }

    fn scenarios_per_task(&self, split_depth: usize) -> u64 {
        pow_u64(self.base, self.remaining_match_count() - split_depth)
    }

    // ── Task building ──────────────────────────────────────────────────
    // Slot is resolved when match 0 is first branched and then propagated
    // unchanged into every descendant task. Tasks that start after match
    // 0 already carry their final slot; the DFS never needs to check it.

    fn build_tasks(&self, split_depth: usize, initial_state: StandingState) -> Vec<Task> {
        let capacity = self.task_count_for_depth(split_depth) as usize;
        let mut tasks = Vec::with_capacity(capacity);
        self.build_tasks_from(0, split_depth, initial_state, SLOT_UNSET, &mut tasks);
        tasks
    }

    fn build_tasks_from(
        &self,
        match_idx: usize,
        split_depth: usize,
        state: StandingState,
        slot: u8,
        tasks: &mut Vec<Task>,
    ) {
        if match_idx == split_depth {
            tasks.push(Task {
                next_match: match_idx,
                state,
                slot,
            });
            return;
        }
        let (a, b) = self.matches[match_idx];

        // Resolve slot exactly once: only when branching match 0.
        let (slot_a, slot_b, slot_nr) = if slot == SLOT_UNSET {
            (SLOT_A, SLOT_B, SLOT_NR)
        } else {
            (slot, slot, slot)
        };

        let mut sa = state;
        sa.record_win(a);
        self.build_tasks_from(match_idx + 1, split_depth, sa, slot_a, tasks);

        let mut sb = state;
        sb.record_win(b);
        self.build_tasks_from(match_idx + 1, split_depth, sb, slot_b, tasks);

        if self.allow_no_results {
            let mut snr = state;
            snr.record_no_result(a, b);
            self.build_tasks_from(match_idx + 1, split_depth, snr, slot_nr, tasks);
        }
    }

    // ── Per-task execution ─────────────────────────────────────────────
    // The DFS carries a single Counts. After it completes, simulate_task
    // routes the result into AllCounts with two cheap AddAssign calls
    // (overall + one conditioned bucket).

    fn simulate_task(&self, task: &Task) -> AllCounts {
        let mut counts = Counts::default();
        let mut state = task.state;
        self.dfs_from(task.next_match, &mut state, &mut counts);

        let mut all = AllCounts::default();
        all.overall += &counts;
        match task.slot {
            SLOT_A => all.if_a_wins += &counts,
            SLOT_B => all.if_b_wins += &counts,
            SLOT_NR => all.if_nr += &counts,
            _ => {} // SLOT_UNSET: split_depth == 0, no next-match tables needed
        }
        all
    }

    fn dfs_from(&self, match_idx: usize, state: &mut StandingState, counts: &mut Counts) {
        if match_idx == self.remaining_match_count() {
            self.ranker.classify(state, counts);
            return;
        }
        let (a, b) = self.matches[match_idx];

        state.record_win(a);
        self.dfs_from(match_idx + 1, state, counts);
        state.undo_win(a);

        state.record_win(b);
        self.dfs_from(match_idx + 1, state, counts);
        state.undo_win(b);

        if self.allow_no_results {
            state.record_no_result(a, b);
            self.dfs_from(match_idx + 1, state, counts);
            state.undo_no_result(a, b);
        }
    }

    fn run(
        &self,
        initial_state: &StandingState,
        num_threads: usize,
        interactive: bool,
        colors: &Colors,
    ) -> AllCounts {
        if self.remaining_match_count() == 0 {
            let mut counts = Counts::default();
            self.ranker.classify(initial_state, &mut counts);
            let mut all = AllCounts::default();
            all.overall += &counts;
            return all;
        }
        let split_depth = self.choose_split_depth(num_threads);
        let tasks = self.build_tasks(split_depth, *initial_state);
        let scenarios_per_task = self.scenarios_per_task(split_depth);
        let progress = ProgressTracker::new(self.total_scenarios(), scenarios_per_task);
        let parallel = ParallelDfsSimulator::new(self.clone(), num_threads);
        parallel.run(tasks, &progress, interactive, colors)
    }
}

// ================================================================
// DFS PARALLEL EXECUTION
// ================================================================

struct ParallelDfsSimulator {
    simulator: DfsSimulator,
    num_threads: usize,
}

impl ParallelDfsSimulator {
    fn new(simulator: DfsSimulator, num_threads: usize) -> Self {
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
    ) -> AllCounts {
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
    ) -> Vec<thread::JoinHandle<AllCounts>> {
        (0..self.num_threads)
            .map(|_| {
                self.spawn_worker(
                    Arc::clone(&tasks),
                    Arc::clone(&next_task),
                    progress.counter(),
                    progress.scenarios_per_task(),
                )
            })
            .collect()
    }

    fn spawn_worker(
        &self,
        tasks: Arc<Vec<Task>>,
        next_task: Arc<AtomicUsize>,
        scenarios_done: Arc<AtomicU64>,
        scenarios_per_task: u64,
    ) -> thread::JoinHandle<AllCounts> {
        let simulator = self.simulator.clone();
        thread::spawn(move || {
            let mut local = AllCounts::default();
            loop {
                let idx = next_task.fetch_add(1, Ordering::Relaxed);
                if idx >= tasks.len() {
                    break;
                }
                local += &simulator.simulate_task(&tasks[idx]);
                scenarios_done.fetch_add(scenarios_per_task, Ordering::Relaxed);
            }
            local
        })
    }

    fn collect_counts(&self, handles: Vec<thread::JoinHandle<AllCounts>>) -> AllCounts {
        let mut total = AllCounts::default();
        for handle in handles {
            total += &handle.join().expect("worker thread panicked");
        }
        total
    }
}

// ================================================================
// DFS PROGRESS TRACKER
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
                draw_progress(
                    ProgressPhase::Dfs {
                        done,
                        total: self.total_scenarios,
                    },
                    colors,
                    start_time,
                );
                last_drawn = done;
            }
            if done >= self.total_scenarios {
                break;
            }
            thread::sleep(Duration::from_millis(PROGRESS_POLL_INTERVAL_MS));
        }
        println!("\n");
    }
}

// ================================================================
// MEMORY HELPERS
// ================================================================

/// Returns current RSS (resident set size) in bytes by reading
/// /proc/self/status on Linux. Returns None on unsupported platforms.
fn current_rss_bytes() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in text.lines() {
        if line.starts_with("VmRSS:") {
            let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

fn fmt_mem(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}

// ================================================================
// PROGRESS BAR (shared helper)
// ================================================================

pub enum ProgressPhase {
    Dfs {
        done: u64,
        total: u64,
    },
    DpSimulating {
        match_idx: usize,
        total_matches: usize,
        state_count: usize,
    },
    DpClassifying {
        states_done: usize,
        total_states: usize,
    },
}

fn draw_progress(phase: ProgressPhase, colors: &Colors, start_time: Instant) {
    let mem_str = current_rss_bytes()
        .map(fmt_mem)
        .unwrap_or_else(|| "N/A".to_string());

    let elapsed = start_time.elapsed().as_secs_f64();

    let (pct, info_str) = match phase {
        ProgressPhase::Dfs { done, total } => {
            let p = if total == 0 {
                1.0
            } else {
                done as f64 / total as f64
            };
            let info = format!(
                "Scenarios: {c}{}{r}/{y}{}{r}",
                format_with_commas(done),
                format_with_commas(total),
                c = colors.cyan,
                y = colors.yellow,
                r = colors.reset
            );
            (p, info)
        }
        ProgressPhase::DpSimulating {
            match_idx,
            total_matches,
            state_count,
        } => {
            let p = if total_matches == 0 {
                1.0
            } else {
                match_idx as f64 / total_matches as f64
            };
            // A power of 10 roughly maps the exponential state growth of the
            // DP algorithm based on observed cumulative sum data.
            let p_curved = p.powf(10.0);
            let info = format!(
                "Simulating: {c}{}{r}/{y}{}{r} | States: {g}{}{r}",
                match_idx,
                total_matches,
                format_with_commas(state_count as u64),
                c = colors.cyan,
                y = colors.yellow,
                g = colors.green,
                r = colors.reset
            );
            (p_curved, info)
        }
        ProgressPhase::DpClassifying {
            states_done,
            total_states,
        } => {
            let p = if total_states == 0 {
                1.0
            } else {
                states_done as f64 / total_states as f64
            };
            let info = format!(
                "Classifying: {c}{}{r}/{y}{}{r}",
                format_with_commas(states_done as u64),
                format_with_commas(total_states as u64),
                c = colors.cyan,
                y = colors.yellow,
                r = colors.reset
            );
            (p, info)
        }
    };

    let eta = if pct > 0.0 && pct < 1.0 {
        (elapsed / pct) - elapsed
    } else {
        0.0
    };

    let bar_width = 40_usize;
    let filled = (pct * bar_width as f64) as usize;
    let filled = filled.min(bar_width);

    let bar: String = (0..bar_width)
        .map(|i| {
            if i < filled {
                '█'
            } else if i == filled {
                '▓'
            } else {
                '░'
            }
        })
        .collect();

    print!(
        "\r{clear}{bold}{cyan}Progress:{reset} [{magenta}{bar}{reset}] {bold}{pct:>5.1}%{reset} | {info} | Elapsed: {cyan}{elapsed:.1}s{reset} | ETA: {magenta}{eta:.1}s{reset} | RAM: {ram}",
        clear = colors.clear,
        bold = colors.bold,
        cyan = colors.cyan,
        reset = colors.reset,
        magenta = colors.magenta,
        info = info_str,
        pct = pct * 100.0,
        bar = bar,
        elapsed = elapsed,
        eta = eta,
        ram = mem_str,
    );
    io::stdout().flush().unwrap();
}

// ================================================================
// DP SIMULATION
// ================================================================
//
// Merges states with identical standings after each match.
// A state is stored as a packed u128 score vector.
//
// The weight attached to each state tracks three counters:
// [0] if_a_wins – scenarios where the first remaining match was won by team A
// [1] if_b_wins – scenarios where the first remaining match was won by team B
// [2] if_nr – scenarios where the first remaining match was a no-result
//
// overall is not stored explicitly; it is reconstructed later as
// if_a_wins + if_b_wins + if_nr.
//
// After all matches are processed, each unique final state is
// classified once and multiplied by its weights — replacing 3^N leaf
// classifications with O(distinct_states) classifications.
//
// Progress: emits a per-match progress bar (one tick per match).

#[derive(Clone)]
struct DpSimulator {
    matches: Vec<(usize, usize)>,
    ranker: Ranker,
    allow_no_results: bool,
    pub base: u64,
}

impl DpSimulator {
    fn new(parsed: &ParsedInput, allow_no_results: bool) -> Self {
        Self {
            matches: parsed.matches.clone(),
            ranker: Ranker::new(parsed.team_count, parsed.seat_scale),
            allow_no_results,
            base: if allow_no_results { 3 } else { 2 },
        }
    }

    fn remaining_match_count(&self) -> usize {
        self.matches.len()
    }

    fn total_scenarios(&self) -> u64 {
        pow_u64(self.base, self.remaining_match_count())
    }

    // ================================================================
    // CORE MERGE LOGIC
    // ================================================================
    fn process_next_match(
        &self,
        mut states: Vec<Vec<(u128, u64)>>,
        total_states: usize,
        a: usize,
        b: usize,
    ) -> (Vec<Vec<(u128, u64)>>, usize) {
        const CHUNK_SHIFT: usize = 18;
        const CHUNK_MASK: usize = 0x3FFFF;
        const CHUNK_SIZE: usize = 262_144;

        let expected_len = (total_states * 15) / 10;
        let expected_chunks = (expected_len / CHUNK_SIZE) + 1;
        let mut next_states = Vec::with_capacity(expected_chunks);
        next_states.push(Vec::with_capacity(CHUNK_SIZE));

        let delta_a = WIN_SCORE_DELTA << (a * TEAM_BITS);
        let delta_b = WIN_SCORE_DELTA << (b * TEAM_BITS);
        let delta_nr = if self.allow_no_results {
            (NR_SCORE_DELTA << (a * TEAM_BITS)) | (NR_SCORE_DELTA << (b * TEAM_BITS))
        } else {
            0
        };

        let mut idx_a = 0;
        let mut idx_b = 0;
        let mut idx_nr = if self.allow_no_results {
            0
        } else {
            total_states
        };

        let len = total_states;
        let mut next_len = 0;
        let mut current_chunk = 0;
        let mut last_freed_chunk = 0;

        while idx_a < len || idx_b < len || idx_nr < len {
            let min_idx = idx_a.min(idx_b).min(idx_nr);
            let safe_to_free_chunk = min_idx >> CHUNK_SHIFT;
            while last_freed_chunk < safe_to_free_chunk {
                states[last_freed_chunk] = Vec::new(); // Instantly frees memory
                last_freed_chunk += 1;
            }

            let state_a = if idx_a < len {
                states[idx_a >> CHUNK_SHIFT][idx_a & CHUNK_MASK]
            } else {
                (u128::MAX, 0)
            };
            let state_b = if idx_b < len {
                states[idx_b >> CHUNK_SHIFT][idx_b & CHUNK_MASK]
            } else {
                (u128::MAX, 0)
            };
            let state_nr = if idx_nr < len {
                states[idx_nr >> CHUNK_SHIFT][idx_nr & CHUNK_MASK]
            } else {
                (u128::MAX, 0)
            };

            let val_a = if idx_a < len {
                state_a.0 + delta_a
            } else {
                u128::MAX
            };
            let val_b = if idx_b < len {
                state_b.0 + delta_b
            } else {
                u128::MAX
            };
            let val_nr = if idx_nr < len {
                state_nr.0 + delta_nr
            } else {
                u128::MAX
            };

            let min_val = val_a.min(val_b).min(val_nr);
            let mut w = 0;

            if val_a == min_val {
                w += state_a.1;
                idx_a += 1;
            }
            if val_b == min_val {
                w += state_b.1;
                idx_b += 1;
            }
            if val_nr == min_val {
                w += state_nr.1;
                idx_nr += 1;
            }

            if next_states[current_chunk].len() == CHUNK_SIZE {
                next_states.push(Vec::with_capacity(CHUNK_SIZE));
                current_chunk += 1;
            }
            next_states[current_chunk].push((min_val, w));
            next_len += 1;
        }

        (next_states, next_len)
    }

    // ================================================================
    // SIMULATION LOOP
    // ================================================================
    fn build_states(
        &self,
        branch_initial_state: StandingState,
        matches: &[(usize, usize)],
        interactive: bool,
        colors: &Colors,
        global_start_time: Instant,
        is_hybrid: bool,
    ) -> (Vec<Vec<(u128, u64)>>, usize) {
        // In pure DP mode (is_hybrid = false), add 1 to account for Match 0
        // which was pre-processed by the caller.
        // In hybrid mode (is_hybrid = true), no match is pre-processed.
        let total_matches = if is_hybrid {
            matches.len()
        } else {
            matches.len() + 1
        };
        let mut total_states = 1;
        let mut states: Vec<Vec<(u128, u64)>> = vec![vec![(branch_initial_state.score, 1)]];

        if interactive {
            draw_progress(
                ProgressPhase::DpSimulating {
                    match_idx: if is_hybrid { 0 } else { 1 },
                    total_matches,
                    state_count: total_states,
                },
                colors,
                global_start_time,
            );
        }

        for (idx, &(a, b)) in matches.iter().enumerate() {
            let (next_states, next_len) = self.process_next_match(states, total_states, a, b);
            states = next_states;
            total_states = next_len;

            if interactive {
                draw_progress(
                    ProgressPhase::DpSimulating {
                        match_idx: if is_hybrid { idx + 1 } else { idx + 2 },
                        total_matches,
                        state_count: total_states,
                    },
                    colors,
                    global_start_time,
                );
            }
        }

        (states, total_states)
    }

    // ================================================================
    // PARALLEL CLASSIFICATION
    // ================================================================
    fn classify_states_parallel(
        &self,
        states: Vec<Vec<(u128, u64)>>,
        total_states: usize,
        interactive: bool,
        colors: &Colors,
        global_start_time: Instant,
    ) -> Counts {
        let num_threads = determine_num_threads();
        // Wrap the chunked states in a thread-safe Iterator
        // Using into_iter() means chunks are consumed and their memory is instantly freed when a thread is done with them.
        let chunk_iter = Arc::new(std::sync::Mutex::new(states.into_iter()));
        let states_done = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::with_capacity(num_threads);

        for _ in 0..num_threads {
            let chunk_iter = Arc::clone(&chunk_iter);
            let states_done = Arc::clone(&states_done);
            let ranker = self.ranker.clone(); // Ranker is cheap to clone

            handles.push(thread::spawn(move || {
                let mut local_counts = Counts::default();

                loop {
                    // Safely pull the next chunk from the queue
                    let chunk_opt = {
                        let mut iter = chunk_iter.lock().unwrap();
                        iter.next()
                    };

                    let chunk = match chunk_opt {
                        Some(c) => c,
                        None => break, // No more chunks, thread can exit
                    };

                    let chunk_len = chunk.len();

                    // Classify every state in the chunk
                    for (score, w) in chunk {
                        let state = StandingState { score };
                        let mut leaf = Counts::default();
                        ranker.classify(&state, &mut leaf);

                        for j in 0..ranker.team_count {
                            local_counts.top2_pts[j] += leaf.top2_pts[j] * w;
                            local_counts.top2_good_nrr_units[j] += leaf.top2_good_nrr_units[j] * w;
                            local_counts.top4_pts[j] += leaf.top4_pts[j] * w;
                            local_counts.top4_good_nrr_units[j] += leaf.top4_good_nrr_units[j] * w;
                        }
                    }
                    // Update progress counter
                    states_done.fetch_add(chunk_len, Ordering::Relaxed);
                }
                local_counts
            }));
        }

        // Main thread manages the UI loop
        if interactive {
            let mut last_drawn = usize::MAX;
            loop {
                let done = states_done.load(Ordering::Relaxed).min(total_states);
                if done != last_drawn {
                    draw_progress(
                        ProgressPhase::DpClassifying {
                            states_done: done,
                            total_states,
                        },
                        colors,
                        global_start_time,
                    );
                    last_drawn = done;
                }
                if done >= total_states {
                    break;
                }
                thread::sleep(Duration::from_millis(PROGRESS_POLL_INTERVAL_MS));
            }
            println!("\n");
        }

        // Wait for all threads to finish and aggregate the results
        let mut final_counts = Counts::default();
        for handle in handles {
            final_counts += &handle.join().unwrap();
        }

        final_counts
    }

    // ================================================================
    // BRANCH WRAPPER
    // ================================================================
    fn simulate_and_classify_branch(
        &self,
        branch_initial_state: StandingState,
        matches: &[(usize, usize)],
        branch_name: &str,
        interactive: bool,
        colors: &Colors,
        global_start_time: Instant,
    ) -> Counts {
        println!("{}{}{} ==========", colors.cyan, branch_name, colors.reset);
        let (states, total_states) = self.build_states(
            branch_initial_state,
            matches,
            interactive,
            colors,
            global_start_time,
            false,
        );
        // Print a newline so the Simulating Progress bar is saved and Classifying gets a fresh line
        println!();
        self.classify_states_parallel(states, total_states, interactive, colors, global_start_time)
    }

    // ================================================================
    // MAIN ORCHESTRATOR
    // ================================================================
    fn run(&self, initial_state: &StandingState, interactive: bool, colors: &Colors) -> AllCounts {
        if self.matches.is_empty() {
            let mut leaf = Counts::default();
            self.ranker.classify(initial_state, &mut leaf);
            let mut all = AllCounts::default();
            all.overall += &leaf;
            return all;
        }

        let global_start_time = Instant::now();
        let (a0, b0) = self.matches[0];
        let remaining_matches = &self.matches[1..];
        let mut all = AllCounts::default();

        // Run Branch A
        let mut state_a = *initial_state;
        state_a.record_win(a0);
        let counts_a = self.simulate_and_classify_branch(
            state_a,
            remaining_matches,
            "==== Branch: Team A Wins ",
            interactive,
            colors,
            global_start_time,
        );
        all.if_a_wins += &counts_a;
        all.overall += &counts_a;

        // Run Branch B
        let mut state_b = *initial_state;
        state_b.record_win(b0);
        let counts_b = self.simulate_and_classify_branch(
            state_b,
            remaining_matches,
            "==== Branch: Team B Wins ",
            interactive,
            colors,
            global_start_time,
        );
        all.if_b_wins += &counts_b;
        all.overall += &counts_b;

        // Run Branch NR (If allowed)
        if self.allow_no_results {
            let mut state_nr = *initial_state;
            state_nr.record_no_result(a0, b0);
            let counts_nr = self.simulate_and_classify_branch(
                state_nr,
                remaining_matches,
                "==== Branch: No Result ",
                interactive,
                colors,
                global_start_time,
            );
            all.if_nr += &counts_nr;
            all.overall += &counts_nr;
        }

        all
    }
}

// ================================================================
// HYBRID STRATEGY OPTIMIZER & SIMULATOR
// ================================================================

struct HybridStrategy {
    remaining: usize,
    optimal_dp_size: usize,
    free_ram_mb: f64,
    usable_ram_mb: f64,
    est_peak_ram: f64,
    est_compute_time: f64,
}

fn get_free_system_ram_mb() -> f64 {
    if let Ok(text) = std::fs::read_to_string("/proc/meminfo") {
        for line in text.lines() {
            if line.starts_with("MemAvailable:")
                && let Some(kb_str) = line.split_whitespace().nth(1)
                && let Ok(kb) = kb_str.parse::<f64>()
            {
                return kb / 1024.0;
            }
        }
    }
    2000.0 // Default to 2 GB if not on Linux or undetected
}

fn estimate_dp_cost(d: usize, base: u64) -> (f64, f64) {
    let (ram_growth, time_growth): (f64, f64) = if base == 3 {
        (1.45, 1.45)
    } else {
        (1.28, 1.18)
    };
    let diff = d as f64 - 20.0;
    let ram_mb = 10.0 * ram_growth.powf(diff);
    let time_s = 0.2 * time_growth.powf(diff);
    (ram_mb, time_s)
}

fn optimize_hybrid_strategy(remaining: usize, base: u64) -> HybridStrategy {
    let free_ram_mb = get_free_system_ram_mb();

    // SAFE RAM CHECK: If > 1.5GB, leave 1GB for the OS. If very tight, use exactly 50%.
    let usable_ram_mb = if free_ram_mb > 1500.0 {
        (free_ram_mb - 1000.0).max(free_ram_mb * 0.5)
    } else {
        free_ram_mb * 0.5
    };

    let mut optimal_dp_size = 1;
    let mut est_peak_ram = 0.0;
    let mut est_compute_time = 0.0;

    // OPTIMAL STRATEGY: Greedy Max-DP
    for d in (1..=remaining).rev() {
        let (ram_req, time_req) = estimate_dp_cost(d, base);
        if ram_req <= usable_ram_mb {
            let dfs_branches = (base as f64).powi((remaining - d) as i32);
            optimal_dp_size = d;
            est_peak_ram = ram_req;
            est_compute_time = dfs_branches * time_req;
            break;
        }
    }

    HybridStrategy {
        remaining,
        optimal_dp_size,
        free_ram_mb,
        usable_ram_mb,
        est_peak_ram,
        est_compute_time,
    }
}

#[derive(Clone)]
struct HybridSimulator {
    matches: Vec<(usize, usize)>,
    allow_no_results: bool,
    base: u64,
    dp_simulator: DpSimulator,
}

impl HybridSimulator {
    fn new(parsed: &ParsedInput, allow_no_results: bool) -> Self {
        Self {
            matches: parsed.matches.clone(),
            allow_no_results,
            base: if allow_no_results { 3 } else { 2 },
            dp_simulator: DpSimulator::new(parsed, allow_no_results),
        }
    }

    fn remaining_match_count(&self) -> usize {
        self.matches.len()
    }

    fn total_scenarios(&self) -> u64 {
        pow_u64(self.base, self.remaining_match_count())
    }

    fn build_dfs_tasks(
        &self,
        match_idx: usize,
        split_depth: usize,
        state: StandingState,
        slot: u8,
        tasks: &mut Vec<Task>,
    ) {
        if match_idx == split_depth {
            tasks.push(Task {
                next_match: match_idx,
                state,
                slot,
            });
            return;
        }
        let (a, b) = self.matches[match_idx];
        let (slot_a, slot_b, slot_nr) = if slot == SLOT_UNSET {
            (SLOT_A, SLOT_B, SLOT_NR)
        } else {
            (slot, slot, slot)
        };

        let mut sa = state;
        sa.record_win(a);
        self.build_dfs_tasks(match_idx + 1, split_depth, sa, slot_a, tasks);

        let mut sb = state;
        sb.record_win(b);
        self.build_dfs_tasks(match_idx + 1, split_depth, sb, slot_b, tasks);

        if self.allow_no_results {
            let mut snr = state;
            snr.record_no_result(a, b);
            self.build_dfs_tasks(match_idx + 1, split_depth, snr, slot_nr, tasks);
        }
    }

    fn single_pass_dp(
        &self,
        start_state: StandingState,
        remaining_matches: &[(usize, usize)],
        interactive: bool,
        colors: &Colors,
    ) -> Counts {
        let start_time = Instant::now();
        let (states, total_states) = self.dp_simulator.build_states(
            start_state,
            remaining_matches,
            interactive,
            colors,
            start_time,
            true,
        );
        println!();
        self.dp_simulator.classify_states_parallel(
            states,
            total_states,
            interactive,
            colors,
            start_time,
        )
    }

    fn run(
        &self,
        initial_state: &StandingState,
        interactive: bool,
        reporter: &Reporter,
    ) -> AllCounts {
        let remaining = self.matches.len();
        if remaining == 0 {
            return self
                .dp_simulator
                .run(initial_state, interactive, reporter.colors());
        }

        let strategy = optimize_hybrid_strategy(remaining, self.base);
        let split_depth = remaining - strategy.optimal_dp_size;

        reporter.print_hybrid_strategy(&strategy);

        if split_depth == 0 {
            println!(
                "{}Auto Optimizer: Fitting entirely in RAM. Falling back to Pure DP.{}\n",
                reporter.colors().green,
                reporter.colors().reset
            );
            return self
                .dp_simulator
                .run(initial_state, interactive, reporter.colors());
        }

        println!(
            "{}Auto Optimizer: Proceeding with Hybrid DFS-DP{}",
            reporter.colors().green,
            reporter.colors().reset
        );
        println!(
            "Splitting first {} matches via DFS. DP running on remaining {} matches.",
            split_depth, strategy.optimal_dp_size
        );
        println!();

        let global_start_time = Instant::now();
        let mut tasks = Vec::new();
        self.build_dfs_tasks(0, split_depth, *initial_state, SLOT_UNSET, &mut tasks);

        let mut final_all = AllCounts::default();

        for (idx, task) in tasks.iter().enumerate() {
            println!(
                "{bold}{cyan}==== Hybrid Task {}/{} ===={reset}",
                idx + 1,
                tasks.len(),
                bold = reporter.colors().bold,
                cyan = reporter.colors().cyan,
                reset = reporter.colors().reset
            );

            let dp_matches = &self.matches[task.next_match..];

            let result_counts =
                self.single_pass_dp(task.state, dp_matches, interactive, reporter.colors());

            let after_elapsed = global_start_time.elapsed().as_secs_f64();
            println!(
                "{green}Task {}/{} Completed | Total Elapsed: {:.1}s{reset}\n",
                idx + 1,
                tasks.len(),
                after_elapsed,
                green = reporter.colors().green,
                reset = reporter.colors().reset
            );

            // Accumulate the global total
            final_all.overall += &result_counts;

            // Route to the specific branch totals for the Next Match Impact table
            match task.slot {
                SLOT_A => final_all.if_a_wins += &result_counts,
                SLOT_B => final_all.if_b_wins += &result_counts,
                SLOT_NR => final_all.if_nr += &result_counts,
                _ => {}
            }
        }

        final_all
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
        // pos_w + team_w + 4 pct columns + 5 space separators
        self.layout.pos_w + self.layout.team_w + (4 * self.layout.pct_w) + 5
    }

    fn print_current_standings(&self, parsed: &ParsedInput) {
        let heading_text = format!(
            "========= Current Standings after Match {} =========",
            parsed.completed_matches
        );
        println!(
            "{}{:^width$}{}",
            self.colors.cyan,
            heading_text,
            self.colors.reset,
            width = self.standings_table_width()
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
        let mut initial_scores = [0u16; MAX_TEAMS];
        for (i, score) in initial_scores
            .iter_mut()
            .enumerate()
            .take(parsed.team_count)
        {
            *score = ((parsed.initial_state.score >> (i * TEAM_BITS)) & TEAM_MASK) as u16;
        }
        let current_order = sort_teams(parsed.team_count, &initial_scores);
        for (idx, &team_idx) in current_order.iter().take(parsed.team_count).enumerate() {
            println!(
                "{:>pos_w$} {}{:team_w$}{} {:>stat_w$} {:>stat_w$} {:>stat_w$} {:>stat_w$} {:>stat_w$}",
                idx + 1,
                self.colors.green,
                parsed.team_names[team_idx],
                self.colors.reset,
                parsed.matches_played[team_idx],
                parsed.initial_state.wins(team_idx),
                parsed.losses[team_idx],
                parsed.no_results[team_idx],
                parsed.initial_state.points(team_idx),
                pos_w = self.layout.pos_w,
                team_w = self.layout.team_w,
                stat_w = self.layout.stat_w,
            );
        }
    }

    fn print_simulation_header(
        &self,
        algorithm: Algorithm,
        completed_matches: usize,
        remaining_matches: usize,
        base: u64,
        total_scenarios: u64,
        num_threads: usize,
    ) {
        println!(
            "\n{}================== League Status ==================={}",
            self.colors.cyan, self.colors.reset
        );
        println!(
            "  {}Matches Completed :{} {}",
            self.colors.magenta, self.colors.reset, completed_matches
        );
        println!(
            "  {}Matches Remaining :{} {}",
            self.colors.magenta, self.colors.reset, remaining_matches
        );
        println!(
            "  {}Outcome Mode      :{} {} per match",
            self.colors.magenta, self.colors.reset, base
        );
        println!(
            "  {}Total Scenarios   :{} {}",
            self.colors.magenta,
            self.colors.reset,
            format_with_commas(total_scenarios)
        );
        println!(
            "  {}Algorithm         :{} {}",
            self.colors.magenta, self.colors.reset, algorithm
        );
        if algorithm == Algorithm::Dfs {
            println!(
                "  {}Threads           :{} {}",
                self.colors.magenta, self.colors.reset, num_threads
            );
        }
        println!();
    }

    fn print_hybrid_strategy(&self, strategy: &HybridStrategy) {
        println!(
            "{}============ Hybrid Optimizer Strategy ============={}",
            self.colors.cyan, self.colors.reset
        );
        println!(
            "  {}Free System RAM   :{} {:.0} MB (Usable: {:.0} MB)",
            self.colors.magenta, self.colors.reset, strategy.free_ram_mb, strategy.usable_ram_mb
        );
        println!(
            "  {}Optimal DP Size   :{} {} matches (DFS will split {})",
            self.colors.magenta,
            self.colors.reset,
            strategy.optimal_dp_size,
            strategy.remaining - strategy.optimal_dp_size
        );
        println!(
            "  {}Est. Compute Time :{} {:.1} seconds",
            self.colors.magenta, self.colors.reset, strategy.est_compute_time
        );
        if strategy.est_peak_ram >= 1024.0 {
            println!(
                "  {}Est. Peak RAM     :{} {:.1} GB",
                self.colors.magenta,
                self.colors.reset,
                strategy.est_peak_ram / 1024.0
            );
        } else {
            println!(
                "  {}Est. Peak RAM     :{} {:.0} MB",
                self.colors.magenta, self.colors.reset, strategy.est_peak_ram
            );
        }
        println!();
    }

    fn print_current_probabilities_heading(&self, parsed: &ParsedInput) {
        let heading_text = format!(
            "========= Current Probabilities after Match {} =========",
            parsed.completed_matches
        );
        println!(
            "{}{:^width$}{}",
            self.colors.cyan,
            heading_text,
            self.colors.reset,
            width = self.probabilities_table_width()
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
            println!(
                "{}{:^width$}{}\n",
                self.colors.magenta,
                heading_text,
                self.colors.reset,
                width = self.probabilities_table_width()
            );
        }
    }

    fn print_next_match_scenario_heading(&self, title: &str) {
        let heading_text = format!("========= {} =========", title);
        println!(
            "{}{:^width$}{}",
            self.colors.cyan,
            heading_text,
            self.colors.reset,
            width = self.probabilities_table_width()
        );
    }

    fn print_results(
        &self,
        parsed: &ParsedInput,
        counts: &Counts,
        total_scenarios: u64,
        seat_scale: u64,
    ) {
        let mut rows: Vec<Row> = (0..parsed.team_count)
            .map(|i| Row {
                team: parsed.team_names[i].clone(),
                top2_pts: counts.top2_pts[i],
                top2_good_nrr_units: counts.top2_good_nrr_units[i],
                top4_pts: counts.top4_pts[i],
                top4_good_nrr_units: counts.top4_good_nrr_units[i],
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
            "{}{:>pos_w$} {:team_w$} {:>pct_w$} {:>pct_w$} {:>pct_w$} {:>pct_w$}{}",
            self.colors.yellow,
            "",
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

    fn print_probability_results(
        &self,
        parsed: &ParsedInput,
        all_counts: &AllCounts,
        total_scenarios: u64,
        base: u64,
        allow_no_results: bool,
    ) {
        self.print_current_probabilities_heading(parsed);
        self.print_results(
            parsed,
            &all_counts.overall,
            total_scenarios,
            parsed.seat_scale,
        );

        if !parsed.matches.is_empty() {
            let (a, b) = parsed.matches[0];
            let cond_total = total_scenarios / base;

            println!();
            self.print_next_match_impact_heading(parsed);

            let title_a = format!("If {} beats {}", parsed.team_names[a], parsed.team_names[b]);
            self.print_next_match_scenario_heading(&title_a);
            self.print_results(parsed, &all_counts.if_a_wins, cond_total, parsed.seat_scale);
            println!();

            let title_b = format!("If {} beats {}", parsed.team_names[b], parsed.team_names[a]);
            self.print_next_match_scenario_heading(&title_b);
            self.print_results(parsed, &all_counts.if_b_wins, cond_total, parsed.seat_scale);
            println!();

            if allow_no_results {
                let title_nr = format!(
                    "If {} vs {} ends in NR",
                    parsed.team_names[a], parsed.team_names[b]
                );
                self.print_next_match_scenario_heading(&title_nr);
                self.print_results(parsed, &all_counts.if_nr, cond_total, parsed.seat_scale);
                println!();
            }
        }
    }
}

// ================================================================
// SIMULATION RUNNER
// ================================================================

enum SimulationRunner {
    Hybrid(HybridSimulator),
    Dfs(DfsSimulator),
    Dp(DpSimulator),
}

impl SimulationRunner {
    fn run_simulation(
        &self,
        algorithm: Algorithm,
        parsed: &ParsedInput,
        cli: &CliArgs,
        reporter: &Reporter,
        num_threads: usize,
        interactive: bool,
    ) -> Result<(), AppError> {
        let (remaining, base_val, total_scenarios) = match self {
            SimulationRunner::Hybrid(h) => (h.remaining_match_count(), h.base, h.total_scenarios()),
            SimulationRunner::Dfs(d) => (d.remaining_match_count(), d.base, d.total_scenarios()),
            SimulationRunner::Dp(d) => (d.remaining_match_count(), d.base, d.total_scenarios()),
        };

        check_u64_overflow(parsed.seat_scale, remaining, base_val);
        reporter.print_current_standings(parsed);
        reporter.print_simulation_header(
            algorithm,
            parsed.completed_matches,
            remaining,
            base_val,
            total_scenarios,
            if matches!(algorithm, Algorithm::Dp) {
                1
            } else {
                num_threads
            },
        );

        if remaining == 0 {
            println!("No remaining matches to simulate.");
            return Ok(());
        }

        let all_counts = match self {
            SimulationRunner::Hybrid(h) => h.run(&parsed.initial_state, interactive, reporter),
            SimulationRunner::Dfs(d) => d.run(
                &parsed.initial_state,
                num_threads,
                interactive,
                reporter.colors(),
            ),
            SimulationRunner::Dp(d) => d.run(&parsed.initial_state, interactive, reporter.colors()),
        };

        reporter.print_probability_results(
            parsed,
            &all_counts,
            total_scenarios,
            base_val,
            cli.allow_no_results,
        );

        Ok(())
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
        format!(
            "{:.2}%",
            (units as f64) * 100.0 / ((total_scenarios as f64) * (seat_scale as f64))
        )
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

/// Exits with a friendly message if the scenario count would overflow u64 counters.
fn check_u64_overflow(seat_scale: u64, remaining: usize, base: u64) {
    let total_scenarios = pow_u64(base, remaining);
    if total_scenarios != u64::MAX && total_scenarios.checked_mul(seat_scale).is_some() {
        return;
    }

    let safe_for_base = |b: u64| -> u64 {
        let mut m = 0u64;
        let mut v = seat_scale;
        while let Some(next) = v.checked_mul(b) {
            v = next;
            m += 1;
        }
        m
    };

    println!(
        "Too many scenarios to compute safely with {} matches remaining.\n\
         \n\
         Maximum supported remaining matches:\n\
           Without --allow-no-results : {}\n\
           With    --allow-no-results : {}\n\
         \n\
         Try after more matches are completed.",
        remaining,
        safe_for_base(2),
        safe_for_base(3),
    );
    std::process::exit(0);
}

fn run() -> Result<(), AppError> {
    let cli = parse_args()?;
    let interactive = io::stdout().is_terminal();
    let matches_input = read_matches_file(&cli.file_path)?;
    let parsed = parse_inputs(&matches_input)?;
    let reporter = Reporter::new(&parsed, interactive);
    let num_threads = determine_num_threads();
    let runner = match cli.algorithm {
        Algorithm::Auto => {
            SimulationRunner::Hybrid(HybridSimulator::new(&parsed, cli.allow_no_results))
        }
        Algorithm::Dfs => SimulationRunner::Dfs(DfsSimulator::new(&parsed, cli.allow_no_results)),
        Algorithm::Dp => SimulationRunner::Dp(DpSimulator::new(&parsed, cli.allow_no_results)),
    };
    runner.run_simulation(
        cli.algorithm,
        &parsed,
        &cli,
        &reporter,
        num_threads,
        interactive,
    )?;
    Ok(())
}

fn main() {
    let colors = Colors::new(io::stderr().is_terminal());
    if let Err(e) = run() {
        eprintln!("{}Error:{} {}", colors.yellow, colors.reset, e);
        std::process::exit(1);
    }
}
