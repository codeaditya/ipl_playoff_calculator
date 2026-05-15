use crate::models::{
    Algorithm, AllCounts, Counts, MAX_TEAMS, ParsedInput, StandingState, TEAM_BITS, TEAM_MASK,
};
use crate::ranking::sort_teams;
use crate::simulate::auto::AutoOptimizedStrategy;
use crate::simulate::dp::estimate_dp_cost;
use crate::terminal::{Colors, Terminal};
use crate::utils::{
    fmt_pct, fmt_scaled_pct, format_with_commas, get_free_system_ram_mb, get_usable_ram_mb,
};

pub struct Row {
    pub team: String,
    pub top2_pts: u64,
    pub top2_good_nrr_units: u64,
    pub top4_pts: u64,
    pub top4_good_nrr_units: u64,
}

pub struct TableLayout {
    pub team_w: usize,
    pub pos_w: usize,
    pub stat_w: usize,
    pub pct_w: usize,
}

impl TableLayout {
    pub fn from_input(parsed: &ParsedInput) -> Self {
        Self {
            team_w: parsed
                .team_names
                .iter()
                .map(|name| name.len())
                .max()
                .unwrap_or(6)
                .max(6),
            pos_w: 4,
            stat_w: 4,
            pct_w: 19,
        }
    }
}

pub struct Reporter {
    colors: Colors,
    layout: TableLayout,
    team_names: Vec<String>,
    team_count: usize,
    seat_scale: u64,
    completed_matches: usize,
    matches_played: [u8; MAX_TEAMS],
    losses: [u8; MAX_TEAMS],
    no_results: [u8; MAX_TEAMS],
    initial_state: StandingState,
    next_match: Option<(usize, usize)>,
    allow_no_results: bool,
}

impl Reporter {
    pub fn new(parsed: &ParsedInput, term: &Terminal, allow_no_results: bool) -> Self {
        Self {
            colors: term.colors,
            layout: TableLayout::from_input(parsed),
            team_names: parsed.team_names.clone(),
            team_count: parsed.team_count,
            seat_scale: parsed.seat_scale,
            completed_matches: parsed.completed_matches,
            matches_played: parsed.matches_played,
            losses: parsed.losses,
            no_results: parsed.no_results,
            initial_state: parsed.initial_state,
            next_match: parsed.matches.first().copied(),
            allow_no_results,
        }
    }

    pub fn colors(&self) -> &Colors {
        &self.colors
    }

    pub fn seat_scale(&self) -> u64 {
        self.seat_scale
    }

    pub fn completed_matches(&self) -> usize {
        self.completed_matches
    }

    fn standings_table_width(&self) -> usize {
        // pos_w + team_w + 5 stats columns + 6 space separators
        self.layout.pos_w + self.layout.team_w + (5 * self.layout.stat_w) + 6
    }

    fn probabilities_table_width(&self) -> usize {
        // pos_w + team_w + 4 pct columns + 5 space separators
        self.layout.pos_w + self.layout.team_w + (4 * self.layout.pct_w) + 5
    }

    pub fn print_current_standings(&self) {
        let heading_text = format!(
            "========= Current Standings after Match {} =========",
            self.completed_matches
        );
        println!(
            "{}{:^width$}{}",
            self.colors.cyan,
            heading_text,
            self.colors.reset,
            width = self.standings_table_width()
        );
        println!(
            "{}{:>pos_w$} {:team_w$} {:>stat_w$} {:>stat_w$} {:>stat_w$} {:>stat_w$} {:>stat_w$}{}",
            self.colors.yellow,
            "",
            "Team",
            "M",
            "W",
            "L",
            "NR",
            "Pts",
            self.colors.reset,
            pos_w = self.layout.pos_w,
            team_w = self.layout.team_w,
            stat_w = self.layout.stat_w,
        );
        let mut initial_scores = [0u16; MAX_TEAMS];
        for (i, score) in initial_scores.iter_mut().enumerate().take(self.team_count) {
            *score = ((self.initial_state.score >> (i * TEAM_BITS)) & TEAM_MASK) as u16;
        }
        let current_order = sort_teams(self.team_count, &initial_scores);
        for (idx, &team_idx) in current_order.iter().take(self.team_count).enumerate() {
            println!(
                "{:>pos_w$} {}{:team_w$}{} {:>stat_w$} {:>stat_w$} {:>stat_w$} {:>stat_w$} {:>stat_w$}",
                idx + 1,
                self.colors.green,
                self.team_names[team_idx],
                self.colors.reset,
                self.matches_played[team_idx],
                self.initial_state.wins(team_idx),
                self.losses[team_idx],
                self.no_results[team_idx],
                self.initial_state.points(team_idx),
                pos_w = self.layout.pos_w,
                team_w = self.layout.team_w,
                stat_w = self.layout.stat_w,
            );
        }
    }

    pub fn print_simulation_header(
        &self,
        algorithm: Algorithm,
        completed_matches: usize,
        remaining_matches: usize,
        base: u64,
        total_scenarios: u64,
        num_threads: usize,
    ) {
        println!(
            "\n{}================== League Status ==================={}",
            self.colors.cyan, self.colors.reset
        );
        println!(
            "  {}Matches Completed :{} {}",
            self.colors.magenta, self.colors.reset, completed_matches
        );
        println!(
            "  {}Matches Remaining :{} {}",
            self.colors.magenta, self.colors.reset, remaining_matches
        );
        println!(
            "  {}Outcome Mode      :{} {} per match",
            self.colors.magenta, self.colors.reset, base
        );
        println!(
            "  {}Total Scenarios   :{} {}",
            self.colors.magenta,
            self.colors.reset,
            format_with_commas(total_scenarios)
        );
        println!(
            "  {}Algorithm         :{} {}",
            self.colors.magenta, self.colors.reset, algorithm
        );
        if algorithm == Algorithm::Dfs {
            println!(
                "  {}Threads           :{} {}",
                self.colors.magenta, self.colors.reset, num_threads
            );
        }
        println!();
    }

    pub fn print_auto_optimized_strategy(&self, strategy: &AutoOptimizedStrategy) {
        println!(
            "{}============= Auto Optimizer Strategy =============={}",
            self.colors.cyan, self.colors.reset
        );
        println!(
            "  {}Optimal DP Size   :{} {} matches (DFS will split {})",
            self.colors.magenta,
            self.colors.reset,
            strategy.optimal_dp_size,
            strategy.remaining - strategy.optimal_dp_size
        );
        println!(
            "  {}Free System RAM   :{} {:.0} MB (Usable: {:.0} MB)",
            self.colors.magenta, self.colors.reset, strategy.free_ram_mb, strategy.usable_ram_mb
        );
        println!(
            "  {}Est. Peak RAM     :{} {:.0} MB",
            self.colors.magenta, self.colors.reset, strategy.est_peak_ram_mb
        );
        println!(
            "  {}Est. Compute Time :{} {:.1} seconds",
            self.colors.magenta, self.colors.reset, strategy.est_compute_time
        );
        println!();
    }

    pub fn print_dp_estimate(&self, d: usize, base: u64) {
        let free_ram_mb = get_free_system_ram_mb();
        let usable_ram_mb = get_usable_ram_mb(free_ram_mb);
        let (est_ram_mb, est_time_s) = estimate_dp_cost(d, base);
        println!(
            "{}============= DP Simulation Estimates =============={}",
            self.colors.cyan, self.colors.reset
        );
        println!(
            "  {}Free System RAM   :{} {:.0} MB (Usable: {:.0} MB)",
            self.colors.magenta, self.colors.reset, free_ram_mb, usable_ram_mb
        );
        println!(
            "  {}Est. Peak RAM     :{} {:.0} MB",
            self.colors.magenta, self.colors.reset, est_ram_mb
        );
        println!(
            "  {}Est. Compute Time :{} {:.1} seconds",
            self.colors.magenta, self.colors.reset, est_time_s
        );
        println!();
    }

    pub fn print_current_probabilities_heading(&self) {
        let heading_text = format!(
            "========= Current Probabilities after Match {} =========",
            self.completed_matches
        );
        println!(
            "{}{:^width$}{}",
            self.colors.cyan,
            heading_text,
            self.colors.reset,
            width = self.probabilities_table_width()
        );
    }

    pub fn print_next_match_impact_heading(&self) {
        if let Some((a, b)) = self.next_match {
            let heading_text = format!(
                "========= Impact of Next Match {}: {} vs {} =========",
                self.completed_matches + 1,
                self.team_names[a],
                self.team_names[b]
            );
            println!(
                "{}{:^width$}{}\n",
                self.colors.magenta,
                heading_text,
                self.colors.reset,
                width = self.probabilities_table_width()
            );
        }
    }

    pub fn print_next_match_scenario_heading(&self, title: &str) {
        let heading_text = format!("========= {} =========", title);
        println!(
            "{}{:^width$}{}",
            self.colors.cyan,
            heading_text,
            self.colors.reset,
            width = self.probabilities_table_width()
        );
    }

    pub fn print_results(&self, counts: &Counts, total_scenarios: u64) {
        let mut rows: Vec<Row> = (0..self.team_count)
            .map(|i| Row {
                team: self.team_names[i].clone(),
                top2_pts: counts.top2_pts[i],
                top2_good_nrr_units: counts.top2_good_nrr_units[i],
                top4_pts: counts.top4_pts[i],
                top4_good_nrr_units: counts.top4_good_nrr_units[i],
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
            "{}{:>pos_w$} {:team_w$} {:>pct_w$} {:>pct_w$} {:>pct_w$} {:>pct_w$}{}",
            self.colors.yellow,
            "",
            "Team",
            "Top 2 Pts",
            "Top 2 Pts+Good NRR",
            "Top 4 Pts",
            "Top 4 Pts+Good NRR",
            self.colors.reset,
            pos_w = self.layout.pos_w,
            team_w = self.layout.team_w,
            pct_w = self.layout.pct_w,
        );
        for row in rows {
            println!(
                "{:>pos_w$} {}{:team_w$}{} {:>pct_w$} {:>pct_w$} {:>pct_w$} {:>pct_w$}",
                "",
                self.colors.green,
                row.team,
                self.colors.reset,
                fmt_pct(row.top2_pts, total_scenarios),
                fmt_scaled_pct(row.top2_good_nrr_units, total_scenarios, self.seat_scale),
                fmt_pct(row.top4_pts, total_scenarios),
                fmt_scaled_pct(row.top4_good_nrr_units, total_scenarios, self.seat_scale),
                pos_w = self.layout.pos_w,
                team_w = self.layout.team_w,
                pct_w = self.layout.pct_w,
            );
        }
    }

    pub fn print_probability_results(
        &self,
        all_counts: &AllCounts,
        total_scenarios: u64,
        base: u64,
    ) {
        self.print_current_probabilities_heading();
        self.print_results(&all_counts.overall, total_scenarios);

        if let Some((a, b)) = self.next_match {
            let cond_total = total_scenarios / base;

            println!();
            self.print_next_match_impact_heading();

            let title_a = format!("If {} beats {}", self.team_names[a], self.team_names[b]);
            self.print_next_match_scenario_heading(&title_a);
            self.print_results(&all_counts.if_a_wins, cond_total);
            println!();

            let title_b = format!("If {} beats {}", self.team_names[b], self.team_names[a]);
            self.print_next_match_scenario_heading(&title_b);
            self.print_results(&all_counts.if_b_wins, cond_total);
            println!();

            if self.allow_no_results {
                let title_nr = format!(
                    "If {} vs {} ends in NR",
                    self.team_names[a], self.team_names[b]
                );
                self.print_next_match_scenario_heading(&title_nr);
                self.print_results(&all_counts.if_nr, cond_total);
                println!();
            }
        }
    }
}
