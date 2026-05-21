use std::ops::AddAssign;

// ================================================================
// CONSTANTS
// ================================================================

pub const MAX_TEAMS: usize = 10;
pub const TASKS_PER_THREAD_TARGET: u64 = 512;
pub const PROGRESS_POLL_INTERVAL_MS: u64 = 100;

// Slot constants stored in Task::slot - which branch of match 0 this task covers.
// Using u8 keeps Task small and avoids enum padding.
pub const SLOT_UNSET: u8 = 0; // task starts before match 0 has been branched (split_depth == 0)
pub const SLOT_A: u8 = 1; // A wins match 0
pub const SLOT_B: u8 = 2; // B wins match 0
pub const SLOT_NR: u8 = 3; // no result in match 0

// ================================================================
// ALGORITHM SELECTION
// ================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Algorithm {
    Dfs,
    Dp,
    Auto,
}

impl std::fmt::Display for Algorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Algorithm::Auto => write!(f, "AUTO"),
            Algorithm::Dfs => write!(f, "DFS"),
            Algorithm::Dp => write!(f, "DP"),
        }
    }
}

// ================================================================
// ERROR TYPE
// ================================================================

#[derive(Debug)]
pub enum AppError {
    Parse(String),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Parse(msg) => write!(f, "{}", msg),
        }
    }
}

// ================================================================
// DATA MODELS
// ================================================================

pub const TEAM_BITS: usize = 10;
pub const TEAM_MASK: u128 = 0x3FF;
pub const TEAM_SHIFTS: [u32; MAX_TEAMS] = {
    let mut shifts = [0u32; MAX_TEAMS];
    let mut i = 0;
    while i < MAX_TEAMS {
        shifts[i] = (i * TEAM_BITS) as u32;
        i += 1;
    }
    shifts
};

pub const WIN_SCORE_DELTA: u128 = (2 << 4) | 1; // 33 (2 points, 1 win)
pub const NR_SCORE_DELTA: u128 = 1 << 4; // 16 (1 point, 0 wins)

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct StandingState {
    pub score: u128,
}

impl StandingState {
    #[inline]
    pub fn record_win(&mut self, team: usize) {
        self.score += WIN_SCORE_DELTA << (team * TEAM_BITS);
    }

    #[inline]
    pub fn undo_win(&mut self, team: usize) {
        self.score -= WIN_SCORE_DELTA << (team * TEAM_BITS);
    }

    #[inline]
    pub fn record_no_result(&mut self, a: usize, b: usize) {
        self.score += NR_SCORE_DELTA << (a * TEAM_BITS);
        self.score += NR_SCORE_DELTA << (b * TEAM_BITS);
    }

    #[inline]
    pub fn undo_no_result(&mut self, a: usize, b: usize) {
        self.score -= NR_SCORE_DELTA << (a * TEAM_BITS);
        self.score -= NR_SCORE_DELTA << (b * TEAM_BITS);
    }

    #[inline]
    pub fn points(&self, team: usize) -> u8 {
        (((self.score >> (team * TEAM_BITS)) & TEAM_MASK) >> 4) as u8
    }

    #[inline]
    pub fn wins(&self, team: usize) -> u8 {
        ((self.score >> (team * TEAM_BITS)) & 0xF) as u8
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Counts {
    pub top2_pts: [u64; MAX_TEAMS],
    pub top2_good_nrr_units: [u64; MAX_TEAMS],
    pub top4_pts: [u64; MAX_TEAMS],
    pub top4_good_nrr_units: [u64; MAX_TEAMS],
}

impl AddAssign<&Counts> for Counts {
    fn add_assign(&mut self, other: &Counts) {
        for i in 0..MAX_TEAMS {
            self.top2_pts[i] += other.top2_pts[i];
            self.top2_good_nrr_units[i] += other.top2_good_nrr_units[i];
            self.top4_pts[i] += other.top4_pts[i];
            self.top4_good_nrr_units[i] += other.top4_good_nrr_units[i];
        }
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct AllCounts {
    pub overall: Counts,
    pub if_a_wins: Counts,
    pub if_b_wins: Counts,
    pub if_nr: Counts,
}

impl AddAssign<&AllCounts> for AllCounts {
    fn add_assign(&mut self, other: &AllCounts) {
        self.overall += &other.overall;
        self.if_a_wins += &other.if_a_wins;
        self.if_b_wins += &other.if_b_wins;
        self.if_nr += &other.if_nr;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Task {
    pub next_match: usize,
    pub state: StandingState,
    pub slot: u8, // SLOT_A / SLOT_B / SLOT_NR / SLOT_UNSET
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedInput {
    pub team_names: Vec<String>,
    pub team_count: usize,
    pub seat_scale: u64,
    pub initial_state: StandingState,
    pub matches_played: [u8; MAX_TEAMS],
    pub losses: [u8; MAX_TEAMS],
    pub no_results: [u8; MAX_TEAMS],
    pub matches: Vec<(usize, usize)>,
    pub completed_matches: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(TEAM_BITS, 10);
        assert_eq!(TEAM_MASK, 0x3FF);
        assert_eq!(WIN_SCORE_DELTA, 33);
        assert_eq!(NR_SCORE_DELTA, 16);
        assert_eq!(SLOT_UNSET, 0);
        assert_eq!(SLOT_A, 1);
        assert_eq!(SLOT_B, 2);
        assert_eq!(SLOT_NR, 3);
    }

    #[test]
    fn test_team_shifts() {
        for (i, item) in TEAM_SHIFTS.iter().enumerate().take(MAX_TEAMS) {
            assert_eq!(*item, (i * TEAM_BITS) as u32);
        }
    }

    #[test]
    fn test_standing_state_default() {
        let state = StandingState::default();
        assert_eq!(state.score, 0);
        for i in 0..MAX_TEAMS {
            assert_eq!(state.points(i), 0);
            assert_eq!(state.wins(i), 0);
        }
    }

    #[test]
    fn test_record_win_and_undo_win() {
        let mut state = StandingState::default();
        state.record_win(0);
        assert_eq!(state.points(0), 2);
        assert_eq!(state.wins(0), 1);
        assert_eq!(state.points(1), 0);

        state.undo_win(0);
        assert_eq!(state.points(0), 0);
        assert_eq!(state.wins(0), 0);
    }

    #[test]
    fn test_record_win_multiple_teams() {
        let mut state = StandingState::default();
        state.record_win(3);
        state.record_win(7);
        state.record_win(3);

        assert_eq!(state.points(3), 4);
        assert_eq!(state.wins(3), 2);
        assert_eq!(state.points(7), 2);
        assert_eq!(state.wins(7), 1);
        assert_eq!(state.points(0), 0);
    }

    #[test]
    fn test_record_no_result_and_undo_no_result() {
        let mut state = StandingState::default();
        state.record_no_result(2, 5);
        assert_eq!(state.points(2), 1);
        assert_eq!(state.wins(2), 0);
        assert_eq!(state.points(5), 1);
        assert_eq!(state.wins(5), 0);

        state.undo_no_result(2, 5);
        assert_eq!(state.points(2), 0);
        assert_eq!(state.wins(2), 0);
        assert_eq!(state.points(5), 0);
        assert_eq!(state.wins(5), 0);
    }

    #[test]
    fn test_win_and_nr_round_trip() {
        let mut state = StandingState::default();
        state.record_win(1);
        state.record_no_result(1, 4);
        state.record_win(4);
        state.undo_win(1);
        state.undo_no_result(1, 4);
        state.undo_win(4);
        assert_eq!(state.score, 0);
    }

    #[test]
    fn test_points_wins_encoding() {
        let mut state = StandingState::default();
        state.record_win(0);
        state.record_win(0);
        state.record_no_result(0, 1);

        assert_eq!(state.points(0), 5);
        assert_eq!(state.wins(0), 2);
        assert_eq!(state.points(1), 1);
        assert_eq!(state.wins(1), 0);
    }

    #[test]
    fn test_algorithm_display() {
        assert_eq!(Algorithm::Dfs.to_string(), "DFS");
        assert_eq!(Algorithm::Dp.to_string(), "DP");
        assert_eq!(Algorithm::Auto.to_string(), "AUTO");
    }

    #[test]
    fn test_counts_add_assign() {
        let mut a = Counts::default();
        a.top2_pts[0] = 10;
        a.top4_pts[1] = 20;

        let mut b = Counts::default();
        b.top2_pts[0] = 5;
        b.top4_pts[1] = 15;

        a += &b;
        assert_eq!(a.top2_pts[0], 15);
        assert_eq!(a.top4_pts[1], 35);
    }

    #[test]
    fn test_all_counts_add_assign() {
        let mut a = AllCounts::default();
        a.overall.top2_pts[0] = 10;
        a.if_a_wins.top4_pts[1] = 5;

        let mut b = AllCounts::default();
        b.overall.top2_pts[0] = 3;
        b.if_a_wins.top4_pts[1] = 2;

        a += &b;
        assert_eq!(a.overall.top2_pts[0], 13);
        assert_eq!(a.if_a_wins.top4_pts[1], 7);
    }
}
