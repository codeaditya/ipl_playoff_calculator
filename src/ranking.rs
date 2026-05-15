use crate::models::{Counts, MAX_TEAMS, StandingState, TEAM_MASK, TEAM_SHIFTS};

#[derive(Clone)]
pub struct Ranker {
    pub team_count: usize,
    pub seat_scale: u64,
}

impl Ranker {
    pub fn new(team_count: usize, seat_scale: u64) -> Self {
        Self {
            team_count,
            seat_scale,
        }
    }

    #[inline]
    pub fn classify(&self, state: &StandingState, counts: &mut Counts) {
        // --- OPTIMIZED SCORE EXTRACTION ---
        // Extract all team scores in one pass.
        // Using a fixed-size array and const shifts lets the compiler
        // see the full unroll and emit vectorized code where possible.
        let raw = state.score;
        let mut scores = [0u16; MAX_TEAMS];

        // This loop has no data dependency between iterations
        // (each reads `raw` independently) — the compiler can unroll
        // and/or vectorize this on any target.
        for (i, score) in scores.iter_mut().enumerate().take(self.team_count) {
            *score = ((raw >> TEAM_SHIFTS[i]) & TEAM_MASK) as u16;
        }

        // Only consider active teams — zero out the rest explicitly
        // so sort_teams sees clean data without a branch.
        for score in scores.iter_mut().take(MAX_TEAMS).skip(self.team_count) {
            *score = 0;
        }

        let order = sort_teams(self.team_count, &scores);

        let mut start = 0;
        let mut placed_above = 0;

        while start < self.team_count {
            let mut end = start + 1;
            let score_val = scores[order[start]];

            // Group teams with identical points and wins
            while end < self.team_count && scores[order[end]] == score_val {
                end += 1;
            }

            let group_len = end - start;
            let group_len_u64 = group_len as u64;

            // --- TOP 2 Cutoff ---
            let spots_top2 = if placed_above >= 2 {
                0
            } else {
                (2 - placed_above).min(group_len)
            };
            let units_top2 = if spots_top2 == 0 {
                0
            } else {
                (spots_top2 as u64 * self.seat_scale) / group_len_u64
            };

            // --- TOP 4 Cutoff ---
            let spots_top4 = if placed_above >= 4 {
                0
            } else {
                (4 - placed_above).min(group_len)
            };
            let units_top4 = if spots_top4 == 0 {
                0
            } else {
                (spots_top4 as u64 * self.seat_scale) / group_len_u64
            };

            // Assign stats directly to the combined counts struct
            for &team in order.iter().take(end).skip(start) {
                if units_top2 > 0 {
                    counts.top2_good_nrr_units[team] += units_top2;
                }
                if spots_top2 == group_len {
                    counts.top2_pts[team] += 1;
                }
                if units_top4 > 0 {
                    counts.top4_good_nrr_units[team] += units_top4;
                }
                if spots_top4 == group_len {
                    counts.top4_pts[team] += 1;
                }
            }

            start = end;
            placed_above += group_len;
        }
    }
}

pub fn sort_teams(team_count: usize, scores: &[u16; MAX_TEAMS]) -> [usize; MAX_TEAMS] {
    let mut order = [0usize; MAX_TEAMS];
    for (i, slot) in order.iter_mut().enumerate().take(team_count) {
        *slot = i;
    }
    for i in 1..team_count {
        let key = order[i];
        let key_score = scores[key];
        let mut j = i;
        // Using while with an explicit condition the compiler can
        // convert to a cmov (conditional move) instead of a branch
        // eliminating branch mispredictions for small N
        while j > 0 && scores[order[j - 1]] < key_score {
            order[j] = order[j - 1];
            j -= 1;
        }
        order[j] = key;
    }
    order
}
