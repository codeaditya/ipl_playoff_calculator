pub mod auto;
pub mod dfs;
pub mod dp;

use self::auto::AutoSimulator;
use self::dfs::DfsSimulator;
use self::dp::DpSimulator;
use crate::models::{Algorithm, AppError, StandingState};
use crate::reporter::Reporter;
use crate::terminal::Terminal;
use crate::utils::{determine_num_threads, pow_u64};

pub enum SimulationRunner {
    Dfs(DfsSimulator),
    Dp(DpSimulator),
    Auto(AutoSimulator),
}

impl SimulationRunner {
    pub fn run_simulation(
        &self,
        initial_state: &StandingState,
        reporter: &Reporter,
        term: &Terminal,
    ) -> Result<(), AppError> {
        let (remaining, base_val, total_scenarios) = match self {
            SimulationRunner::Dfs(d) => (d.remaining_match_count(), d.base(), d.total_scenarios()),
            SimulationRunner::Dp(d) => (d.remaining_match_count(), d.base(), d.total_scenarios()),
            SimulationRunner::Auto(h) => (h.remaining_match_count(), h.base(), h.total_scenarios()),
        };

        check_u64_overflow(reporter.seat_scale(), remaining, base_val);
        reporter.print_current_standings();
        reporter.print_simulation_header(
            match self {
                SimulationRunner::Dfs(_) => Algorithm::Dfs,
                SimulationRunner::Dp(_) => Algorithm::Dp,
                SimulationRunner::Auto(_) => Algorithm::Auto,
            },
            reporter.completed_matches(),
            remaining,
            base_val,
            total_scenarios,
            determine_num_threads(),
        );

        if remaining == 0 {
            println!("No remaining matches to simulate.");
            return Ok(());
        }

        let all_counts = match self {
            SimulationRunner::Dfs(d) => d.run(initial_state, reporter, term),
            SimulationRunner::Dp(d) => d.run(initial_state, reporter, term),
            SimulationRunner::Auto(h) => h.run(initial_state, reporter, term),
        };

        reporter.print_probability_results(&all_counts, total_scenarios, base_val);

        Ok(())
    }
}

/// Exits with a friendly message if the scenario count would overflow u64 counters.
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
        "Too many scenarios to compute safely with {} matches remaining.\n\
         \n\
         Maximum supported remaining matches:\n\
           Without --allow-no-results : {}\n\
           With    --allow-no-results : {}\n\
         \n\
         Try after more matches are completed.",
        remaining,
        safe_for_base(2),
        safe_for_base(3),
    );
    std::process::exit(0);
}
