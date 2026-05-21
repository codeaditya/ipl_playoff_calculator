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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TEAM_BITS;
    use crate::utils::seat_scale_for_team_count;

    fn packed_score(points: u16, wins: u16) -> u16 {
        (points << 4) | wins
    }

    #[test]
    fn test_sort_teams_all_different() {
        let mut scores = [0u16; MAX_TEAMS];
        scores[0] = packed_score(18, 9);
        scores[1] = packed_score(12, 6);
        scores[2] = packed_score(16, 8);
        scores[3] = packed_score(10, 5);
        let team_count = 4;

        let order = sort_teams(team_count, &scores);
        assert_eq!(order[0], 0);
        assert_eq!(order[1], 2);
        assert_eq!(order[2], 1);
        assert_eq!(order[3], 3);
    }

    #[test]
    fn test_sort_teams_with_ties() {
        let mut scores = [0u16; MAX_TEAMS];
        scores[0] = packed_score(18, 9);
        scores[1] = packed_score(14, 7);
        scores[2] = packed_score(14, 7);
        scores[3] = packed_score(10, 5);
        let team_count = 4;

        let order = sort_teams(team_count, &scores);
        assert_eq!(order[0], 0);
        assert_eq!(order[1], 1);
        assert_eq!(order[2], 2);
        assert_eq!(order[3], 3);
    }

    #[test]
    fn test_sort_teams_all_same() {
        let mut scores = [0u16; MAX_TEAMS];
        for score in scores.iter_mut().take(5) {
            *score = packed_score(14, 7);
        }
        let team_count = 5;

        let order = sort_teams(team_count, &scores);
        for (i, team_order) in order.iter().enumerate().take(5) {
            assert_eq!(*team_order, i);
        }
    }

    #[test]
    fn test_sort_teams_single_team() {
        let mut scores = [0u16; MAX_TEAMS];
        scores[0] = packed_score(20, 10);
        let order = sort_teams(1, &scores);
        assert_eq!(order[0], 0);
    }

    #[test]
    fn test_ranker_no_ties() {
        let team_count = 4;
        let seat_scale = seat_scale_for_team_count(team_count);
        let ranker = Ranker::new(team_count, seat_scale);
        let state = StandingState {
            score: (packed_score(18, 9) as u128)
                | (packed_score(16, 8) as u128) << TEAM_BITS
                | (packed_score(12, 6) as u128) << (2 * TEAM_BITS)
                | (packed_score(10, 5) as u128) << (3 * TEAM_BITS),
        };

        let mut counts = Counts::default();
        ranker.classify(&state, &mut counts);

        assert_eq!(counts.top2_pts[0], 1);
        assert_eq!(counts.top2_pts[1], 1);
        assert_eq!(counts.top2_pts[2], 0);
        assert_eq!(counts.top2_pts[3], 0);

        assert_eq!(counts.top4_pts[0], 1);
        assert_eq!(counts.top4_pts[1], 1);
        assert_eq!(counts.top4_pts[2], 1);
        assert_eq!(counts.top4_pts[3], 1);
    }

    #[test]
    fn test_ranker_tie_at_top2_boundary() {
        let team_count = 4;
        let seat_scale = seat_scale_for_team_count(team_count);
        let ranker = Ranker::new(team_count, seat_scale);
        let state = StandingState {
            score: (packed_score(18, 9) as u128)
                | (packed_score(14, 7) as u128) << TEAM_BITS
                | (packed_score(14, 7) as u128) << (2 * TEAM_BITS)
                | (packed_score(10, 5) as u128) << (3 * TEAM_BITS),
        };

        let mut counts = Counts::default();
        ranker.classify(&state, &mut counts);

        assert_eq!(counts.top2_pts[0], 1);
        assert_eq!(counts.top2_pts[1], 0);
        assert_eq!(counts.top2_pts[2], 0);
        assert_eq!(counts.top2_good_nrr_units[1], seat_scale / 2);
        assert_eq!(counts.top2_good_nrr_units[2], seat_scale / 2);

        assert_eq!(counts.top4_pts[0], 1);
        assert_eq!(counts.top4_pts[1], 1);
        assert_eq!(counts.top4_pts[2], 1);
        assert_eq!(counts.top4_pts[3], 1);
    }

    #[test]
    fn test_ranker_five_way_tie() {
        let team_count = 5;
        let seat_scale = seat_scale_for_team_count(team_count);
        let ranker = Ranker::new(team_count, seat_scale);
        let mut state = StandingState::default();
        for i in 0..team_count {
            state.score |= (packed_score(14, 7) as u128) << (i * TEAM_BITS);
        }

        let mut counts = Counts::default();
        ranker.classify(&state, &mut counts);

        for i in 0..team_count {
            assert_eq!(counts.top2_pts[i], 0);
            assert_eq!(
                counts.top2_good_nrr_units[i],
                (2 * seat_scale) / team_count as u64
            );
            assert_eq!(counts.top4_pts[i], 0);
            assert_eq!(
                counts.top4_good_nrr_units[i],
                (4 * seat_scale) / team_count as u64
            );
        }
    }

    #[test]
    fn test_ranker_two_groups() {
        let team_count = 4;
        let seat_scale = seat_scale_for_team_count(team_count);
        let ranker = Ranker::new(team_count, seat_scale);
        let state = StandingState {
            score: (packed_score(18, 9) as u128)
                | (packed_score(18, 9) as u128) << TEAM_BITS
                | (packed_score(10, 5) as u128) << (2 * TEAM_BITS)
                | (packed_score(10, 5) as u128) << (3 * TEAM_BITS),
        };

        let mut counts = Counts::default();
        ranker.classify(&state, &mut counts);

        assert_eq!(counts.top2_pts[0], 1);
        assert_eq!(counts.top2_pts[1], 1);
        assert_eq!(counts.top2_pts[2], 0);
        assert_eq!(counts.top2_pts[3], 0);

        assert_eq!(counts.top4_pts[0], 1);
        assert_eq!(counts.top4_pts[1], 1);
        assert_eq!(counts.top4_pts[2], 1);
        assert_eq!(counts.top4_pts[3], 1);
    }

    #[test]
    fn test_ranker_below_top4() {
        let team_count = 6;
        let seat_scale = seat_scale_for_team_count(team_count);
        let ranker = Ranker::new(team_count, seat_scale);
        let state = StandingState {
            score: (packed_score(20, 10) as u128)
                | (packed_score(18, 9) as u128) << TEAM_BITS
                | (packed_score(16, 8) as u128) << (2 * TEAM_BITS)
                | (packed_score(14, 7) as u128) << (3 * TEAM_BITS)
                | (packed_score(12, 6) as u128) << (4 * TEAM_BITS)
                | (packed_score(10, 5) as u128) << (5 * TEAM_BITS),
        };

        let mut counts = Counts::default();
        ranker.classify(&state, &mut counts);

        for i in 0..4 {
            assert_eq!(counts.top4_pts[i], 1);
        }
        assert_eq!(counts.top4_pts[4], 0);
        assert_eq!(counts.top4_pts[5], 0);
        assert_eq!(counts.top4_good_nrr_units[4], 0);
        assert_eq!(counts.top4_good_nrr_units[5], 0);
    }

    #[test]
    fn test_ranker_with_nr_scores() {
        let team_count = 3;
        let seat_scale = seat_scale_for_team_count(team_count);
        let ranker = Ranker::new(team_count, seat_scale);
        let mut state = StandingState::default();
        state.record_win(0);
        state.record_no_result(1, 2);

        let mut counts = Counts::default();
        ranker.classify(&state, &mut counts);

        assert_eq!(state.points(0), 2);
        assert_eq!(state.points(1), 1);
        assert_eq!(state.points(2), 1);

        assert_eq!(counts.top2_pts[0], 1);
        assert_eq!(counts.top2_pts[1], 0);
        assert_eq!(counts.top2_pts[2], 0);
        assert_eq!(counts.top2_good_nrr_units[1], seat_scale / 2);
        assert_eq!(counts.top2_good_nrr_units[2], seat_scale / 2);
    }
}
