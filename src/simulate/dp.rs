use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::models::{
    AllCounts, Counts, NR_SCORE_DELTA, PROGRESS_POLL_INTERVAL_MS, ParsedInput, StandingState,
    TEAM_BITS, WIN_SCORE_DELTA,
};
use crate::ranking::Ranker;
use crate::reporter::Reporter;
use crate::terminal::{ProgressPhase, Terminal, draw_progress};
use crate::utils::{determine_num_threads, pow_u64};

// ================================================================
// DP SIMULATION
// ================================================================
//
// Merges states with identical standings after each match.
// A state is stored as a packed u128 score vector.
//
// After all matches are processed, each unique final state is
// classified once and multiplied by its weights — replacing 3^N leaf
// classifications with O(distinct_states) classifications.
//
// Each state is stored using a Structure-of-Arrays (SoA) layout —
// chunked Vecs of scores (u128) and weights (u64). During the 3-way
// merge in process_next_match, only scores are loaded for comparison;
// weights are loaded only when a stream's value is selected as the
// minimum.
//
// Chunking (Vec<Vec<T>> instead of Vec<T>): each inner Vec holds
// CHUNK_SIZE elements. As the 3-way merge's linear scan advances,
// chunks whose entire index range has been consumed (min_idx >>
// CHUNK_SHIFT) are freed immediately via Vec::new(). This keeps
// peak memory close to max(input_size, output_size) rather than
// input_size + output_size.
//
// Progress: emits a per-match progress bar (one tick per match).

pub struct States {
    pub scores: Vec<Vec<u128>>,
    pub weights: Vec<Vec<u64>>,
    pub total_states: usize,
}

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

    pub fn matches(&self) -> &[(usize, usize)] {
        &self.matches
    }

    pub fn remaining_match_count(&self) -> usize {
        self.matches.len()
    }

    pub fn total_scenarios(&self) -> u64 {
        pow_u64(self.base, self.remaining_match_count())
    }

    /// == CORE MERGE LOGIC ===================================================
    /// Merges three derived sequences (A-wins, B-wins, NR) into one
    /// sorted output, combining weights for duplicate scores.
    fn process_next_match(
        &self,
        mut scores: Vec<Vec<u128>>,
        mut weights: Vec<Vec<u64>>,
        total_states: usize,
        a: usize,
        b: usize,
    ) -> States {
        // Chunk size is chosen so a full chunk pair stays within typical
        // 8-32 MB L3 cache.
        // scores (16 B) + weights (8 B) = 24 B/state, so 2^17 × 24 = 3 MB
        // >> CHUNK_SHIFT --> chunk index (fast division)
        // & CHUNK_MASK   --> position within chunk (fast modulo)
        const CHUNK_SHIFT: usize = 17;
        const CHUNK_SIZE: usize = 1 << CHUNK_SHIFT;
        const CHUNK_MASK: usize = CHUNK_SIZE - 1;

        let expected_len = (total_states * 15) / 10;
        let expected_chunks = (expected_len / CHUNK_SIZE) + 1;

        let mut next_scores = Vec::with_capacity(expected_chunks);
        let mut next_weights = Vec::with_capacity(expected_chunks);

        // Local stack buffers for writes to bypass double-indirection
        let mut curr_scores = Vec::with_capacity(CHUNK_SIZE);
        let mut curr_weights = Vec::with_capacity(CHUNK_SIZE);

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

        // Separate variables for chunk and local indices to help LLVM elide bounds checks
        let mut chunk_a = 0;
        let mut local_a = 0;
        let mut chunk_b = 0;
        let mut local_b = 0;
        let mut chunk_nr = idx_nr >> CHUNK_SHIFT;
        let mut local_nr = idx_nr & CHUNK_MASK;

        let len = total_states;
        let mut next_len = 0;
        let mut last_freed_chunk = 0;

        // Cache the initial values before the loop
        let mut val_a = if idx_a < len {
            scores[chunk_a][local_a] + delta_a
        } else {
            u128::MAX
        };
        let mut val_b = if idx_b < len {
            scores[chunk_b][local_b] + delta_b
        } else {
            u128::MAX
        };
        let mut val_nr = if idx_nr < len {
            scores[chunk_nr][local_nr] + delta_nr
        } else {
            u128::MAX
        };

        // Define the macro to process the winning stream
        macro_rules! advance_stream {
            ($val:expr, $idx:ident, $local:ident, $chunk:ident, $w:ident, $min_val:expr, $delta:expr) => {
                if $val == $min_val {
                    $w += weights[$chunk][$local];
                    $idx += 1;
                    $local += 1;
                    // Help LLVM elide bounds checks by resetting local counter
                    if $local == CHUNK_SIZE {
                        $chunk += 1;
                        $local = 0;
                    }
                    $val = if $idx < len {
                        scores[$chunk][$local] + $delta
                    } else {
                        u128::MAX
                    };
                }
            };
        }

        while idx_a < len || idx_b < len || idx_nr < len {
            let min_idx = idx_a.min(idx_b).min(idx_nr);
            let safe_to_free_chunk = min_idx >> CHUNK_SHIFT;
            while last_freed_chunk < safe_to_free_chunk {
                scores[last_freed_chunk] = Vec::new();
                weights[last_freed_chunk] = Vec::new();
                last_freed_chunk += 1;
            }

            // Use the cached variables to find the minimum
            let min_val = val_a.min(val_b).min(val_nr);
            let mut w = 0;

            // Only fetch from memory and recalculate if this specific stream won
            // Call the macro for each stream
            advance_stream!(val_a, idx_a, local_a, chunk_a, w, min_val, delta_a);
            advance_stream!(val_b, idx_b, local_b, chunk_b, w, min_val, delta_b);
            advance_stream!(val_nr, idx_nr, local_nr, chunk_nr, w, min_val, delta_nr);

            // Write to local buffers, completely avoiding outer Vec bounds checks
            if curr_scores.len() == CHUNK_SIZE {
                next_scores.push(curr_scores);
                next_weights.push(curr_weights);
                curr_scores = Vec::with_capacity(CHUNK_SIZE);
                curr_weights = Vec::with_capacity(CHUNK_SIZE);
            }

            curr_scores.push(min_val);
            curr_weights.push(w);
            next_len += 1;
        }

        // Push any remaining elements to the outer Vec
        if !curr_scores.is_empty() {
            next_scores.push(curr_scores);
            next_weights.push(curr_weights);
        }

        States {
            scores: next_scores,
            weights: next_weights,
            total_states: next_len,
        }
    }

    pub fn simulate_forward(
        &self,
        branch_initial_state: StandingState,
        matches: &[(usize, usize)],
        term: &Terminal,
        global_start_time: Instant,
        match_offset: usize,
    ) -> States {
        let phase_start = Instant::now();
        let total_matches = matches.len() + match_offset;
        let mut total_states = 1;
        let mut scores: Vec<Vec<u128>> = vec![vec![branch_initial_state.score]];
        let mut weights: Vec<Vec<u64>> = vec![vec![1]];

        if term.interactive {
            draw_progress(
                ProgressPhase::DpSimulating {
                    match_idx: match_offset,
                    total_matches,
                    state_count: total_states,
                },
                term,
                global_start_time,
                phase_start,
            );
        }

        for (idx, &(a, b)) in matches.iter().enumerate() {
            let states = self.process_next_match(scores, weights, total_states, a, b);
            scores = states.scores;
            weights = states.weights;
            total_states = states.total_states;

            if term.interactive {
                draw_progress(
                    ProgressPhase::DpSimulating {
                        match_idx: match_offset + idx + 1,
                        total_matches,
                        state_count: total_states,
                    },
                    term,
                    global_start_time,
                    phase_start,
                );
            }
        }

        States {
            scores,
            weights,
            total_states,
        }
    }

    pub fn classify_states_parallel(
        &self,
        states: States,
        term: &Terminal,
        global_start_time: Instant,
    ) -> Counts {
        let num_threads = determine_num_threads();
        let total_states = states.total_states;
        let chunk_pairs = states.scores.into_iter().zip(states.weights);
        let chunk_iter = Arc::new(Mutex::new(chunk_pairs));
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

                    let (score_chunk, weight_chunk) = match chunk_opt {
                        Some(c) => c,
                        None => break, // No more chunks, thread can exit
                    };

                    let chunk_len = score_chunk.len();

                    // Classify every state in the chunk
                    for i in 0..chunk_len {
                        let score = score_chunk[i];
                        let w = weight_chunk[i];
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
            let phase_start = Instant::now();
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
                        phase_start,
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
        let states =
            self.simulate_forward(branch_initial_state, matches, term, global_start_time, 1);
        // Print a newline so the Simulating Progress bar is saved and Classifying gets a fresh line
        println!();
        self.classify_states_parallel(states, term, global_start_time)
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
}
