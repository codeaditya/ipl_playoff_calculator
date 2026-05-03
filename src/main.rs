use std::env;
use std::fs;
use std::collections::HashMap;
use std::fmt;
use std::ops::AddAssign;
use std::sync::Arc;
use std::thread;
use std::io::{self, Write};
use std::sync::mpsc;
use std::time::Instant;

// ================================================================
// TERMINAL COLORS
// ================================================================

const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";
const GREEN: &str = "\x1b[32m";
const MAGENTA: &str = "\x1b[35m";

// ================================================================

// Internal max capacity to maintain stack allocation performance
const MAX_TEAMS: usize = 16;

/// Task multiplier: generate at least this many tasks per thread for good load balancing.
const TASKS_PER_THREAD_TARGET: u64 = 512;

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

// ================================================================

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
    points: [u8; MAX_TEAMS],
    wins: [u8; MAX_TEAMS],
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
    points: [u8; MAX_TEAMS],
    wins: [u8; MAX_TEAMS],
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

fn canonical(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn parse_match_line(line: &str) -> Result<(String, String), AppError> {
    let lower = line.to_ascii_lowercase();
    if let Some(pos) = lower.find(" vs ") {
        return Ok((line[..pos].trim().to_string(), line[pos + 4..].trim().to_string()));
    }
    if let Some(pos) = lower.find(" v ") {
        return Ok((line[..pos].trim().to_string(), line[pos + 3..].trim().to_string()));
    }
    if let Some(pos) = line.find(',') {
        return Ok((line[..pos].trim().to_string(), line[pos + 1..].trim().to_string()));
    }
    Err(AppError::Parse(format!(
        "Invalid match line: '{}'. Expected 'Team A vs Team B'",
        line
    )))
}

fn parse_inputs(matches_input: &str) -> Result<ParsedInput, AppError> {
    let mut team_names: Vec<String> = Vec::new();
    let mut team_map: HashMap<String, usize> = HashMap::new();
    let mut points = [0u8; MAX_TEAMS];
    let mut wins = [0u8; MAX_TEAMS];
    let mut matches_played = [0u8; MAX_TEAMS];
    let mut losses = [0u8; MAX_TEAMS];
    let mut no_results = [0u8; MAX_TEAMS];
    let mut matches = Vec::new();
    let mut completed_matches = 0usize;

    // Helper closure: returns Err if team limit is exceeded
    let mut get_or_insert_team = |name: &str| -> Result<usize, AppError> {
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
    };

    for raw_line in matches_input.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Use splitn(2, ':') to safely split into at most the match part and outcome part
        let mut parts = line.splitn(2, ':');
        let match_part = parts.next().unwrap_or("").trim();
        let outcome_part = parts.next(); // None if no ':' present

        let (a_name, b_name) = parse_match_line(match_part)?;

        let a = get_or_insert_team(&a_name)?;
        let b = get_or_insert_team(&b_name)?;

        if let Some(outcome_str) = outcome_part {
            completed_matches += 1;
            matches_played[a] += 1;
            matches_played[b] += 1;

            let outcome = canonical(outcome_str.trim());
            if outcome == canonical("NR") {
                points[a] += 1;
                points[b] += 1;
                no_results[a] += 1;
                no_results[b] += 1;
            } else if outcome == canonical(&a_name) {
                points[a] += 2;
                wins[a] += 1;
                losses[b] += 1;
            } else if outcome == canonical(&b_name) {
                points[b] += 2;
                wins[b] += 1;
                losses[a] += 1;
            } else {
                return Err(AppError::Parse(format!(
                    "Invalid outcome '{}' in line '{}'",
                    outcome_str.trim(),
                    line
                )));
            }
        } else {
            matches.push((a, b));
        }
    }

    let team_count = team_names.len();
    let seat_scale = seat_scale_for_team_count(team_count);

    Ok(ParsedInput {
        team_count,
        seat_scale,
        team_names,
        points,
        wins,
        matches_played,
        losses,
        no_results,
        matches,
        completed_matches,
    })
}

fn pow_u64(base: u64, exp: usize) -> Result<u64, AppError> {
    (0..exp).try_fold(1u64, |acc, _| {
        acc.checked_mul(base)
            .ok_or_else(|| AppError::Parse("Scenario count overflowed u64".to_string()))
    })
}

fn sort_teams(team_count: usize, points: &[u8; MAX_TEAMS], wins: &[u8; MAX_TEAMS]) -> [usize; MAX_TEAMS] {
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
    guaranteed: &mut [u64; MAX_TEAMS],
    seat_aware_units: &mut [u64; MAX_TEAMS],
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
                seat_aware_units[team] += seat_units_per_team;
            }
            if fully_inside_cutoff {
                guaranteed[team] += 1;
            }
        }

        placed_above += group.len;
    }
}

fn classify(
    team_count: usize,
    seat_scale: u64,
    points: &[u8; MAX_TEAMS],
    wins: &[u8; MAX_TEAMS],
    counts: &mut Counts,
) {
    let order = sort_teams(team_count, points, wins);
    let (groups, group_count) = build_groups(&order, team_count, points, wins);
    apply_cutoff(&groups, group_count, 2, seat_scale, &mut counts.top2_pts, &mut counts.top2_good_nrr_units);
    apply_cutoff(&groups, group_count, 4, seat_scale, &mut counts.top4_pts, &mut counts.top4_good_nrr_units);
}

fn dfs(
    match_idx: usize,
    matches: &[(usize, usize)],
    team_count: usize,
    seat_scale: u64,
    allow_no_results: bool,
    points: &mut [u8; MAX_TEAMS],
    wins: &mut [u8; MAX_TEAMS],
    counts: &mut Counts,
) {
    if match_idx == matches.len() {
        classify(team_count, seat_scale, points, wins, counts);
        return;
    }

    let (a, b) = matches[match_idx];

    // Outcome 1: Team A wins
    points[a] += 2;
    wins[a] += 1;
    dfs(match_idx + 1, matches, team_count, seat_scale, allow_no_results, points, wins, counts);
    wins[a] -= 1;
    points[a] -= 2;

    // Outcome 2: Team B wins
    points[b] += 2;
    wins[b] += 1;
    dfs(match_idx + 1, matches, team_count, seat_scale, allow_no_results, points, wins, counts);
    wins[b] -= 1;
    points[b] -= 2;

    // Outcome 3: No Result (Tie/Washout)
    if allow_no_results {
        points[a] += 1;
        points[b] += 1;
        dfs(match_idx + 1, matches, team_count, seat_scale, allow_no_results, points, wins, counts);
        points[b] -= 1;
        points[a] -= 1;
    }
}

fn build_tasks(
    match_idx: usize,
    split_depth: usize,
    matches: &[(usize, usize)],
    allow_no_results: bool,
    points: &mut [u8; MAX_TEAMS],
    wins: &mut [u8; MAX_TEAMS],
    tasks: &mut Vec<Task>,
) {
    if match_idx == split_depth {
        tasks.push(Task {
            next_match: match_idx,
            points: *points,
            wins: *wins,
        });
        return;
    }

    let (a, b) = matches[match_idx];

    points[a] += 2;
    wins[a] += 1;
    build_tasks(match_idx + 1, split_depth, matches, allow_no_results, points, wins, tasks);
    wins[a] -= 1;
    points[a] -= 2;

    points[b] += 2;
    wins[b] += 1;
    build_tasks(match_idx + 1, split_depth, matches, allow_no_results, points, wins, tasks);
    wins[b] -= 1;
    points[b] -= 2;

    if allow_no_results {
        points[a] += 1;
        points[b] += 1;
        build_tasks(match_idx + 1, split_depth, matches, allow_no_results, points, wins, tasks);
        points[b] -= 1;
        points[a] -= 1;
    }
}

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
    format!("{:.4}%", numerator as f64 * 100.0 / denominator as f64)
}

fn fmt_scaled_pct(units: u64, total_scenarios: u64, seat_scale: u64) -> String {
    let denom = (total_scenarios as f64) * (seat_scale as f64);
    format!("{:.4}%", (units as f64) * 100.0 / denom)
}

fn print_usage(program_name: &str) {
    eprintln!("{BOLD}{CYAN}IPL Playoff Calculator{RESET}\n");
    eprintln!("{BOLD}{YELLOW}Usage:{RESET} {} <matches_file> [--allow-no-results]", program_name);
    eprintln!("\n{BOLD}Arguments:{RESET}");
    eprintln!("  <matches_file>       Path to the text file containing the schedule.");
    eprintln!("  --allow-no-results   (Optional) Include ties/washouts (1 pt each) in future outcomes.");
    eprintln!("\n{BOLD}Matches File Format Instructions:{RESET}");
    eprintln!("  - Provide one match per line. Lines starting with '#' are ignored.");
    eprintln!("  - {BOLD}Upcoming match:{RESET}  Team A vs Team B");
    eprintln!("  - {BOLD}Completed match:{RESET} Team A vs Team B : Winner Team");
    eprintln!("  - {BOLD}No Result / Tie:{RESET} Team A vs Team B : NR");
    eprintln!("\n{BOLD}Example:{RESET}");
    eprintln!("  CSK vs RCB : CSK  # CSK won this match");
    eprintln!("  MI vs DC          # Upcoming match\n");
}

fn run() -> Result<(), AppError> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_usage(&args[0]);
        std::process::exit(1);
    }

    let file_path = &args[1];
    let allow_no_results = args.iter().any(|arg| arg == "--allow-no-results");

    let matches_input = fs::read_to_string(file_path).map_err(|e| {
        AppError::Parse(format!("Error reading file '{}': {}", file_path, e))
    })?;

    let parsed = parse_inputs(&matches_input)?;
    let team_count = parsed.team_count;
    let seat_scale = parsed.seat_scale;

    let base = if allow_no_results { 3u64 } else { 2u64 };
    let total_scenarios = pow_u64(base, parsed.matches.len())?;
    let num_threads = thread::available_parallelism().map(|p| p.get()).unwrap_or(4);

    let mut split_depth = 0usize;
    let mut task_count = 1u64;
    while split_depth < parsed.matches.len()
        && task_count < (num_threads as u64 * TASKS_PER_THREAD_TARGET)
    {
        split_depth += 1;
        task_count = task_count
            .checked_mul(base)
            .ok_or_else(|| AppError::Parse("Task count overflowed u64".to_string()))?;
    }

    let mut points = parsed.points;
    let mut wins = parsed.wins;
    let mut tasks = Vec::with_capacity(task_count as usize);
    build_tasks(0, split_depth, &parsed.matches, allow_no_results, &mut points, &mut wins, &mut tasks);

    let matches = Arc::new(parsed.matches.clone());
    let tasks = Arc::new(tasks);
    let chunk_size = tasks.len().div_ceil(num_threads);

    // Compute dynamic column width from the longest team name
    let name_col_width = parsed.team_names.iter().map(|n| n.len()).max().unwrap_or(6).max(6);

    println!("{BOLD}{CYAN}=========== Current Standings ==========={RESET}");
    println!(
        "{BOLD}{YELLOW}{:>8} {:<width$} {:>4} {:>4} {:>4} {:>4} {:>4}{RESET}",
        "Position", "Team", "M", "W", "L", "NR", "Pts",
        width = name_col_width
    );

    let current_order = sort_teams(team_count, &parsed.points, &parsed.wins);
    for (idx, &i) in current_order.iter().take(team_count).enumerate() {
        println!(
            "{:>8} {BOLD}{GREEN}{:<width$}{RESET} {:>4} {:>4} {:>4} {:>4} {BOLD}{:>4}{RESET}",
            idx + 1,
            parsed.team_names[i],
            parsed.matches_played[i],
            parsed.wins[i],
            parsed.losses[i],
            parsed.no_results[i],
            parsed.points[i],
            width = name_col_width
        );
    }

    println!();

    println!("{BOLD}{CYAN}========= Playoff Probabilities ========={RESET}");
    println!("{MAGENTA}Completed matches:{RESET} {}", parsed.completed_matches);
    println!("{MAGENTA}Remaining matches:{RESET} {}", matches.len());
    println!("{MAGENTA}Outcome mode:{RESET} {} per match", base);
    println!("{MAGENTA}Total scenarios:{RESET} {}", format_with_commas(total_scenarios));
    println!("{MAGENTA}Threads:{RESET} {}", num_threads);
    println!();

    let total_tasks = tasks.len();

    if total_tasks == 0 {
        println!("No remaining matches to simulate.");
        // Still need to classify the current state
        let mut final_counts = Counts::default();
        classify(team_count, seat_scale, &parsed.points, &parsed.wins, &mut final_counts);
        print_results(&parsed, &final_counts, total_scenarios, seat_scale, name_col_width);
        return Ok(());
    }

    let (tx, rx) = mpsc::channel();
    let start_time = Instant::now();

    let mut handles = Vec::new();
    for t in 0..num_threads {
        let start = t * chunk_size;
        if start >= tasks.len() {
            break;
        }

        let end = ((t + 1) * chunk_size).min(tasks.len());
        let tasks_clone = Arc::clone(&tasks);
        let matches_clone = Arc::clone(&matches);
        let tx_clone = tx.clone();

        let handle = thread::spawn(move || {
            let mut local = Counts::default();
            for task in &tasks_clone[start..end] {
                let mut local_points = task.points;
                let mut local_wins = task.wins;
                dfs(
                    task.next_match,
                    &matches_clone,
                    team_count,
                    seat_scale,
                    allow_no_results,
                    &mut local_points,
                    &mut local_wins,
                    &mut local,
                );
                let _ = tx_clone.send(());
            }
            local
        });

        handles.push(handle);
    }

    // Drop the original sender so the channel closes when all threads finish
    drop(tx);

    // --- Progress Bar UI Loop ---
    let mut completed_tasks = 0usize;

    for _ in rx {
        completed_tasks += 1;
        let elapsed = start_time.elapsed().as_secs_f64();
        let pct = (completed_tasks as f64) / (total_tasks as f64);

        let eta = if pct > 0.0 {
            (elapsed / pct) - elapsed
        } else {
            0.0
        };

        let bar_width = 40;
        let filled = (pct * bar_width as f64) as usize;
        let bar: String = (0..bar_width)
            .map(|i| if i < filled { '=' } else if i == filled { '>' } else { ' ' })
            .collect();

        print!(
            "\r{CYAN}Progress:{RESET} [{bar}] {BOLD}{:>5.1}%{RESET} | {YELLOW}Elapsed:{RESET} {:>5.1}s | {GREEN}ETA:{RESET} {:>5.1}s ",
            pct * 100.0,
            elapsed,
            eta
        );
        io::stdout().flush().unwrap();
    }

    println!("\n"); // Move to a new line once complete

    // Collect the final calculations from all threads
    let mut total_counts = Counts::default();
    for handle in handles {
        let local = handle.join().unwrap();
        total_counts += &local;
    }

    print_results(&parsed, &total_counts, total_scenarios, seat_scale, name_col_width);

    Ok(())
}

fn print_results(
    parsed: &ParsedInput,
    total_counts: &Counts,
    total_scenarios: u64,
    seat_scale: u64,
    name_col_width: usize,
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
        "{BOLD}{YELLOW}{:<width$} {:>14} {:>20} {:>14} {:>20}{RESET}",
        "Team", "Top 2 Pts", "Top 2 Pts+Good NRR", "Top 4 Pts", "Top 4 Pts+Good NRR",
        width = name_col_width
    );

    for row in rows {
        println!(
            "{BOLD}{GREEN}{:<width$}{RESET} {:>14} {:>20} {:>14} {:>20}",
            row.team,
            fmt_pct(row.top2_pts, total_scenarios),
            fmt_scaled_pct(row.top2_good_nrr_units, total_scenarios, seat_scale),
            fmt_pct(row.top4_pts, total_scenarios),
            fmt_scaled_pct(row.top4_good_nrr_units, total_scenarios, seat_scale),
            width = name_col_width
        );
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{BOLD}{YELLOW}Error:{RESET} {}", e);
        std::process::exit(1);
    }
}
