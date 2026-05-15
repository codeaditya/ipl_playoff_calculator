use std::sync::{Mutex, OnceLock};
use std::thread;

use sysinfo::{Pid, ProcessesToUpdate, System};

// ================================================================
// MATH HELPERS
// ================================================================

pub const fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

pub const fn lcm_u64(a: u64, b: u64) -> u64 {
    if a == 0 || b == 0 {
        0
    } else {
        a / gcd_u64(a, b) * b
    }
}

pub const fn seat_scale_for_team_count(team_count: usize) -> u64 {
    let mut scale = 1u64;
    let mut x = 1usize;
    while x <= team_count {
        scale = lcm_u64(scale, x as u64);
        x += 1;
    }
    scale
}

pub fn pow_u64(base: u64, exp: usize) -> u64 {
    (0..exp)
        .try_fold(1u64, |acc, _| acc.checked_mul(base))
        .unwrap_or(u64::MAX)
}

// ================================================================
// SYSTEM INFO HELPERS
// ================================================================

pub fn current_rss_bytes() -> Option<u64> {
    static SYS: OnceLock<Mutex<System>> = OnceLock::new();
    let system = SYS.get_or_init(|| Mutex::new(System::new()));
    let mut system = system.lock().ok()?;
    let pid = Pid::from_u32(std::process::id());
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid).map(|p| p.memory())
}

pub fn get_free_system_ram_mb() -> f64 {
    let mut system = System::new();
    system.refresh_memory();
    system.available_memory() as f64 / 1_048_576.0
}

pub fn get_usable_ram_mb(free_ram_mb: f64) -> f64 {
    // SAFE RAM CHECK: If > 1.5GB, leave 1GB for the OS. If very tight, use exactly 50%.
    if free_ram_mb > 1500.0 {
        (free_ram_mb - 1000.0).max(free_ram_mb * 0.5)
    } else {
        free_ram_mb * 0.5
    }
}

pub fn determine_num_threads() -> usize {
    thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4)
}

// ================================================================
// FORMATTING HELPERS
// ================================================================

pub fn format_with_commas(n: u64) -> String {
    let mut s = n.to_string();
    let mut i = s.len();
    while i > 3 {
        i -= 3;
        s.insert(i, ',');
    }
    s
}

pub fn fmt_mem(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}

pub fn fmt_pct(numerator: u64, denominator: u64) -> String {
    if numerator == 0 {
        "-".to_string()
    } else {
        format!("{:.2}%", numerator as f64 * 100.0 / denominator as f64)
    }
}

pub fn fmt_scaled_pct(units: u64, total_scenarios: u64, seat_scale: u64) -> String {
    if units == 0 {
        "-".to_string()
    } else {
        format!(
            "{:.2}%",
            (units as f64) * 100.0 / ((total_scenarios as f64) * (seat_scale as f64))
        )
    }
}
