use std::collections::HashMap;
use std::convert::TryInto;
use std::sync::Arc;
use std::thread;

// ================================================================
// EDITABLE GLOBALS
// ================================================================

const TEAM_COUNT: usize = 10;
const ALLOW_NO_RESULTS: bool = false;

// Format per line: Team Name | Points | Wins
const STANDINGS_INPUT: &str = r#"
PBKS | 13 | 6
RCB  | 12 | 6
SRH  | 12 | 6
RR   | 12 | 6
GT   | 10 | 5
CSK  | 8  | 4
DC   | 8  | 4
KKR  | 7  | 3
MI   | 4  | 2
LSG  | 4  | 2
"#;

// Format per line: Team A vs Team B
const MATCHES_INPUT: &str = r#"
GT vs PBKS
MI vs LSG
DC vs CSK
SRH vs PBKS
LSG vs RCB
DC vs KKR
RR vs GT
CSK vs LSG
RCB vs MI
PBKS vs DC
GT vs SRH
RCB vs KKR
PBKS vs MI
LSG vs CSK
KKR vs GT
PBKS vs RCB
DC vs RR
CSK vs SRH
RR vs LSG
KKR vs MI
GT vs CSK
SRH vs RCB
LSG vs PBKS
MI vs RR
KKR vs DC
"#;

// ================================================================
// END EDITABLE GLOBALS
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

const SEAT_SCALE: u64 = seat_scale_for_team_count(TEAM_COUNT);

// ================================================================

#[derive(Clone, Copy, Default)]
struct Counts {
    top2_pts: [u64; TEAM_COUNT],
    top2_good_nrr_units: [u64; TEAM_COUNT],
    top4_pts: [u64; TEAM_COUNT],
    top4_good_nrr_units: [u64; TEAM_COUNT],
}

impl Counts {
    fn add_assign(&mut self, other: &Counts) {
        for i in 0..TEAM_COUNT {
            self.top2_pts[i] += other.top2_pts[i];
            self.top2_good_nrr_units[i] += other.top2_good_nrr_units[i];
            self.top4_pts[i] += other.top4_pts[i];
            self.top4_good_nrr_units[i] += other.top4_good_nrr_units[i];
        }
    }
}

#[derive(Clone, Copy)]
struct Task {
    next_match: usize,
    points: [u8; TEAM_COUNT],
    wins: [u8; TEAM_COUNT],
}

#[derive(Clone, Copy, Default)]
struct Group {
    points: u8,
    wins: u8,
    members: [usize; TEAM_COUNT],
    len: usize,
}

#[derive(Clone)]
struct ParsedInput {
    team_names: Vec<String>,
    points: [u8; TEAM_COUNT],
    wins: [u8; TEAM_COUNT],
    matches: Vec<(usize, usize)>,
}

#[derive(Clone)]
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

fn parse_standings() -> (Vec<String>, [u8; TEAM_COUNT], [u8; TEAM_COUNT], HashMap<String, usize>) {
    let mut team_names = Vec::new();
    let mut points_vec = Vec::new();
    let mut wins_vec = Vec::new();
    let mut team_map = HashMap::new();

    for raw_line in STANDINGS_INPUT.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let parts: Vec<&str> = line.split('|').map(|x| x.trim()).collect();
        if parts.len() != 3 {
            panic!("Invalid standings line: '{}'. Expected format: Team | Points | Wins", line);
        }
        if parts[0].eq_ignore_ascii_case("team") {
            continue;
        }

        let team = parts[0].to_string();
        let pts: u8 = parts[1].parse().unwrap_or_else(|_| panic!("Invalid points in '{}'.", line));
        let wins: u8 = parts[2].parse().unwrap_or_else(|_| panic!("Invalid wins in '{}'.", line));

        let key = canonical(&team);
        if team_map.contains_key(&key) {
            panic!("Duplicate team in standings: '{}'", team);
        }

        let idx = team_names.len();
        team_names.push(team);
        points_vec.push(pts);
        wins_vec.push(wins);
        team_map.insert(key, idx);
    }

    if team_names.len() != TEAM_COUNT {
        panic!("Expected {} teams in STANDINGS_INPUT, found {}", TEAM_COUNT, team_names.len());
    }

    let points: [u8; TEAM_COUNT] = points_vec.try_into().unwrap();
    let wins: [u8; TEAM_COUNT] = wins_vec.try_into().unwrap();
    (team_names, points, wins, team_map)
}

fn parse_match_line(line: &str) -> (String, String) {
    let lower = line.to_ascii_lowercase();
    if let Some(pos) = lower.find(" vs ") {
        return (line[..pos].trim().to_string(), line[pos + 4..].trim().to_string());
    }
    if let Some(pos) = lower.find(" v ") {
        return (line[..pos].trim().to_string(), line[pos + 3..].trim().to_string());
    }
    if let Some(pos) = line.find(',') {
        return (line[..pos].trim().to_string(), line[pos + 1..].trim().to_string());
    }
    panic!("Invalid match line: '{}'. Expected 'Team A vs Team B'", line);
}

fn parse_matches(team_map: &HashMap<String, usize>) -> Vec<(usize, usize)> {
    let mut matches = Vec::new();

    for raw_line in MATCHES_INPUT.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (a_name, b_name) = parse_match_line(line);
        let a = *team_map.get(&canonical(&a_name)).unwrap_or_else(|| panic!("Unknown team '{}' in '{}'.", a_name, line));
        let b = *team_map.get(&canonical(&b_name)).unwrap_or_else(|| panic!("Unknown team '{}' in '{}'.", b_name, line));

        if a == b {
            panic!("Invalid match '{}': a team cannot play itself", line);
        }
        matches.push((a, b));
    }

    matches
}

fn parse_inputs() -> ParsedInput {
    let (team_names, points, wins, team_map) = parse_standings();
    let matches = parse_matches(&team_map);
    ParsedInput { team_names, points, wins, matches }
}

fn pow_u64(base: u64, exp: usize) -> u64 {
    let mut ans = 1u64;
    for _ in 0..exp {
        ans = ans.checked_mul(base).expect("Scenario count overflowed u64");
    }
    ans
}

fn sort_teams(points: &[u8; TEAM_COUNT], wins: &[u8; TEAM_COUNT]) -> [usize; TEAM_COUNT] {
    let mut order = [0usize; TEAM_COUNT];
    for i in 0..TEAM_COUNT {
        order[i] = i;
    }
    order.sort_unstable_by(|&a, &b| {
        points[b]
            .cmp(&points[a])
            .then(wins[b].cmp(&wins[a]))
            .then(a.cmp(&b))
    });
    order
}

fn build_groups(
    order: &[usize; TEAM_COUNT],
    points: &[u8; TEAM_COUNT],
    wins: &[u8; TEAM_COUNT],
) -> ([Group; TEAM_COUNT], usize) {
    let mut groups = [Group::default(); TEAM_COUNT];
    let mut group_count = 0usize;

    for &team in order.iter() {
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
            members: [0usize; TEAM_COUNT],
            len: 1,
        };
        group.members[0] = team;
        groups[group_count] = group;
        group_count += 1;
    }

    (groups, group_count)
}

fn apply_cutoff(
    groups: &[Group; TEAM_COUNT],
    group_count: usize,
    cutoff: usize,
    guaranteed: &mut [u64; TEAM_COUNT],
    seat_aware_units: &mut [u64; TEAM_COUNT],
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
            (spots_here as u64) * SEAT_SCALE / (group.len as u64)
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

fn classify(points: &[u8; TEAM_COUNT], wins: &[u8; TEAM_COUNT], counts: &mut Counts) {
    let order = sort_teams(points, wins);
    let (groups, group_count) = build_groups(&order, points, wins);

    apply_cutoff(&groups, group_count, 2, &mut counts.top2_pts, &mut counts.top2_good_nrr_units);
    apply_cutoff(&groups, group_count, 4, &mut counts.top4_pts, &mut counts.top4_good_nrr_units);
}

fn dfs(
    match_idx: usize,
    matches: &[(usize, usize)],
    points: &mut [u8; TEAM_COUNT],
    wins: &mut [u8; TEAM_COUNT],
    counts: &mut Counts,
) {
    if match_idx == matches.len() {
        classify(points, wins, counts);
        return;
    }

    let (a, b) = matches[match_idx];

    points[a] += 2;
    wins[a] += 1;
    dfs(match_idx + 1, matches, points, wins, counts);
    wins[a] -= 1;
    points[a] -= 2;

    points[b] += 2;
    wins[b] += 1;
    dfs(match_idx + 1, matches, points, wins, counts);
    wins[b] -= 1;
    points[b] -= 2;

    if ALLOW_NO_RESULTS {
        points[a] += 1;
        points[b] += 1;
        dfs(match_idx + 1, matches, points, wins, counts);
        points[b] -= 1;
        points[a] -= 1;
    }
}

fn build_tasks(
    match_idx: usize,
    split_depth: usize,
    matches: &[(usize, usize)],
    points: &mut [u8; TEAM_COUNT],
    wins: &mut [u8; TEAM_COUNT],
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
    build_tasks(match_idx + 1, split_depth, matches, points, wins, tasks);
    wins[a] -= 1;
    points[a] -= 2;

    points[b] += 2;
    wins[b] += 1;
    build_tasks(match_idx + 1, split_depth, matches, points, wins, tasks);
    wins[b] -= 1;
    points[b] -= 2;

    if ALLOW_NO_RESULTS {
        points[a] += 1;
        points[b] += 1;
        build_tasks(match_idx + 1, split_depth, matches, points, wins, tasks);
        points[b] -= 1;
        points[a] -= 1;
    }
}

fn fmt_pct(numerator: u64, denominator: u64) -> String {
    format!("{:.4}%", numerator as f64 * 100.0 / denominator as f64)
}

fn fmt_scaled_pct(units: u64, total_scenarios: u64) -> String {
    let denom = (total_scenarios as f64) * (SEAT_SCALE as f64);
    format!("{:.4}%", (units as f64) * 100.0 / denom)
}

fn main() {
    let parsed = parse_inputs();
    let base = if ALLOW_NO_RESULTS { 3u64 } else { 2u64 };
    let total_scenarios = pow_u64(base, parsed.matches.len());
    let num_threads = thread::available_parallelism().map(|p| p.get()).unwrap_or(4);

    let mut split_depth = 0usize;
    let mut task_count = 1u64;
    while split_depth < parsed.matches.len() && task_count < (num_threads as u64 * 8) {
        split_depth += 1;
        task_count = task_count.checked_mul(base).expect("Task count overflowed u64");
    }

    let mut points = parsed.points;
    let mut wins = parsed.wins;
    let mut tasks = Vec::with_capacity(task_count as usize);
    build_tasks(0, split_depth, &parsed.matches, &mut points, &mut wins, &mut tasks);

    let matches = Arc::new(parsed.matches.clone());
    let tasks = Arc::new(tasks);
    let chunk_size = tasks.len().div_ceil(num_threads);

    println!("IPL Playoff Probabilities");
    println!("Teams: {}", TEAM_COUNT);
    println!("Remaining matches: {}", matches.len());
    println!("Outcome mode: {} per match", base);
    println!("Total scenarios: {}", total_scenarios);
    println!("Threads: {}", num_threads);
    println!();

    let mut handles = Vec::new();
    for t in 0..num_threads {
        let start = t * chunk_size;
        if start >= tasks.len() {
            break;
        }
        let end = ((t + 1) * chunk_size).min(tasks.len());
        let tasks_clone = Arc::clone(&tasks);
        let matches_clone = Arc::clone(&matches);

        let handle = thread::spawn(move || {
            let mut local = Counts::default();
            for task in &tasks_clone[start..end] {
                let mut local_points = task.points;
                let mut local_wins = task.wins;
                dfs(task.next_match, &matches_clone, &mut local_points, &mut local_wins, &mut local);
            }
            local
        });

        handles.push(handle);
    }

    let mut total_counts = Counts::default();
    for handle in handles {
        let local = handle.join().unwrap();
        total_counts.add_assign(&local);
    }

    let mut rows: Vec<Row> = (0..TEAM_COUNT)
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
        "{:<6} {:>14} {:>20} {:>14} {:>20}",
        "Team",
        "Top 2 Pts",
        "Top 2 Pts+Good NRR",
        "Top 4 Pts",
        "Top 4 Pts+Good NRR",
    );

    for row in rows {
        println!(
            "{:<6} {:>14} {:>20} {:>14} {:>20}",
            row.team,
            fmt_pct(row.top2_pts, total_scenarios),
            fmt_scaled_pct(row.top2_good_nrr_units, total_scenarios),
            fmt_pct(row.top4_pts, total_scenarios),
            fmt_scaled_pct(row.top4_good_nrr_units, total_scenarios),
        );
    }
}
