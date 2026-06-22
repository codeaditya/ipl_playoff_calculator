use std::time::Instant;

use super::cost::estimate_dp_cost;
use super::dfs::DfsSimulator;
use super::dp::DpSimulator;
use crate::models::{
    AllCounts, Counts, ParsedInput, SLOT_A, SLOT_B, SLOT_NR, SLOT_UNSET, StandingState,
};
use crate::reporter::Reporter;
use crate::terminal::Terminal;
use crate::utils::{get_free_system_ram_mb, get_usable_ram_mb, pow_u64};

pub struct AutoOptimizedStrategy {
    pub remaining: usize,
    pub optimal_dp_size: usize,
    pub free_ram_mb: f64,
    pub usable_ram_mb: f64,
    pub est_peak_ram_mb: f64,
    pub est_compute_time: f64,
}

impl AutoOptimizedStrategy {
    pub fn for_remaining(remaining: usize, base: u64) -> AutoOptimizedStrategy {
        let free_ram_mb = get_free_system_ram_mb();
        let usable_ram_mb = get_usable_ram_mb(free_ram_mb);

        let mut optimal_dp_size = 1;
        let mut est_peak_ram_mb = 0.0;
        let mut est_compute_time = 0.0;

        // OPTIMAL STRATEGY: Greedy Max-DP
        for d in (1..=remaining).rev() {
            let (ram_req_mb, time_req) = if d == remaining {
                estimate_dp_cost(d - 1, base)
            } else {
                estimate_dp_cost(d, base)
            };
            if ram_req_mb <= usable_ram_mb {
                let dfs_branches = (base as f64).powi((remaining - d) as i32);
                optimal_dp_size = d;
                est_peak_ram_mb = ram_req_mb;
                est_compute_time = if optimal_dp_size == remaining {
                    time_req
                } else {
                    dfs_branches * time_req / base as f64
                };
                break;
            }
        }

        AutoOptimizedStrategy {
            remaining,
            optimal_dp_size,
            free_ram_mb,
            usable_ram_mb,
            est_peak_ram_mb,
            est_compute_time,
        }
    }
}

#[derive(Clone)]
pub struct AutoSimulator {
    matches: Vec<(usize, usize)>,
    base: u64,
    dp_simulator: DpSimulator,
    dfs_simulator: DfsSimulator,
}

impl AutoSimulator {
    pub fn new(parsed: &ParsedInput, allow_no_results: bool) -> Self {
        Self {
            matches: parsed.matches.clone(),
            base: if allow_no_results { 3 } else { 2 },
            dp_simulator: DpSimulator::new(parsed, allow_no_results),
            dfs_simulator: DfsSimulator::new(parsed, allow_no_results),
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

    fn run_dp_branch(
        &self,
        start_state: StandingState,
        remaining_matches: &[(usize, usize)],
        term: &Terminal,
    ) -> Counts {
        let start_time = Instant::now();
        let states =
            self.dp_simulator
                .simulate_forward(start_state, remaining_matches, term, start_time, 0);
        println!();
        self.dp_simulator
            .classify_states_parallel(states, term, start_time)
    }

    pub fn run(
        &self,
        initial_state: &StandingState,
        reporter: &Reporter,
        term: &Terminal,
    ) -> AllCounts {
        let remaining = self.matches.len();
        if remaining == 0 {
            return self.dp_simulator.run(initial_state, reporter, term);
        }

        let strategy = AutoOptimizedStrategy::for_remaining(remaining, self.base);
        let split_depth = remaining - strategy.optimal_dp_size;

        reporter.print_auto_optimized_strategy(&strategy);

        if split_depth == 0 {
            println!(
                "{}Auto Optimizer: Fitting entirely in RAM. Falling back to Pure DP.{}\n",
                reporter.colors().green,
                reporter.colors().reset
            );
            return self.dp_simulator.run(initial_state, reporter, term);
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
        self.dfs_simulator
            .build_tasks_from(0, split_depth, *initial_state, SLOT_UNSET, &mut tasks);

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

            let result_counts = self.run_dp_branch(task.state, dp_matches, term);

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
