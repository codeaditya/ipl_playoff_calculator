use std::collections::HashMap;
use std::env;
use std::fmt;
use std::fs;
use std::future::Future;
use std::io::{self, IsTerminal, Write};
use std::ops::AddAssign;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
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

/// Returns u64::MAX on overflow — callers use check_u64_overflow() as the gatekeeper.
fn pow_u64(base: u64, exp: usize) -> u64 {
    (0..exp)
        .try_fold(1u64, |acc, _| acc.checked_mul(base))
        .unwrap_or(u64::MAX)
}

// ================================================================
// ASYNC HELPER — no external crates
// ================================================================
//
// wgpu's futures resolve synchronously once the device is polled;
// they do not need a reactor. A simple spin-poll loop is sufficient
// and safe for all wgpu async calls.

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = Box::pin(f);

    static VTABLE: RawWakerVTable = RawWakerVTable::new(
        |_| RawWaker::new(std::ptr::null(), &VTABLE), // clone
        |_| (),                                       // wake
        |_| (),                                       // wake_by_ref
        |_| (),                                       // drop
    );
    let raw = RawWaker::new(std::ptr::null(), &VTABLE);
    // Safety: vtable functions are all no-ops; valid for a spin-poll loop.
    let waker = unsafe { Waker::from_raw(raw) };
    let mut cx = Context::from_waker(&waker);

    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => {}
        }
    }
}

// ================================================================
// CLI
// ================================================================

struct CliArgs {
    file_path: String,
    allow_no_results: bool,
    use_gpu: bool,
}

fn parse_args() -> Result<CliArgs, AppError> {
    let mut args = env::args();
    let program_name = args
        .next()
        .unwrap_or_else(|| "ipl-playoff-calculator".to_string());

    let interactive = io::stderr().is_terminal();
    let colors = Colors::new(interactive);

    let mut file_path: Option<String> = None;
    let mut allow_no_results = false;
    let mut use_gpu = false;

    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => {
                print_usage(&program_name, &colors);
                std::process::exit(0);
            }
            "--allow-no-results" => {
                allow_no_results = true;
            }
            "--gpu" => {
                use_gpu = true;
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
        use_gpu,
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
        "{bold}{yellow}Usage:{reset} {} [--allow-no-results] [--gpu] <matches-file>",
        program_name,
        bold = c.bold,
        yellow = c.yellow,
        reset = c.reset
    );
    eprintln!("\n{bold}Arguments:{reset}", bold = c.bold, reset = c.reset);
    eprintln!("  <matches-file>       Path to the text file containing the schedule.");
    eprintln!(
        "  --allow-no-results   (Optional) Include ties/washouts (1 pt each) in future outcomes."
    );
    eprintln!(
        "  --gpu                (Optional) Run simulation on GPU via wgpu (Vulkan/DX12/Metal)."
    );
    eprintln!(
        "\n{bold}Matches File Format:{reset}",
        bold = c.bold,
        reset = c.reset
    );
    eprintln!("  - One match per line. Lines starting with '#' are ignored.");
    eprintln!(
        "  - {bold}Upcoming:{reset}  Team A vs Team B",
        bold = c.bold,
        reset = c.reset
    );
    eprintln!(
        "  - {bold}Completed:{reset} Team A vs Team B : Winner",
        bold = c.bold,
        reset = c.reset
    );
    eprintln!(
        "  - {bold}No Result:{reset} Team A vs Team B : NR",
        bold = c.bold,
        reset = c.reset
    );
    eprintln!("\n{bold}Example:{reset}", bold = c.bold, reset = c.reset);
    eprintln!("  CSK vs RCB : CSK");
    eprintln!("  MI vs DC\n");
}

// ================================================================
// DATA MODELS
// ================================================================

const WIN_SCORE_DELTA: u16 = (2 << 8) | 1; // 2 points, 1 win
const NR_SCORE_DELTA: u16 = (1 << 8) | 0; // 1 point, 0 wins

#[derive(Clone, Copy, Default, Debug)]
struct StandingState {
    score: [u16; MAX_TEAMS],
}

impl StandingState {
    #[inline]
    fn record_win(&mut self, team: usize) {
        self.score[team] += WIN_SCORE_DELTA;
    }
    #[inline]
    fn undo_win(&mut self, team: usize) {
        self.score[team] -= WIN_SCORE_DELTA;
    }
    #[inline]
    fn record_no_result(&mut self, a: usize, b: usize) {
        self.score[a] += NR_SCORE_DELTA;
        self.score[b] += NR_SCORE_DELTA;
    }
    #[inline]
    fn undo_no_result(&mut self, a: usize, b: usize) {
        self.score[a] -= NR_SCORE_DELTA;
        self.score[b] -= NR_SCORE_DELTA;
    }
    #[inline]
    fn points(&self, team: usize) -> u8 {
        (self.score[team] >> 8) as u8
    }
    #[inline]
    fn wins(&self, team: usize) -> u8 {
        (self.score[team] & 0xFF) as u8
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
    slot: u8,
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
                .map(|n| n.len())
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
        let order = sort_teams(self.team_count, &state.score);
        let mut start = 0;
        let mut placed_above = 0;

        while start < self.team_count {
            let mut end = start + 1;
            let score_val = state.score[order[start]];
            while end < self.team_count && state.score[order[end]] == score_val {
                end += 1;
            }

            let group_len = end - start;
            let group_len_u64 = group_len as u64;

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

            for idx in start..end {
                let team = order[idx];
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

fn sort_teams(team_count: usize, score: &[u16; MAX_TEAMS]) -> [usize; MAX_TEAMS] {
    let mut order = [0usize; MAX_TEAMS];
    for i in 0..team_count {
        order[i] = i;
    }
    for i in 1..team_count {
        let key = order[i];
        let mut j = i;
        while j > 0 {
            let prev = order[j - 1];
            if score[prev] >= score[key] {
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
// SIMULATION
// ================================================================

#[derive(Clone)]
struct Simulator {
    matches: Arc<Vec<(usize, usize)>>,
    ranker: Ranker,
    allow_no_results: bool,
    pub base: u64,
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
            _ => {}
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
}

// ================================================================
// GPU SIMULATION
// ================================================================
//
// Architecture:
//   - Each GPU thread handles exactly ONE scenario, identified by a
//     64-bit index encoded as (idx_hi: u32, idx_lo: u32).
//   - The scenario index is decoded digit-by-digit in base 2 or 3
//     using emulated 64-bit division (hi:lo / base) in the shader,
//     since WGSL has no native u64 type.
//   - Output is 4 × Counts structs (overall, if_a, if_b, if_nr),
//     each holding 4 arrays of 10 u64s — stored as u32 hi/lo pairs
//     in the storage buffer (80 u32s per Counts, 320 u32s total).
//   - Atomic adds accumulate results from all threads.
//   - slot routing (overall / if_a / if_b / if_nr) mirrors the CPU
//     task slot logic: the first match's outcome determines the slot.
//
// Buffer layout (all u32, std430):
//   Uniforms (read-only):
//     [0]     num_matches          (u32)
//     [1]     base                 (u32)  — 2 or 3
//     [2]     total_scenarios_lo   (u32)  — low  32 bits of total_scenarios
//     [3]     total_scenarios_hi   (u32)  — high 32 bits of total_scenarios
//     [4]     team_count           (u32)
//     [5]     seat_scale_lo        (u32)
//     [6]     seat_scale_hi        (u32)
//     [7]     _pad                 (u32)
//     [8..17]  initial_scores[10]  (u32 each — u16 score widened)
//     [18..17+MAX_MATCHES] match_a[i] (u32)
//     [18+MAX_MATCHES..17+2*MAX_MATCHES] match_b[i] (u32)
//
//   Output (atomic u32, std430):
//     4 Counts structs × 4 arrays × 10 teams × 2 u32 (hi/lo) = 320 u32s
//     Layout: [overall.top2_pts_lo[10], overall.top2_pts_hi[10],
//              overall.top2_nrr_lo[10], overall.top2_nrr_hi[10],
//              overall.top4_pts_lo[10], overall.top4_pts_hi[10],
//              overall.top4_nrr_lo[10], overall.top4_nrr_hi[10],
//              if_a_wins × same 8 arrays,
//              if_b_wins × same 8 arrays,
//              if_nr     × same 8 arrays]

const MAX_MATCHES: usize = 74; // IPL has at most 74 matches per season
const WORKGROUP_SIZE: u32 = 256;

// WGSL shader source.
// We use a const string rather than include_str! so the binary is self-contained.
fn wgsl_shader() -> &'static str {
    r#"
// ── Uniform scalars (16-byte-aligned fields only) ────────────────
struct Uniforms {
    num_matches : u32,
    base : u32,
    task_total_lo : u32,
    task_total_hi : u32,
    team_count : u32,
    seat_scale_lo : u32,
    seat_scale_hi : u32,
    task_offset_lo : u32,
    task_offset_hi : u32,
    M : u32,
    scenarios_per_task : u32,
    _pad : u32,
}

// ── Storage buffer: arrays (4-byte stride OK in storage) ──────────
struct Arrays {
    initial_scores : array<u32, 10>,
    match_a : array<u32, 74>,
    match_b : array<u32, 74>,
}

@group(0) @binding(0) var<uniform> uni : Uniforms;
@group(0) @binding(1) var<storage, read> arrs : Arrays;
@group(0) @binding(2) var<storage, read_write> out : array<atomic<u32>>;

// SHARED WORKGROUP MEMORY (Eliminates DDR4 global memory bottlenecks on APUs)
var<workgroup> local_out: array<atomic<u32>, 320>;

// ── u64 helpers ───────────────────────────────────────────────────
fn u64_divmod(hi: ptr<function, u32>, lo: ptr<function, u32>, divisor: u32) -> u32 {
    let hi_q = *hi / divisor;
    let hi_r = *hi % divisor;
    let mid = (hi_r << 16u) | (*lo >> 16u);
    let mid_q = mid / divisor;
    let mid_r = mid % divisor;
    let lo_c = (mid_r << 16u) | (*lo & 0xFFFFu);
    let lo_q = lo_c / divisor;
    let rem = lo_c % divisor;
    *hi = hi_q;
    *lo = (mid_q << 16u) | lo_q;
    return rem;
}

fn u64_ge(a_hi: u32, a_lo: u32, b_hi: u32, b_lo: u32) -> bool {
    if a_hi != b_hi { return a_hi > b_hi; }
    return a_lo >= b_lo;
}

fn u64_mul_carry(a: u32, b: u32) -> u32 {
    let a_lo = a & 0xFFFFu; let a_hi = a >> 16u;
    let b_lo = b & 0xFFFFu; let b_hi = b >> 16u;
    let mid1 = a_lo * b_hi; let mid2 = a_hi * b_lo;
    let lo = a_lo * b_lo;
    let carry_from_lo = (lo >> 16u) + (mid1 & 0xFFFFu) + (mid2 & 0xFFFFu);
    return a_hi * b_hi + (mid1 >> 16u) + (mid2 >> 16u) + (carry_from_lo >> 16u);
}

// ── Branchless Classify and accumulate ─────────────────────────────
fn classify_and_accumulate(scores: array<u32, 10>, slot: u32) {
    // 1. Pack scores and team IDs into a single integer
    var p: array<u32, 10>;
    for (var i = 0u; i < 10u; i++) {
        p[i] = (scores[i] << 4u) | i;
    }

    // 2. Branchless Sorting Network (compiles to fast hardware min/max instructions)
    for (var i = 0u; i < 10u; i++) {
        for (var j = i + 1u; j < 10u; j++) {
            let a = p[i]; let b = p[j];
            p[i] = max(a, b);
            p[j] = min(a, b);
        }
    }

    let team_count = uni.team_count;
    var start = 0u; var placed_above = 0u;

    // 3. Classify groups
    while start < team_count {
        var end = start + 1u;
        let score_val = p[start] >> 4u;
        while end < team_count && (p[end] >> 4u) == score_val { end++; }
        let group_len = end - start;

        // Top-2
        var spots2 = 0u;
        if placed_above < 2u { spots2 = min(2u - placed_above, group_len); }
        var ut2_lo = 0u; var ut2_hi = 0u;
        if spots2 > 0u {
            let mul_lo = spots2 * uni.seat_scale_lo;
            let carry = u64_mul_carry(spots2, uni.seat_scale_lo);
            let mul_hi = spots2 * uni.seat_scale_hi + carry;
            var th = mul_hi; var tl = mul_lo;
            u64_divmod(&th, &tl, group_len);
            ut2_lo = tl; ut2_hi = th;
        }

        // Top-4
        var spots4 = 0u;
        if placed_above < 4u { spots4 = min(4u - placed_above, group_len); }
        var ut4_lo = 0u; var ut4_hi = 0u;
        if spots4 > 0u {
            let mul_lo = spots4 * uni.seat_scale_lo;
            let carry = u64_mul_carry(spots4, uni.seat_scale_lo);
            let mul_hi = spots4 * uni.seat_scale_hi + carry;
            var th = mul_hi; var tl = mul_lo;
            u64_divmod(&th, &tl, group_len);
            ut4_lo = tl; ut4_hi = th;
        }

        // Write to LOCAL Shared Memory instead of global
        for (var s = 0u; s <= 1u; s++) {
            let write_slot = select(slot, 0u, s == 0u);
            if s == 1u && slot == 0u { continue; }
            let b = write_slot * 80u;

            for (var idx = start; idx < end; idx++) {
                let t = p[idx] & 0xFu; // Extract the team ID back out
                if ut2_lo != 0u || ut2_hi != 0u {
                    let prev2 = atomicAdd(&local_out[b + 20u + t], ut2_lo);
                    atomicAdd(&local_out[b + 30u + t], ut2_hi + u32(prev2 + ut2_lo < prev2));
                }
                if spots2 == group_len { atomicAdd(&local_out[b + 0u + t], 1u); }
                if ut4_lo != 0u || ut4_hi != 0u {
                    let prev4 = atomicAdd(&local_out[b + 60u + t], ut4_lo);
                    atomicAdd(&local_out[b + 70u + t], ut4_hi + u32(prev4 + ut4_lo < prev4));
                }
                if spots4 == group_len { atomicAdd(&local_out[b + 40u + t], 1u); }
            }
        }
        start = end;
        placed_above += group_len;
    }
}

// ── Main Compute Kernel ───────────────────────────────────────────
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) lid: u32) {
    // 1. Initialize local memory
    for (var i = lid; i < 320u; i += 256u) {
        atomicStore(&local_out[i], 0u);
    }
    workgroupBarrier();

    var task_lo = uni.task_offset_lo + gid.x;
    var task_hi = uni.task_offset_hi + u32(task_lo < uni.task_offset_lo);

    // Verify task bounds (do not return early, must hit final barrier)
    if !u64_ge(task_hi, task_lo, uni.task_total_hi, uni.task_total_lo) {
        var base_scores: array<u32, 10>;
        for (var t = 0u; t < 10u; t++) { base_scores[t] = arrs.initial_scores[t]; }

        var base_slot = 0u;
        var slot_set = false;
        let outer_matches = uni.num_matches - uni.M;

        // 1. Calculate top-of-tree once
        for (var i = 0u; i < outer_matches; i++) {
            var outcome: u32;
            if uni.base == 2u {
                outcome = task_lo & 1u;
                task_lo = (task_lo >> 1u) | ((task_hi & 1u) << 31u);
                task_hi = task_hi >> 1u;
            } else {
                outcome = u64_divmod(&task_hi, &task_lo, uni.base);
            }

            let a = arrs.match_a[i];
            let b = arrs.match_b[i];
            if outcome == 0u {
                base_scores[a] += 0x0201u;
            } else if outcome == 1u {
                base_scores[b] += 0x0201u;
            } else {
                base_scores[a] += 0x0100u;
                base_scores[b] += 0x0100u;
            }

            if !slot_set {
                base_slot = outcome + 1u;
                slot_set = true;
            }
        }

        // 2. Pre-fetch inner matches into registers! (Massive bandwidth saving)
        var inner_a: array<u32, 6>;
        var inner_b: array<u32, 6>;
        for (var i = 0u; i < uni.M; i++) {
            inner_a[i] = arrs.match_a[outer_matches + i];
            inner_b[i] = arrs.match_b[outer_matches + i];
        }

        // 3. Fast native 32-bit loop with register caching
        for (var s = 0u; s < uni.scenarios_per_task; s++) {
            var scores = base_scores;
            var slot = base_slot;
            var inner_slot_set = slot_set;
            var temp_s = s;

            for (var i = 0u; i < uni.M; i++) {
                let outcome = temp_s % uni.base;
                temp_s /= uni.base;

                let a = inner_a[i];
                let b = inner_b[i];
                if outcome == 0u {
                    scores[a] += 0x0201u;
                } else if outcome == 1u {
                    scores[b] += 0x0201u;
                } else {
                    scores[a] += 0x0100u;
                    scores[b] += 0x0100u;
                }

                if !inner_slot_set {
                    slot = outcome + 1u;
                    inner_slot_set = true;
                }
            }
            classify_and_accumulate(scores, slot);
        }
    }

    // 4. Flush local accumulation out to global memory WITH CARRY PROPAGATION
    workgroupBarrier();

    // There are 160 (low, high) pairs in the 320-item buffer.
    // We map each thread to a specific pair to safely propagate the carry bit.
    for (var p = lid; p < 160u; p += 256u) {
        let block = p / 40u;          // 4 outcome blocks (overall, if_a, if_b, if_nr)
        let rem_p = p % 40u;
        let group = rem_p / 10u;      // 4 stats (top2_pts, top2_nrr, top4_pts, top4_nrr)
        let team  = rem_p % 10u;      // 10 teams

        // Calculate exact indices for the low and high chunks of this specific stat
        let lo_idx = block * 80u + group * 20u + team;
        let hi_idx = lo_idx + 10u;

        let val_lo = atomicLoad(&local_out[lo_idx]);
        let val_hi = atomicLoad(&local_out[hi_idx]);

        if val_lo > 0u || val_hi > 0u {
            // Add low and calculate if it caused a global overflow
            let prev_lo = atomicAdd(&out[lo_idx], val_lo);
            let carry = u32(prev_lo + val_lo < prev_lo);

            // Add high + the calculated global carry
            atomicAdd(&out[hi_idx], val_hi + carry);
        }
    }
}
"#
}

// GPU context holds the device and queue, which are Clone in wgpu 24+.
struct GpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl GpuContext {
    fn try_init() -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });

        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok()?;

        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ipl-gpu"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .ok()?;

        Some(Self { device, queue })
    }
}

// Output buffer: 4 Counts × 80 u32s = 320 u32s.
// Index helpers for the flat output buffer.
// Layout per Counts block (80 u32s):
//   [0..9]   top2_pts_lo      [10..19] top2_pts_hi
//   [20..29] top2_nrr_lo      [30..39] top2_nrr_hi
//   [40..49] top4_pts_lo      [50..59] top4_pts_hi
//   [60..69] top4_nrr_lo      [70..79] top4_nrr_hi
const COUNTS_BLOCK_U32S: usize = 80;
const OUT_BUFFER_U32S: usize = 4 * COUNTS_BLOCK_U32S; // 320

fn read_counts_from_buf(buf: &[u32], block: usize) -> Counts {
    let b = block * COUNTS_BLOCK_U32S;
    let mut c = Counts::default();
    for t in 0..MAX_TEAMS {
        c.top2_pts[t] = (buf[b + 10 + t] as u64) << 32 | buf[b + 0 + t] as u64;
        c.top2_good_nrr_units[t] = (buf[b + 30 + t] as u64) << 32 | buf[b + 20 + t] as u64;
        c.top4_pts[t] = (buf[b + 50 + t] as u64) << 32 | buf[b + 40 + t] as u64;
        c.top4_good_nrr_units[t] = (buf[b + 70 + t] as u64) << 32 | buf[b + 60 + t] as u64;
    }
    c
}

fn simulate_all_gpu(
    ctx: &GpuContext,
    parsed: &ParsedInput,
    simulator: &Simulator,
    total_scenarios: u64,
    interactive: bool,
    colors: &Colors,
) -> AllCounts {
    let num_matches = simulator.remaining_match_count();
    let device = &ctx.device;
    let queue = &ctx.queue;

    // --- Dynamic task splitting ---
    // Let each GPU thread handle up to 6 internal matches (e.g., 3^6 = 729 scenarios)
    let m = num_matches.min(6);
    let scenarios_per_task = (simulator.base as u64).pow(m as u32);
    let task_total = total_scenarios / scenarios_per_task;

    // ── Uniform buffer: 12 scalar u32s (16-byte aligned) ────
    let mut uniform_data = [0u32; 12];
    uniform_data[0] = num_matches as u32;
    uniform_data[1] = simulator.base as u32;
    uniform_data[2] = task_total as u32;
    uniform_data[3] = (task_total >> 32) as u32;
    uniform_data[4] = parsed.team_count as u32;
    uniform_data[5] = parsed.seat_scale as u32;
    uniform_data[6] = (parsed.seat_scale >> 32) as u32;
    // Indices 7 and 8 are task_offset_lo/hi, populated dynamically below
    uniform_data[9] = m as u32;
    uniform_data[10] = scenarios_per_task as u32;
    uniform_data[11] = 0; // padding

    let uniform_bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(uniform_data.as_ptr() as *const u8, uniform_data.len() * 4)
    };
    let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("uniforms"),
        size: uniform_bytes.len() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&uniform_buf, 0, uniform_bytes);

    // ── Arrays buffer: initial_scores + match_a + match_b (storage, 4-byte stride OK)
    let arrays_u32s = 10 + MAX_MATCHES + MAX_MATCHES;
    let mut arrays_data = vec![0u32; arrays_u32s];
    for t in 0..MAX_TEAMS {
        arrays_data[t] = parsed.initial_state.score[t] as u32;
    }
    for (i, &(a, b)) in simulator.matches.iter().enumerate() {
        arrays_data[10 + i] = a as u32;
        arrays_data[10 + MAX_MATCHES + i] = b as u32;
    }
    let arrays_bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(arrays_data.as_ptr() as *const u8, arrays_data.len() * 4)
    };
    let arrays_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("arrays"),
        size: arrays_bytes.len() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&arrays_buf, 0, arrays_bytes);

    // ── GPU buffers ──────────────────────────────────────────────

    let out_buf_size = (OUT_BUFFER_U32S * 4) as u64;
    let out_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("output"),
        size: out_buf_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let readback_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: out_buf_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    // ── Pipeline ─────────────────────────────────────────────────
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("sim_shader"),
        source: wgpu::ShaderSource::Wgsl(wgsl_shader().into()),
    });

    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("pl"),
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });

    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("sim_pipeline"),
        layout: Some(&pipeline_layout),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("bg"),
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: arrays_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: out_buf.as_entire_binding(),
            },
        ],
    });

    // ── Dispatch ─────────────────────────────────────────────────
    // We use a 2D dispatch: x covers the low 32 bits of the scenario index,
    // y covers the high 32 bits. Each thread handles one scenario.
    // For total_scenarios <= 2^32 we only need y=1.
    // For total_scenarios > 2^32 (e.g. base=2, N>32) we need y>1.
    //
    // wgpu's max dispatch per dimension is 65535. Since our overflow check
    // ensures total_scenarios fits in u64 and the seat_scale bound limits
    // us well below 2^64, this covers all reachable scenario counts.
    //
    // We divide the scenario space into chunks of WORKGROUP_SIZE * 65535
    // along the x axis, iterating y from 0 upward.

    let start_time = Instant::now();

    // Total threads needed = total_scenarios (each thread = one scenario).
    // Dispatch grid: dispatch_x * WORKGROUP_SIZE threads per y-row.
    // We use fixed dispatch_x = 65535 (maximum), so each y-row covers
    // 65535 * WORKGROUP_SIZE scenarios. We calculate how many y rows needed.
    // Each batch covers ROWS_PER_BATCH Y-rows. The shader uses uni.y_offset
    // to know which absolute row gid.y=0 corresponds to, so each batch
    // processes a distinct, non-overlapping slice of the scenario space.
    // ROWS_PER_BATCH is kept small enough that each dispatch finishes well
    // within the GPU driver's TDR (Timeout Detection & Recovery) limit.

    // TDR-Safe Chunking: Process roughly 100 million scenarios per dispatch
    let target_scenarios_per_batch = 100_000_000u64;
    let target_tasks = (target_scenarios_per_batch / scenarios_per_task).max(256);

    // Cap at the hardware limit of 65535 workgroups per dimension
    let workgroups_per_batch = (target_tasks / WORKGROUP_SIZE as u64).min(65535) as u32;
    let actual_tasks_per_batch = (workgroups_per_batch * WORKGROUP_SIZE) as u64;
    let total_batches = task_total.div_ceil(actual_tasks_per_batch);

    if interactive {
        print!(
            "\r{}GPU Progress:{} [  0.0%] | {}Elapsed:{} {:>5.1}s | {}ETA:{} {:>5.1}s   ",
            colors.cyan,
            colors.reset,
            colors.yellow,
            colors.reset,
            0.0_f64,
            colors.green,
            colors.reset,
            0.0_f64,
        );
        io::stdout().flush().unwrap();
    }

    for batch in 0..total_batches {
        let task_offset = batch * actual_tasks_per_batch;
        let tasks_this_batch = (task_total - task_offset).min(actual_tasks_per_batch);
        let workgroups_this_batch = tasks_this_batch.div_ceil(WORKGROUP_SIZE as u64) as u32;

        // Pass the 64-bit task offset to the shader
        let offset_lo = task_offset as u32;
        let offset_hi = (task_offset >> 32) as u32;
        queue.write_buffer(&uniform_buf, 7 * 4, &offset_lo.to_ne_bytes());
        queue.write_buffer(&uniform_buf, 8 * 4, &offset_hi.to_ne_bytes());

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("enc") });

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("sim_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);

        // Dispatch 1D grid
        pass.dispatch_workgroups(workgroups_this_batch, 1, 1);
        drop(pass); // Drop the pass explicitly before submitting

        queue.submit(std::iter::once(encoder.finish()));
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .unwrap();

        if interactive {
            let tasks_done = task_offset + tasks_this_batch;
            let scenarios_done = tasks_done * scenarios_per_task;
            let pct = scenarios_done as f64 / total_scenarios as f64 * 100.0;
            let elapsed = start_time.elapsed().as_secs_f64();
            let eta = if scenarios_done > 0 {
                elapsed / scenarios_done as f64 * (total_scenarios - scenarios_done) as f64
            } else {
                0.0
            };
            print!(
                "\r{}GPU Progress:{} [{:>5.1}%] | {}Elapsed:{} {:>5.1}s | {}ETA:{} {:>5.1}s   ",
                colors.cyan,
                colors.reset,
                pct,
                colors.yellow,
                colors.reset,
                elapsed,
                colors.green,
                colors.reset,
                eta,
            );
            io::stdout().flush().unwrap();
        }
    }

    if interactive {
        println!("\n");
    }

    // ── Copy output to readback buffer and map ───────────────────
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("copy"),
    });
    encoder.copy_buffer_to_buffer(&out_buf, 0, &readback_buf, 0, out_buf_size);
    queue.submit(std::iter::once(encoder.finish()));

    let (tx, rx) = std::sync::mpsc::channel();
    readback_buf
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    rx.recv().unwrap().unwrap();

    let raw: Vec<u32> = {
        let view = readback_buf.slice(..).get_mapped_range();
        let words: &[u32] =
            unsafe { std::slice::from_raw_parts(view.as_ptr() as *const u32, OUT_BUFFER_U32S) };
        words.to_vec()
    };
    readback_buf.unmap();

    // ── Reconstruct AllCounts from raw u32 buffer ─────────────────
    // Block 0 = overall, 1 = if_a_wins, 2 = if_b_wins, 3 = if_nr
    AllCounts {
        overall: read_counts_from_buf(&raw, 0),
        if_a_wins: read_counts_from_buf(&raw, 1),
        if_b_wins: read_counts_from_buf(&raw, 2),
        if_nr: read_counts_from_buf(&raw, 3),
    }
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
            (elapsed / done as f64) * (self.total_scenarios - done) as f64
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
// PARALLEL CPU EXECUTION
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
        self.layout.pos_w + self.layout.team_w + (5 * self.layout.stat_w) + 6
    }
    fn probabilities_table_width(&self) -> usize {
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
            stat_w = self.layout.stat_w
        );
        let current_order = sort_teams(parsed.team_count, &parsed.initial_state.score);
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
                stat_w = self.layout.stat_w
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
        use_gpu: bool,
    ) {
        println!(
            "\n{}========= League Status ========={}",
            self.colors.cyan, self.colors.reset
        );
        println!(
            " {}Matches Completed :{} {}",
            self.colors.magenta, self.colors.reset, completed_matches
        );
        println!(
            " {}Matches Remaining :{} {}",
            self.colors.magenta, self.colors.reset, remaining_matches
        );
        println!(
            " {}Outcome Mode      :{} {} per match",
            self.colors.magenta, self.colors.reset, base
        );
        println!(
            " {}Total Scenarios   :{} {}",
            self.colors.magenta,
            self.colors.reset,
            format_with_commas(total_scenarios)
        );
        if use_gpu {
            println!(
                " {}Compute           :{} GPU (wgpu)",
                self.colors.magenta, self.colors.reset
            );
        } else {
            println!(
                " {}Threads           :{} {}",
                self.colors.magenta, self.colors.reset, num_threads
            );
        }
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
            pct_w = self.layout.pct_w
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
                pct_w = self.layout.pct_w
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
        "Too many scenarios to compute safely with {} matches remaining.\n\n\
        Maximum supported remaining matches:\n\
        Without --allow-no-results : {}\n\
        With --allow-no-results    : {}\n\n\
        Try after more matches are completed.",
        remaining,
        safe_for_base(2),
        safe_for_base(3)
    );
    std::process::exit(0);
}

fn simulate_all(
    parsed: &ParsedInput,
    simulator: &Simulator,
    num_threads: usize,
    interactive: bool,
    colors: &Colors,
    total_scenarios: u64,
    gpu_ctx: Option<&GpuContext>,
) -> AllCounts {
    if simulator.remaining_match_count() == 0 {
        let mut counts = Counts::default();
        simulator
            .ranker
            .classify(&parsed.initial_state, &mut counts);
        let mut all = AllCounts::default();
        all.overall += &counts;
        return all;
    }

    if let Some(ctx) = gpu_ctx {
        return simulate_all_gpu(ctx, parsed, simulator, total_scenarios, interactive, colors);
    }

    // CPU path — unchanged
    let split_depth = simulator.choose_split_depth(num_threads);
    let tasks = simulator.build_tasks(split_depth, parsed.initial_state);
    let scenarios_per_task = simulator.scenarios_per_task(split_depth);
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
    let num_threads = determine_num_threads();

    check_u64_overflow(
        parsed.seat_scale,
        simulator.remaining_match_count(),
        simulator.base,
    );

    let total_scenarios = simulator.total_scenarios();

    // Initialise GPU if requested. Fall back to CPU with a warning if unavailable.
    let gpu_ctx: Option<GpuContext> = if cli.use_gpu {
        match GpuContext::try_init() {
            Some(ctx) => Some(ctx),
            None => {
                eprintln!(
                    "{}Warning:{} No suitable GPU adapter found — falling back to CPU.",
                    YELLOW, RESET
                );
                None
            }
        }
    } else {
        None
    };

    reporter.print_current_standings(&parsed);
    reporter.print_simulation_header(
        parsed.completed_matches,
        simulator.remaining_match_count(),
        simulator.base,
        total_scenarios,
        num_threads,
        gpu_ctx.is_some(),
    );

    if simulator.remaining_match_count() == 0 {
        println!("No remaining matches to simulate.");
        return Ok(());
    }

    let all_counts = simulate_all(
        &parsed,
        &simulator,
        num_threads,
        interactive,
        reporter.colors(),
        total_scenarios,
        gpu_ctx.as_ref(),
    );

    reporter.print_current_probabilities_heading(&parsed);
    reporter.print_results(
        &parsed,
        &all_counts.overall,
        total_scenarios,
        parsed.seat_scale,
    );

    if !parsed.matches.is_empty() {
        let (a, b) = parsed.matches[0];
        let base = simulator.base;
        let cond_total = total_scenarios / base;

        println!();
        reporter.print_next_match_impact_heading(&parsed);

        let title_a = format!("If {} beats {}", parsed.team_names[a], parsed.team_names[b]);
        reporter.print_next_match_scenario_heading(&title_a);
        reporter.print_results(
            &parsed,
            &all_counts.if_a_wins,
            cond_total,
            parsed.seat_scale,
        );
        println!();

        let title_b = format!("If {} beats {}", parsed.team_names[b], parsed.team_names[a]);
        reporter.print_next_match_scenario_heading(&title_b);
        reporter.print_results(
            &parsed,
            &all_counts.if_b_wins,
            cond_total,
            parsed.seat_scale,
        );
        println!();

        if cli.allow_no_results {
            let title_nr = format!(
                "If {} vs {} ends in NR",
                parsed.team_names[a], parsed.team_names[b]
            );
            reporter.print_next_match_scenario_heading(&title_nr);
            reporter.print_results(&parsed, &all_counts.if_nr, cond_total, parsed.seat_scale);
            println!();
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
