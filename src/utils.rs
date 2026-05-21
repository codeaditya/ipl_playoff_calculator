use std::cell::Cell;
use std::sync::{Mutex, OnceLock};
use std::thread;

use sysinfo::{Pid, ProcessesToUpdate, System};

thread_local! {
    static SYSTEM_RAM_OVERRIDE: Cell<Option<f64>> = const { Cell::new(None) };
}

/// Sets a per-thread system RAM override (in MB). Pass `None` to clear.
pub fn set_system_ram_override(mb: Option<f64>) {
    SYSTEM_RAM_OVERRIDE.with(|cell| cell.set(mb));
}

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
    if let Some(override_val) = SYSTEM_RAM_OVERRIDE.with(|cell| cell.get()) {
        return override_val;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gcd() {
        assert_eq!(gcd_u64(12, 8), 4);
        assert_eq!(gcd_u64(17, 13), 1);
        assert_eq!(gcd_u64(100, 25), 25);
        assert_eq!(gcd_u64(0, 5), 5);
        assert_eq!(gcd_u64(5, 0), 5);
        assert_eq!(gcd_u64(0, 0), 0);
        assert_eq!(gcd_u64(1, 1), 1);
    }

    #[test]
    fn test_lcm() {
        assert_eq!(lcm_u64(4, 6), 12);
        assert_eq!(lcm_u64(3, 5), 15);
        assert_eq!(lcm_u64(7, 7), 7);
        assert_eq!(lcm_u64(0, 5), 0);
        assert_eq!(lcm_u64(5, 0), 0);
        assert_eq!(lcm_u64(1, 1), 1);
    }

    #[test]
    fn test_seat_scale() {
        assert_eq!(seat_scale_for_team_count(1), 1);
        assert_eq!(seat_scale_for_team_count(2), 2);
        assert_eq!(seat_scale_for_team_count(3), 6);
        assert_eq!(seat_scale_for_team_count(4), 12);
        assert_eq!(seat_scale_for_team_count(5), 60);
        assert_eq!(seat_scale_for_team_count(6), 60);
        assert_eq!(seat_scale_for_team_count(7), 420);
        assert_eq!(seat_scale_for_team_count(8), 840);
        assert_eq!(seat_scale_for_team_count(9), 2520);
        assert_eq!(seat_scale_for_team_count(10), 2520);
    }

    #[test]
    fn test_pow_u64() {
        assert_eq!(pow_u64(2, 0), 1);
        assert_eq!(pow_u64(2, 1), 2);
        assert_eq!(pow_u64(2, 10), 1024);
        assert_eq!(pow_u64(3, 5), 243);
        assert_eq!(pow_u64(10, 3), 1000);
        assert_eq!(pow_u64(1, 100), 1);
    }

    #[test]
    fn test_pow_u64_overflow() {
        let result = pow_u64(2, 64);
        assert_eq!(result, u64::MAX);
    }

    #[test]
    fn test_format_with_commas() {
        assert_eq!(format_with_commas(0), "0");
        assert_eq!(format_with_commas(1), "1");
        assert_eq!(format_with_commas(999), "999");
        assert_eq!(format_with_commas(1000), "1,000");
        assert_eq!(format_with_commas(1234567), "1,234,567");
        assert_eq!(format_with_commas(1000000000), "1,000,000,000");
    }

    #[test]
    fn test_fmt_mem() {
        assert_eq!(fmt_mem(0), "0 KB");
        assert_eq!(fmt_mem(1024), "1 KB");
        assert_eq!(fmt_mem(2048), "2 KB");
        assert_eq!(fmt_mem(1_048_576), "1.0 MB");
        assert_eq!(fmt_mem(2_097_152), "2.0 MB");
        assert_eq!(fmt_mem(1_572_864), "1.5 MB");
    }

    #[test]
    fn test_fmt_pct() {
        assert_eq!(fmt_pct(0, 100), "-");
        assert_eq!(fmt_pct(50, 100), "50.00%");
        assert_eq!(fmt_pct(25, 100), "25.00%");
        assert_eq!(fmt_pct(1, 3), "33.33%");
        assert_eq!(fmt_pct(100, 100), "100.00%");
    }

    #[test]
    fn test_fmt_scaled_pct() {
        assert_eq!(fmt_scaled_pct(0, 100, 6), "-");
        let result = fmt_scaled_pct(150, 100, 6);
        assert_eq!(result, "25.00%");
    }

    #[test]
    fn test_system_ram_override() {
        // Override returns the exact value set
        set_system_ram_override(Some(42.0));
        assert_eq!(get_free_system_ram_mb(), 42.0);

        set_system_ram_override(Some(100.0));
        assert_eq!(get_free_system_ram_mb(), 100.0);

        // Clearing override returns real system RAM
        set_system_ram_override(None);
        let real_ram = get_free_system_ram_mb();
        assert!(real_ram > 0.0);
    }

    #[test]
    fn test_get_usable_ram_mb() {
        assert_eq!(get_usable_ram_mb(1000.0), 500.0);
        assert_eq!(get_usable_ram_mb(1500.0), 750.0);
        assert_eq!(get_usable_ram_mb(2000.0), 1000.0);
        assert_eq!(get_usable_ram_mb(8000.0), 7000.0);
    }

    #[test]
    fn test_determine_num_threads() {
        let threads = determine_num_threads();
        assert!(threads >= 1);
    }
}
