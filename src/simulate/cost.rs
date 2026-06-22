use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::auto::AutoOptimizedStrategy;
use super::dp::DpSimulator;
use crate::models::StandingState;
use crate::terminal::Terminal;
use crate::utils::{current_rss_bytes, determine_num_threads, fmt_mem, format_with_commas};

/// d = Remaining matches that would purely run on DP Simulation excluding the seed match
pub fn estimate_dp_cost(d: usize, base: u64) -> (f64, f64) {
    // Only the classifying stage is parallel. The building stage (merge) is
    // single-threaded, so we scale only the classifying portion.
    // We normalize against the 12-thread baseline on which we calibrated our run.
    let time_scale_factor = 12.0 / (determine_num_threads() as f64);

    if base >= 3 {
        // Calibrated from --calibrate-dp run (base=3, 19 total matches (including the seed match)).
        // Anchor: remaining=19; d=18.
        // Growth rates: RAM ~2.168x per match, Time ~2.166x per match.
        // RAM includes a strict 15% safety pad to prevent OOM.
        let diff = d as f64 - 18.0;
        let additional_ram_buffer = 1.2725;
        let ram_mb = 3_480.0 * 2.168_f64.powf(diff) * additional_ram_buffer;
        let build_time_s = 11.4 * 2.166_f64.powf(diff);
        let classify_time_s = 6.1 * 2.166_f64.powf(diff);
        let total_time_s = build_time_s + classify_time_s * time_scale_factor;

        (ram_mb, total_time_s)
    } else {
        // Calibrated from --calibrate-dp run (base=2, 49 total matches (including the seed match)).
        // Anchor: remaining=41; d=40.
        // Growth rates: RAM ~1.222x per match, Time ~1.242x per match.
        // RAM includes a strict 15% safety pad to prevent OOM.
        let diff = d as f64 - 40.0;
        let additional_ram_buffer = 1.15;
        let ram_mb = 1_435.0 * 1.222_f64.powf(diff) * additional_ram_buffer;
        let build_time_s = 6.5 * 1.242_f64.powf(diff);
        let classify_time_s = 1.8 * 1.242_f64.powf(diff);
        let total_time_s = build_time_s + classify_time_s * time_scale_factor;

        (ram_mb, total_time_s)
    }
}

pub fn calibrate_dp(sim: &DpSimulator, initial_state: StandingState) {
    let term = Terminal::new(false);
    let total = sim.remaining_match_count();
    println!(
        "{repeat} DP Calibration ({total} total matches, base={base}) {repeat}",
        repeat = "=".repeat(26),
        total = total,
        base = sim.base(),
    );
    println!(
        " {:>2} | {:>13} {:>11} {:>9} {:>9} {:>9} | {:>7} {:>11} {:>9}",
        "d",
        "States",
        "Real RAM",
        "Real Time",
        "Build",
        "Classify",
        "Auto DP",
        "Est RAM",
        "Est Time"
    );
    println!("{}", "-".repeat(95));

    let baseline_rss = current_rss_bytes().unwrap_or(0);

    for d in 1..=total {
        let dp_matches = &sim.matches()[total - d..];

        // Spawn a background thread that polls RSS every 50ms and records the peak.
        let peak_rss_atomic = Arc::new(AtomicU64::new(0));
        let stop_flag = Arc::new(AtomicU64::new(0));
        {
            let peak_rss_clone = Arc::clone(&peak_rss_atomic);
            let stop_clone = Arc::clone(&stop_flag);
            std::thread::spawn(move || {
                loop {
                    if let Some(rss) = current_rss_bytes() {
                        peak_rss_clone.fetch_max(rss, Ordering::Relaxed);
                    }
                    if stop_clone.load(Ordering::Relaxed) != 0 {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            });
        }

        let build_start = Instant::now();
        let states = sim.simulate_forward(initial_state, dp_matches, &term, Instant::now(), 0);
        let total_states = states.total_states;
        let time_build_raw = build_start.elapsed().as_secs_f64();

        let classify_start = Instant::now();
        let _counts = sim.classify_states_parallel(states, &term, Instant::now());
        let time_classify_raw = classify_start.elapsed().as_secs_f64();

        // Stop the poller and read peak.
        stop_flag.store(1, Ordering::Relaxed);
        // Give poller one last chance to fire before we read.
        std::thread::sleep(Duration::from_millis(60));
        let peak_rss = peak_rss_atomic.load(Ordering::Relaxed);
        let peak_ram = peak_rss.saturating_sub(baseline_rss);

        // Multiply by base: pure DP runs `base` branches sequentially.
        let build_time = time_build_raw * sim.base() as f64;
        let classify_time = time_classify_raw * sim.base() as f64;
        let total_time = build_time + classify_time;

        // Auto Strategy parameters if we had exactly `d` matches remaining
        let auto_optimized_strategy = AutoOptimizedStrategy::for_remaining(d + 1, sim.base());

        println!(
            " {:>2} | {:>13} {:>11} {:>8.2}s {:>8.2}s {:>8.2}s | {:>7} {:>11} {:>8.2}s",
            d,
            format_with_commas(total_states as u64),
            fmt_mem(peak_ram),
            total_time,
            build_time,
            classify_time,
            auto_optimized_strategy.optimal_dp_size,
            fmt_mem(auto_optimized_strategy.est_peak_ram_mb as u64 * 1024 * 1024),
            auto_optimized_strategy.est_compute_time
        );
    }

    println!("{}", "-".repeat(95));
}
