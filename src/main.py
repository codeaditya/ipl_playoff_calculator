import argparse
import math
import multiprocessing as mp
import sys
import time
from typing import List, Optional, Tuple

# ================================================================
# TERMINAL COLORS
# ================================================================
BOLD = "\033[1m"
RESET = "\033[0m"
CYAN = "\033[36m"
YELLOW = "\033[33m"
GREEN = "\033[32m"
MAGENTA = "\033[35m"
NO_COLOR = ""


class Colors:
    def __init__(self, enabled: bool):
        self.bold = BOLD if enabled else NO_COLOR
        self.reset = RESET if enabled else NO_COLOR
        self.cyan = CYAN if enabled else NO_COLOR
        self.yellow = YELLOW if enabled else NO_COLOR
        self.green = GREEN if enabled else NO_COLOR
        self.magenta = MAGENTA if enabled else NO_COLOR


# ================================================================
# CONSTANTS
# ================================================================
MAX_TEAMS = 10
TASKS_PER_THREAD_TARGET = 512

SLOT_UNSET = 0
SLOT_A = 1
SLOT_B = 2
SLOT_NR = 3

WIN_SCORE_DELTA = (2 << 8) | 1
NR_SCORE_DELTA = (1 << 8) | 0


# ================================================================
# MATH HELPERS
# ================================================================
def seat_scale_for_team_count(team_count: int) -> int:
    scale = 1
    for x in range(1, team_count + 1):
        scale = math.lcm(scale, x)
    return scale


# ================================================================
# DATA MODELS
# ================================================================


class StandingState:
    __slots__ = ["score"]

    def __init__(self, score: Optional[List[int]] = None):
        self.score = score[:] if score else [0] * MAX_TEAMS

    def record_win(self, team: int):
        self.score[team] += WIN_SCORE_DELTA

    def undo_win(self, team: int):
        self.score[team] -= WIN_SCORE_DELTA

    def record_no_result(self, a: int, b: int):
        self.score[a] += NR_SCORE_DELTA
        self.score[b] += NR_SCORE_DELTA

    def undo_no_result(self, a: int, b: int):
        self.score[a] -= NR_SCORE_DELTA
        self.score[b] -= NR_SCORE_DELTA

    def points(self, team: int) -> int:
        return self.score[team] >> 8

    def wins(self, team: int) -> int:
        return self.score[team] & 0xFF

    def clone(self):
        return StandingState(self.score)


class Counts:
    __slots__ = ["top2_pts", "top2_good_nrr_units", "top4_pts", "top4_good_nrr_units"]

    def __init__(self):
        self.top2_pts = [0] * MAX_TEAMS
        self.top2_good_nrr_units = [0] * MAX_TEAMS
        self.top4_pts = [0] * MAX_TEAMS
        self.top4_good_nrr_units = [0] * MAX_TEAMS

    def add_assign(self, other: "Counts"):
        for i in range(MAX_TEAMS):
            self.top2_pts[i] += other.top2_pts[i]
            self.top2_good_nrr_units[i] += other.top2_good_nrr_units[i]
            self.top4_pts[i] += other.top4_pts[i]
            self.top4_good_nrr_units[i] += other.top4_good_nrr_units[i]


class AllCounts:
    __slots__ = ["overall", "if_a_wins", "if_b_wins", "if_nr"]

    def __init__(self):
        self.overall = Counts()
        self.if_a_wins = Counts()
        self.if_b_wins = Counts()
        self.if_nr = Counts()

    def add_assign(self, other: "AllCounts"):
        self.overall.add_assign(other.overall)
        self.if_a_wins.add_assign(other.if_a_wins)
        self.if_b_wins.add_assign(other.if_b_wins)
        self.if_nr.add_assign(other.if_nr)


class Task:
    __slots__ = ["next_match", "state", "slot"]

    def __init__(self, next_match: int, state: StandingState, slot: int):
        self.next_match = next_match
        self.state = state
        self.slot = slot


class ParsedInput:
    def __init__(self):
        self.team_names: List[str] = []
        self.team_count = 0
        self.seat_scale = 0
        self.initial_state = StandingState()
        self.matches_played = [0] * MAX_TEAMS
        self.losses = [0] * MAX_TEAMS
        self.no_results = [0] * MAX_TEAMS
        self.matches: List[Tuple[int, int]] = []
        self.completed_matches = 0


# ================================================================
# RANKING
# ================================================================


def sort_teams(team_count: int, score: List[int]) -> List[int]:
    order = list(range(team_count))
    for i in range(1, team_count):
        key = order[i]
        j = i
        while j > 0:
            prev = order[j - 1]
            if score[prev] >= score[key]:
                break
            order[j] = prev
            j -= 1
            order[j] = key
    return order


class Ranker:
    __slots__ = ["team_count", "seat_scale"]

    def __init__(self, team_count: int, seat_scale: int):
        self.team_count = team_count
        self.seat_scale = seat_scale

    def classify(self, state: StandingState, counts: Counts):
        order = sort_teams(self.team_count, state.score)
        start = 0
        placed_above = 0

        while start < self.team_count:
            end = start + 1
            score_val = state.score[order[start]]

            while end < self.team_count and state.score[order[end]] == score_val:
                end += 1

            group_len = end - start
            spots_top2 = 0 if placed_above >= 2 else min(2 - placed_above, group_len)
            units_top2 = (
                0 if spots_top2 == 0 else (spots_top2 * self.seat_scale) // group_len
            )

            spots_top4 = 0 if placed_above >= 4 else min(4 - placed_above, group_len)
            units_top4 = (
                0 if spots_top4 == 0 else (spots_top4 * self.seat_scale) // group_len
            )

            for idx in range(start, end):
                team = order[idx]
                if units_top2 > 0:
                    counts.top2_good_nrr_units[team] += units_top2
                    if spots_top2 == group_len:
                        counts.top2_pts[team] += 1
                if units_top4 > 0:
                    counts.top4_good_nrr_units[team] += units_top4
                    if spots_top4 == group_len:
                        counts.top4_pts[team] += 1

            start = end
            placed_above += group_len


# ================================================================
# SIMULATION
# ================================================================


class Simulator:
    def __init__(
        self, matches: List[Tuple[int, int]], ranker: Ranker, allow_no_results: bool
    ):
        self.matches = matches
        self.ranker = ranker
        self.allow_no_results = allow_no_results
        self.base = 3 if allow_no_results else 2

    def remaining_match_count(self) -> int:
        return len(self.matches)

    def total_scenarios(self) -> int:
        return self.base ** self.remaining_match_count()

    def choose_split_depth(self, num_threads: int) -> int:
        split_depth = 0
        task_count = 1
        while split_depth < self.remaining_match_count() and task_count < (
            num_threads * TASKS_PER_THREAD_TARGET
        ):
            split_depth += 1
            task_count *= self.base
        return split_depth

    def scenarios_per_task(self, split_depth: int) -> int:
        return self.base ** (self.remaining_match_count() - split_depth)

    def build_tasks(self, split_depth: int, initial_state: StandingState) -> List[Task]:
        tasks = []
        self._build_tasks_from(0, split_depth, initial_state, SLOT_UNSET, tasks)
        return tasks

    def _build_tasks_from(
        self,
        match_idx: int,
        split_depth: int,
        state: StandingState,
        slot: int,
        tasks: List[Task],
    ):
        if match_idx == split_depth:
            tasks.append(Task(match_idx, state.clone(), slot))
            return

        a, b = self.matches[match_idx]
        slot_a, slot_b, slot_nr = (
            (SLOT_A, SLOT_B, SLOT_NR) if slot == SLOT_UNSET else (slot, slot, slot)
        )

        state.record_win(a)
        self._build_tasks_from(match_idx + 1, split_depth, state, slot_a, tasks)
        state.undo_win(a)

        state.record_win(b)
        self._build_tasks_from(match_idx + 1, split_depth, state, slot_b, tasks)
        state.undo_win(b)

        if self.allow_no_results:
            state.record_no_result(a, b)
            self._build_tasks_from(match_idx + 1, split_depth, state, slot_nr, tasks)
            state.undo_no_result(a, b)

    def simulate_task(self, task: Task) -> AllCounts:
        counts = Counts()
        state = task.state.clone()
        self._dfs_from(task.next_match, state, counts)

        all_counts = AllCounts()
        all_counts.overall.add_assign(counts)
        if task.slot == SLOT_A:
            all_counts.if_a_wins.add_assign(counts)
        elif task.slot == SLOT_B:
            all_counts.if_b_wins.add_assign(counts)
        elif task.slot == SLOT_NR:
            all_counts.if_nr.add_assign(counts)

        return all_counts

    def _dfs_from(self, match_idx: int, state: StandingState, counts: Counts):
        if match_idx == self.remaining_match_count():
            self.ranker.classify(state, counts)
            return

        a, b = self.matches[match_idx]

        state.record_win(a)
        self._dfs_from(match_idx + 1, state, counts)
        state.undo_win(a)

        state.record_win(b)
        self._dfs_from(match_idx + 1, state, counts)
        state.undo_win(b)

        if self.allow_no_results:
            state.record_no_result(a, b)
            self._dfs_from(match_idx + 1, state, counts)
            state.undo_no_result(a, b)


# Global progress counter for worker processes
_worker_counter = None


def worker_init(shared_counter):
    global _worker_counter
    _worker_counter = shared_counter


def worker_func(task: Task, simulator: Simulator, scenarios_per_task: int) -> AllCounts:
    res = simulator.simulate_task(task)
    if _worker_counter is not None:
        with _worker_counter.get_lock():
            _worker_counter.value += scenarios_per_task
    return res


class ParallelSimulator:
    def __init__(self, simulator: Simulator, num_threads: int):
        self.simulator = simulator
        self.num_threads = num_threads

    def draw_progress(
        self, done: int, total_scenarios: int, start_time: float, colors: Colors
    ):
        elapsed = time.time() - start_time
        pct = done / total_scenarios if total_scenarios > 0 else 1.0
        eta = (elapsed / done) * (total_scenarios - done) if done > 0 else 0.0
        bar_width = 40
        filled = min(int(pct * bar_width), bar_width)

        bar_chars = []
        for i in range(bar_width):
            if i < filled:
                bar_chars.append("=")
            elif i == filled and done < total_scenarios:
                bar_chars.append(">")
            else:
                bar_chars.append(" ")
        bar = "".join(bar_chars)

        sys.stdout.write(
            f"\r{colors.cyan}Progress:{colors.reset} [{bar}] {colors.bold}{pct * 100:>5.1f}%{colors.reset} | "
            f"{colors.magenta}Scenarios:{colors.reset} {done:,}/{total_scenarios:,} | "
            f"{colors.yellow}Elapsed:{colors.reset} {elapsed:>5.1f}s | "
            f"{colors.green}ETA:{colors.reset} {eta:>5.1f}s "
        )
        sys.stdout.flush()

    def run(
        self,
        tasks: List[Task],
        total_scenarios: int,
        scenarios_per_task: int,
        interactive: bool,
        colors: Colors,
    ) -> AllCounts:
        total = AllCounts()
        if len(tasks) == 0:
            return total

        start_time = time.time()

        if self.num_threads <= 1:
            done = 0
            last_drawn = -1
            for task in tasks:
                total.add_assign(self.simulator.simulate_task(task))
                done += scenarios_per_task
                if interactive and done != last_drawn:
                    self.draw_progress(done, total_scenarios, start_time, colors)
                    last_drawn = done
            if interactive:
                print("\n")
            return total

        counter = mp.Value("Q", 0)
        pool = mp.Pool(
            processes=self.num_threads, initializer=worker_init, initargs=(counter,)
        )

        # Dispatch tasks
        chunk_size = max(1, len(tasks) // (self.num_threads * 4))
        async_res = pool.starmap_async(
            worker_func,
            [(task, self.simulator, scenarios_per_task) for task in tasks],
            chunksize=chunk_size,
        )

        # Run UI polling loop
        if interactive:
            last_drawn = -1
            while not async_res.ready():
                done = min(counter.value, total_scenarios)
                if done != last_drawn:
                    self.draw_progress(done, total_scenarios, start_time, colors)
                    last_drawn = done
                time.sleep(0.1)  # PROGRESS_POLL_INTERVAL_MS = 100

            # Final draw to 100%
            done = min(counter.value, total_scenarios)
            self.draw_progress(done, total_scenarios, start_time, colors)
            print("\n")

        results = async_res.get()
        pool.close()
        pool.join()

        for res in results:
            total.add_assign(res)

        return total


# ================================================================
# PARSING
# ================================================================


def canonical(name: str) -> str:
    return "".join(c.upper() for c in name if c.isalnum())


def parse_match_line(line: str) -> Tuple[str, str]:
    lower = line.lower()
    if " vs " in lower:
        pos = lower.find(" vs ")
        return line[:pos].strip(), line[pos + 4 :].strip()
    if " v " in lower:
        pos = lower.find(" v ")
        return line[:pos].strip(), line[pos + 3 :].strip()
    if "," in line:
        pos = line.find(",")
        return line[:pos].strip(), line[pos + 1 :].strip()
    raise ValueError(f"Invalid match line: '{line}'. Expected 'Team A vs Team B'")


def apply_completed_outcome(
    outcome_str: str,
    line: str,
    a_name: str,
    b_name: str,
    a: int,
    b: int,
    state: StandingState,
    losses: List[int],
    no_results: List[int],
):
    outcome = canonical(outcome_str.strip())
    if outcome == canonical("NR"):
        state.record_no_result(a, b)
        no_results[a] += 1
        no_results[b] += 1
        return
    if outcome == canonical(a_name):
        state.record_win(a)
        losses[b] += 1
        return
    if outcome == canonical(b_name):
        state.record_win(b)
        losses[a] += 1
        return
    raise ValueError(
        f"Invalid outcome '{outcome_str.strip()}' in line '{line}'. Expected '{a_name}', '{b_name}', or 'NR'"
    )


def parse_inputs(matches_input: str) -> ParsedInput:
    parsed = ParsedInput()
    team_map = {}

    for raw_line in matches_input.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue

        parts = line.split(":", 1)
        match_part = parts[0].strip()
        outcome_part = parts[1] if len(parts) > 1 else None

        a_name, b_name = parse_match_line(match_part)
        if canonical(a_name) == canonical(b_name):
            raise ValueError(f"A team cannot play against itself: '{line}'")

        def get_or_insert(name):
            key = canonical(name)
            if key in team_map:
                return team_map[key]
            idx = len(parsed.team_names)
            if idx >= MAX_TEAMS:
                raise ValueError(f"Too many teams! Max supported is {MAX_TEAMS}")
            parsed.team_names.append(name)
            team_map[key] = idx
            return idx

        a = get_or_insert(a_name)
        b = get_or_insert(b_name)

        if outcome_part is not None:
            parsed.completed_matches += 1
            parsed.matches_played[a] += 1
            parsed.matches_played[b] += 1
            apply_completed_outcome(
                outcome_part.strip(),
                line,
                a_name,
                b_name,
                a,
                b,
                parsed.initial_state,
                parsed.losses,
                parsed.no_results,
            )
        else:
            parsed.matches.append((a, b))

    parsed.team_count = len(parsed.team_names)
    parsed.seat_scale = seat_scale_for_team_count(parsed.team_count)
    return parsed


# ================================================================
# REPORTING & FORMATTING
# ================================================================


class TableLayout:
    def __init__(self, parsed: ParsedInput):
        self.team_w = (
            max([len(n) for n in parsed.team_names] + [6]) if parsed.team_names else 6
        )
        self.pos_w = 8
        self.stat_w = 4
        self.pct_w = 20


class Reporter:
    def __init__(self, parsed: ParsedInput, interactive: bool):
        self.colors = Colors(interactive)
        self.layout = TableLayout(parsed)

    def standings_table_width(self) -> int:
        return self.layout.pos_w + self.layout.team_w + (5 * self.layout.stat_w) + 6

    def probabilities_table_width(self) -> int:
        return self.layout.pos_w + self.layout.team_w + (4 * self.layout.pct_w) + 5

    def print_current_standings(self, parsed: ParsedInput):
        heading = f"========= Current Standings after Match {parsed.completed_matches} ========="
        w = self.standings_table_width()
        print(f"{self.colors.cyan}{heading:^{w}}{self.colors.reset}")

        header = f"{self.colors.yellow}{'':>{self.layout.pos_w}} {'Team':<{self.layout.team_w}} {'M':>{self.layout.stat_w}} {'W':>{self.layout.stat_w}} {'L':>{self.layout.stat_w}} {'NR':>{self.layout.stat_w}} {'Pts':>{self.layout.stat_w}}{self.colors.reset}"
        print(header)

        order = sort_teams(parsed.team_count, parsed.initial_state.score)
        for i, team_idx in enumerate(order[: parsed.team_count]):
            line = f"{i + 1:>{self.layout.pos_w}} {self.colors.green}{parsed.team_names[team_idx]:<{self.layout.team_w}}{self.colors.reset} {parsed.matches_played[team_idx]:>{self.layout.stat_w}} {parsed.initial_state.wins(team_idx):>{self.layout.stat_w}} {parsed.losses[team_idx]:>{self.layout.stat_w}} {parsed.no_results[team_idx]:>{self.layout.stat_w}} {parsed.initial_state.points(team_idx):>{self.layout.stat_w}}"
            print(line)

    def print_simulation_header(
        self,
        parsed: ParsedInput,
        simulator: Simulator,
        total_scenarios: int,
        num_threads: int,
    ):
        print(
            f"\n{self.colors.cyan}========= League Status ========={self.colors.reset}"
        )
        print(
            f" {self.colors.magenta}Matches Completed :{self.colors.reset} {parsed.completed_matches}"
        )
        print(
            f" {self.colors.magenta}Matches Remaining :{self.colors.reset} {simulator.remaining_match_count()}"
        )
        print(
            f" {self.colors.magenta}Outcome Mode      :{self.colors.reset} {simulator.base} per match"
        )
        print(
            f" {self.colors.magenta}Total Scenarios   :{self.colors.reset} {total_scenarios:,}"
        )
        print(
            f" {self.colors.magenta}Threads           :{self.colors.reset} {num_threads}"
        )
        print()

    def print_results(
        self, parsed: ParsedInput, counts: Counts, total_scenarios: int, title: str
    ):
        heading = f"========= {title} ========="
        w = self.probabilities_table_width()
        print(f"{self.colors.cyan}{heading:^{w}}{self.colors.reset}")

        header = f"{self.colors.yellow}{'':>{self.layout.pos_w}} {'Team':<{self.layout.team_w}} {'Top 2 Pts':>{self.layout.pct_w}} {'Top 2 Pts+Good NRR':>{self.layout.pct_w}} {'Top 4 Pts':>{self.layout.pct_w}} {'Top 4 Pts+Good NRR':>{self.layout.pct_w}}{self.colors.reset}"
        print(header)

        rows = []
        for i in range(parsed.team_count):
            rows.append(
                {
                    "team": parsed.team_names[i],
                    "top2_pts": counts.top2_pts[i],
                    "top2_good_nrr": counts.top2_good_nrr_units[i],
                    "top4_pts": counts.top4_pts[i],
                    "top4_good_nrr": counts.top4_good_nrr_units[i],
                }
            )

        rows.sort(
            key=lambda x: (
                -x["top2_pts"],
                -x["top2_good_nrr"],
                -x["top4_pts"],
                -x["top4_good_nrr"],
                x["team"],
            )
        )

        def fmt_pct(num):
            return "-" if num == 0 else f"{(num * 100.0 / total_scenarios):.2f}%"

        def fmt_scaled(units):
            return (
                "-"
                if units == 0
                else f"{(units * 100.0 / (total_scenarios * parsed.seat_scale)):.2f}%"
            )

        for r in rows:
            line = f"{'':>{self.layout.pos_w}} {self.colors.green}{r['team']:<{self.layout.team_w}}{self.colors.reset} {fmt_pct(r['top2_pts']):>{self.layout.pct_w}} {fmt_scaled(r['top2_good_nrr']):>{self.layout.pct_w}} {fmt_pct(r['top4_pts']):>{self.layout.pct_w}} {fmt_scaled(r['top4_good_nrr']):>{self.layout.pct_w}}"
            print(line)


def run():
    parser = argparse.ArgumentParser(description="IPL Playoff Calculator")
    parser.add_argument(
        "file_path", help="Path to the text file containing the schedule."
    )
    parser.add_argument(
        "--allow-no-results",
        action="store_true",
        help="Include ties/washouts (1 pt each) in future outcomes.",
    )

    args = parser.parse_args()
    interactive = sys.stdout.isatty()

    with open(args.file_path, "r") as f:
        matches_input = f.read()

    parsed = parse_inputs(matches_input)
    reporter = Reporter(parsed, interactive)

    ranker = Ranker(parsed.team_count, parsed.seat_scale)
    simulator = Simulator(parsed.matches, ranker, args.allow_no_results)

    num_threads = mp.cpu_count()
    total_scenarios = simulator.total_scenarios()

    reporter.print_current_standings(parsed)
    reporter.print_simulation_header(parsed, simulator, total_scenarios, num_threads)

    if simulator.remaining_match_count() == 0:
        print("No remaining matches to simulate.")
        return

    split_depth = simulator.choose_split_depth(num_threads)
    tasks = simulator.build_tasks(split_depth, parsed.initial_state)
    scenarios_per_task = simulator.scenarios_per_task(split_depth)

    parallel = ParallelSimulator(simulator, num_threads)
    all_counts = parallel.run(
        tasks, total_scenarios, scenarios_per_task, interactive, reporter.colors
    )

    reporter.print_results(
        parsed,
        all_counts.overall,
        total_scenarios,
        f"Current Probabilities after Match {parsed.completed_matches}",
    )
    print()

    if len(parsed.matches) > 0:
        a, b = parsed.matches[0]
        cond_total = total_scenarios // simulator.base

        heading_text = f"========= Impact of Next Match {parsed.completed_matches + 1}: {parsed.team_names[a]} vs {parsed.team_names[b]} ========="
        w = reporter.probabilities_table_width()
        print(f"{reporter.colors.magenta}{heading_text:^{w}}{reporter.colors.reset}\n")

        reporter.print_results(
            parsed,
            all_counts.if_a_wins,
            cond_total,
            f"If {parsed.team_names[a]} beats {parsed.team_names[b]}",
        )
        print()

        reporter.print_results(
            parsed,
            all_counts.if_b_wins,
            cond_total,
            f"If {parsed.team_names[b]} beats {parsed.team_names[a]}",
        )
        print()

        if args.allow_no_results:
            reporter.print_results(
                parsed,
                all_counts.if_nr,
                cond_total,
                f"If {parsed.team_names[a]} vs {parsed.team_names[b]} ends in NR",
            )
            print()


if __name__ == "__main__":
    run()
