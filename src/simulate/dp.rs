use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::auto::AutoOptimizedStrategy;
use crate::models::{
    AllCounts, Counts, NR_SCORE_DELTA, PROGRESS_POLL_INTERVAL_MS, ParsedInput, StandingState,
    TEAM_BITS, WIN_SCORE_DELTA,
};
use crate::ranking::Ranker;
use crate::reporter::Reporter;
use crate::terminal::{ProgressPhase, Terminal, draw_progress};
use crate::utils::{
    current_rss_bytes, determine_num_threads, fmt_mem, format_with_commas, pow_u64,
};

/// d = Remaining matches that would purely run on DP Simulation excluding the seed match
pub fn estimate_dp_cost(d: usize, base: u64) -> (f64, f64) {
    // Determine the performance scaling factor based on threads.
    // Derived from 12-thread (Ryzen) vs 4-thread (Kaggle) memory bandwidth scaling.
    // The equation models throughput (M states/s): Thr(t) = A + B * threads
    // We normalize against the 12-thread baseline (factor = 1.0).
    let num_threads = determine_num_threads();
    let thr_12t = 1.77 + 0.40 * 12.0;
    let thr_current = 1.77 + 0.40 * (num_threads as f64);
    let time_scale_factor = thr_12t / thr_current;

    if base >= 3 {
        // Calibrated from --calibrate run (base=3, 18 total matches (including the seed match)).
        // Anchor: remaining=18; d=17.
        // Growth rates: RAM ~2.168x per match, Time ~2.166x per match.
        // RAM includes a strict 15% safety pad to prevent OOM.
        // Time uses the exact 12-thread curve fit.
        let diff = d as f64 - 18.0;
        let ram_mb = 4_660.0 * 2.168_f64.powf(diff) * 1.2725;
        let base_time_s = 20.0 * 2.166_f64.powf(diff);

        (ram_mb, base_time_s * time_scale_factor)
    } else {
        // Calibrated from --calibrate run (base=2, 46 total matches (including the seed match)).
        // Anchor: remaining=40; d=39.
        // Growth rates: RAM ~1.222x per match, Time ~1.242x per match.
        // RAM includes a strict 15% safety pad to prevent OOM.
        // Time uses the exact 12-thread curve fit.
        let diff = d as f64 - 40.0;
        let ram_mb = 1_925.0 * 1.222_f64.powf(diff) * 1.15;
        let base_time_s = 10.0 * 1.242_f64.powf(diff);

        (ram_mb, base_time_s * time_scale_factor)
    }
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
pub struct DpSimulator {
    matches: Vec<(usize, usize)>,
    ranker: Ranker,
    allow_no_results: bool,
    base: u64,
}

impl DpSimulator {
    pub fn new(parsed: &ParsedInput, allow_no_results: bool) -> Self {
        Self {
            matches: parsed.matches.clone(),
            ranker: Ranker::new(parsed.team_count, parsed.seat_scale),
            allow_no_results,
            base: if allow_no_results { 3 } else { 2 },
        }
    }

    pub fn base(&self) -> u64 {
        self.base
    }

    pub fn remaining_match_count(&self) -> usize {
        self.matches.len()
    }

    pub fn total_scenarios(&self) -> u64 {
        pow_u64(self.base, self.remaining_match_count())
    }

    /// == CORE MERGE LOGIC ===================================================
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

    pub fn simulate_forward(
        &self,
        branch_initial_state: StandingState,
        matches: &[(usize, usize)],
        term: &Terminal,
        global_start_time: Instant,
        match_offset: usize,
    ) -> (Vec<Vec<(u128, u64)>>, usize) {
        let total_matches = matches.len() + match_offset;
        let mut total_states = 1;
        let mut states: Vec<Vec<(u128, u64)>> = vec![vec![(branch_initial_state.score, 1)]];

        if term.interactive {
            draw_progress(
                ProgressPhase::DpSimulating {
                    match_idx: match_offset,
                    total_matches,
                    state_count: total_states,
                },
                term,
                global_start_time,
            );
        }

        for (idx, &(a, b)) in matches.iter().enumerate() {
            let (next_states, next_len) = self.process_next_match(states, total_states, a, b);
            states = next_states;
            total_states = next_len;

            if term.interactive {
                draw_progress(
                    ProgressPhase::DpSimulating {
                        match_idx: match_offset + idx + 1,
                        total_matches,
                        state_count: total_states,
                    },
                    term,
                    global_start_time,
                );
            }
        }

        (states, total_states)
    }

    pub fn classify_states_parallel(
        &self,
        states: Vec<Vec<(u128, u64)>>,
        total_states: usize,
        term: &Terminal,
        global_start_time: Instant,
    ) -> Counts {
        let num_threads = determine_num_threads();
        // Wrap the chunked states in a thread-safe Iterator
        // Using into_iter() means chunks are consumed and their memory is instantly freed when a thread is done with them.
        let chunk_iter = Arc::new(Mutex::new(states.into_iter()));
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
        if term.interactive {
            let mut last_drawn = usize::MAX;
            loop {
                let done = states_done.load(Ordering::Relaxed).min(total_states);
                if done != last_drawn {
                    draw_progress(
                        ProgressPhase::DpClassifying {
                            states_done: done,
                            total_states,
                        },
                        term,
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

    fn run_branch(
        &self,
        branch_initial_state: StandingState,
        matches: &[(usize, usize)],
        branch_name: &str,
        term: &Terminal,
        global_start_time: Instant,
    ) -> Counts {
        println!(
            "{}{}{} ==========",
            term.colors.cyan, branch_name, term.colors.reset
        );
        let (states, total_states) =
            self.simulate_forward(branch_initial_state, matches, term, global_start_time, 1);
        // Print a newline so the Simulating Progress bar is saved and Classifying gets a fresh line
        println!();
        self.classify_states_parallel(states, total_states, term, global_start_time)
    }

    pub fn run(
        &self,
        initial_state: &StandingState,
        reporter: &Reporter,
        term: &Terminal,
    ) -> AllCounts {
        if self.matches.is_empty() {
            let mut leaf = Counts::default();
            self.ranker.classify(initial_state, &mut leaf);
            let mut all = AllCounts::default();
            all.overall += &leaf;
            return all;
        }
        reporter.print_dp_estimate(self.matches.len().saturating_sub(1), self.base);

        let global_start_time = Instant::now();
        let (a0, b0) = self.matches[0];
        let remaining_matches = &self.matches[1..];
        let mut all = AllCounts::default();

        let mut state_a = *initial_state;
        state_a.record_win(a0);
        let counts_a = self.run_branch(
            state_a,
            remaining_matches,
            "==== Branch: Team A Wins ",
            term,
            global_start_time,
        );
        all.if_a_wins += &counts_a;
        all.overall += &counts_a;

        let mut state_b = *initial_state;
        state_b.record_win(b0);
        let counts_b = self.run_branch(
            state_b,
            remaining_matches,
            "==== Branch: Team B Wins ",
            term,
            global_start_time,
        );
        all.if_b_wins += &counts_b;
        all.overall += &counts_b;

        if self.allow_no_results {
            let mut state_nr = *initial_state;
            state_nr.record_no_result(a0, b0);
            let counts_nr = self.run_branch(
                state_nr,
                remaining_matches,
                "==== Branch: No Result ",
                term,
                global_start_time,
            );
            all.if_nr += &counts_nr;
            all.overall += &counts_nr;
        }

        all
    }

    pub fn run_calibration(&self, initial_state: StandingState, term: &Terminal) {
        let total = self.matches.len();
        println!(
            "=============== DP Calibration ({} total matches, base={}) ================",
            total, self.base
        );
        println!(
            " {:>2} | {:>12} {:>10} {:>9} | {:>7} {:>10} {:>9}",
            "d", "States", "Real RAM", "Real Time", "Auto DP", "Est RAM", "Est Time"
        );
        println!("{}", "-".repeat(74));

        let baseline_rss = current_rss_bytes().unwrap_or(0);

        for d in 1..=total {
            let dp_matches = &self.matches[total - d..];

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

            let start = Instant::now();
            let (states, total_states) =
                self.simulate_forward(initial_state, dp_matches, term, Instant::now(), 0);
            let _counts = self.classify_states_parallel(states, total_states, term, Instant::now());

            let elapsed = start.elapsed().as_secs_f64();

            // Stop the poller and read peak.
            stop_flag.store(1, Ordering::Relaxed);
            // Give poller one last chance to fire before we read.
            std::thread::sleep(Duration::from_millis(60));
            let peak_rss = peak_rss_atomic.load(Ordering::Relaxed);
            let peak_ram = peak_rss.saturating_sub(baseline_rss);

            // Multiply by base: pure DP runs `base` branches sequentially.
            let total_time = elapsed * self.base as f64;

            // Auto Strategy parameters if we had exactly `d` matches remaining
            let auto_optimized_strategy = AutoOptimizedStrategy::for_remaining(d + 1, self.base);

            println!(
                " {:>2} | {:>12} {:>10} {:>8.2}s | {:>7} {:>10} {:>8.2}s",
                d,
                format_with_commas(total_states as u64),
                fmt_mem(peak_ram),
                total_time,
                auto_optimized_strategy.optimal_dp_size,
                fmt_mem(auto_optimized_strategy.est_peak_ram_mb as u64 * 1024 * 1024),
                auto_optimized_strategy.est_compute_time
            );
        }

        println!("{}", "-".repeat(74));
    }
}
