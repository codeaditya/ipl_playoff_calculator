use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crate::models::{
    AllCounts, Counts, PROGRESS_POLL_INTERVAL_MS, ParsedInput, SLOT_A, SLOT_B, SLOT_NR, SLOT_UNSET,
    StandingState, TASKS_PER_THREAD_TARGET, Task,
};
use crate::ranking::Ranker;
use crate::reporter::Reporter;
use crate::terminal::{ProgressPhase, Terminal, draw_progress};
use crate::utils::{determine_num_threads, pow_u64};

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
pub struct DfsSimulator {
    matches: Arc<Vec<(usize, usize)>>,
    ranker: Ranker,
    allow_no_results: bool,
    base: u64,
}

impl DfsSimulator {
    pub fn new(parsed: &ParsedInput, allow_no_results: bool) -> Self {
        Self {
            matches: Arc::new(parsed.matches.clone()),
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

    // == Task building =======================================================
    // Slot is resolved when match 0 is first branched and then propagated
    // unchanged into every descendant task. Tasks that start after match
    // 0 already carry their final slot; the DFS never needs to check it.

    pub fn build_tasks(&self, split_depth: usize, initial_state: StandingState) -> Vec<Task> {
        let capacity = self.task_count_for_depth(split_depth) as usize;
        let mut tasks = Vec::with_capacity(capacity);
        self.build_tasks_from(0, split_depth, initial_state, SLOT_UNSET, &mut tasks);
        tasks
    }

    pub fn build_tasks_from(
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

    // == Per-task execution ==================================================
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

    pub fn run(
        &self,
        initial_state: &StandingState,
        _reporter: &Reporter,
        term: &Terminal,
    ) -> AllCounts {
        if self.remaining_match_count() == 0 {
            let mut counts = Counts::default();
            self.ranker.classify(initial_state, &mut counts);
            let mut all = AllCounts::default();
            all.overall += &counts;
            return all;
        }
        let num_threads = determine_num_threads();
        let split_depth = self.choose_split_depth(num_threads);
        let tasks = self.build_tasks(split_depth, *initial_state);
        let scenarios_per_task = self.scenarios_per_task(split_depth);
        let progress = ProgressTracker::new(self.total_scenarios(), scenarios_per_task);
        let parallel = ParallelDfsSimulator::new(self.clone(), num_threads);
        parallel.run(tasks, &progress, term)
    }
}

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

    fn run(&self, tasks: Vec<Task>, progress: &ProgressTracker, term: &Terminal) -> AllCounts {
        let tasks = Arc::new(tasks);
        let next_task = Arc::new(AtomicUsize::new(0));
        let start_time = Instant::now();
        let handles = self.spawn_workers(tasks, next_task, progress);
        progress.run_ui_loop(term, start_time);
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

pub struct ProgressTracker {
    total_scenarios: u64,
    scenarios_per_task: u64,
    scenarios_done: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl ProgressTracker {
    pub fn new(total_scenarios: u64, scenarios_per_task: u64) -> Self {
        Self {
            total_scenarios,
            scenarios_per_task,
            scenarios_done: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    pub fn counter(&self) -> std::sync::Arc<std::sync::atomic::AtomicU64> {
        std::sync::Arc::clone(&self.scenarios_done)
    }

    pub fn scenarios_per_task(&self) -> u64 {
        self.scenarios_per_task
    }

    pub fn run_ui_loop(&self, term: &Terminal, start_time: Instant) {
        if !term.interactive {
            return;
        }
        let mut last_drawn = u64::MAX;
        loop {
            let done = self
                .scenarios_done
                .load(std::sync::atomic::Ordering::Relaxed)
                .min(self.total_scenarios);
            if done != last_drawn {
                draw_progress(
                    ProgressPhase::Dfs {
                        done,
                        total: self.total_scenarios,
                    },
                    term,
                    start_time,
                    start_time,
                );
                last_drawn = done;
            }
            if done >= self.total_scenarios {
                break;
            }
            std::thread::sleep(Duration::from_millis(PROGRESS_POLL_INTERVAL_MS));
        }
        println!("\n");
    }
}
