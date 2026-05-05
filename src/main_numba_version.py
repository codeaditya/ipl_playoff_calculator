import argparse
import math
import multiprocessing as mp
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from typing import List, Optional, Tuple

import numpy as np
from numba import njit

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

WIN_SCORE_DELTA = (2 << 8) | 1  # 513
NR_SCORE_DELTA = (1 << 8) | 0  # 256


def seat_scale_for_team_count(team_count: int) -> int:
    scale = 1
    for x in range(1, team_count + 1):
        scale = math.lcm(scale, x)
    return scale


# ================================================================
# NUMBA COMPILED CORE (NOGIL)
# ================================================================


@njit
def numba_sort_teams(team_count, score):
    order = np.arange(team_count)
    # Insertion sort for small array
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


@njit
def numba_classify(team_count, seat_scale, score, counts):
    order = numba_sort_teams(team_count, score)
    start = 0
    placed_above = 0

    while start < team_count:
        end = start + 1
        score_val = score[order[start]]

        while end < team_count and score[order[end]] == score_val:
            end += 1

        group_len = end - start
        spots_top2 = 0 if placed_above >= 2 else min(2 - placed_above, group_len)
        units_top2 = 0 if spots_top2 == 0 else (spots_top2 * seat_scale) // group_len

        spots_top4 = 0 if placed_above >= 4 else min(4 - placed_above, group_len)
        units_top4 = 0 if spots_top4 == 0 else (spots_top4 * seat_scale) // group_len

        for idx in range(start, end):
            team = order[idx]
            if units_top2 > 0:
                counts[1, team] += units_top2  # top2_good_nrr_units
                if spots_top2 == group_len:
                    counts[0, team] += 1  # top2_pts
            if units_top4 > 0:
                counts[3, team] += units_top4  # top4_good_nrr_units
                if spots_top4 == group_len:
                    counts[2, team] += 1  # top4_pts

        start = end
        placed_above += group_len


@njit
def numba_dfs_from(
    match_idx,
    num_matches,
    matches,
    state,
    counts,
    team_count,
    seat_scale,
    allow_no_results,
):
    if match_idx == num_matches:
        numba_classify(team_count, seat_scale, state, counts)
        return

    a = matches[match_idx, 0]
    b = matches[match_idx, 1]

    # Team A Wins
    state[a] += 513  # WIN_SCORE_DELTA
    numba_dfs_from(
        match_idx + 1,
        num_matches,
        matches,
        state,
        counts,
        team_count,
        seat_scale,
        allow_no_results,
    )
    state[a] -= 513

    # Team B Wins
    state[b] += 513
    numba_dfs_from(
        match_idx + 1,
        num_matches,
        matches,
        state,
        counts,
        team_count,
        seat_scale,
        allow_no_results,
    )
    state[b] -= 513

    # No Result
    if allow_no_results:
        state[a] += 256  # NR_SCORE_DELTA
        state[b] += 256
        numba_dfs_from(
            match_idx + 1,
            num_matches,
            matches,
            state,
            counts,
            team_count,
            seat_scale,
            allow_no_results,
        )
        state[a] -= 256
        state[b] -= 256


@njit(nogil=True)
def numba_simulate_task(
    next_match,
    slot,
    state_score,
    num_matches,
    matches,
    team_count,
    seat_scale,
    allow_no_results,
):
    counts = np.zeros((4, team_count), dtype=np.uint64)
    state = state_score.copy()

    numba_dfs_from(
        next_match,
        num_matches,
        matches,
        state,
        counts,
        team_count,
        seat_scale,
        allow_no_results,
    )

    # Returns 3D array: [bucket_idx, metric_idx, team_idx]
    # Buckets: 0=Overall, 1=if_a_wins, 2=if_b_wins, 3=if_nr
    res = np.zeros((4, 4, team_count), dtype=np.uint64)
    for m in range(4):
        for t in range(team_count):
            res[0, m, t] = counts[m, t]
            if slot != 0:
                res[slot, m, t] = counts[m, t]
    return res


# Pre-compile warm up to avoid compilation freezing the progress bar timer
def warmup_numba():
    dummy_state = np.zeros(MAX_TEAMS, dtype=np.uint16)
    dummy_matches = np.array([[0, 1]], dtype=np.int32)
    numba_simulate_task(0, 0, dummy_state, 1, dummy_matches, 2, 1, False)


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
# SIMULATION ENGINE
# ================================================================


class Simulator:
    def __init__(self, parsed: ParsedInput, allow_no_results: bool):
        self.matches = parsed.matches
        self.matches_arr = np.array(self.matches, dtype=np.int32)
        self.num_matches = len(self.matches)
        self.team_count = parsed.team_count
        self.seat_scale = parsed.seat_scale
        self.allow_no_results = allow_no_results
        self.base = 3 if allow_no_results else 2

    def total_scenarios(self) -> int:
        return self.base**self.num_matches

    def choose_split_depth(self, num_threads: int) -> int:
        split_depth = 0
        task_count = 1
        while split_depth < self.num_matches and task_count < (
            num_threads * TASKS_PER_THREAD_TARGET
        ):
            split_depth += 1
            task_count *= self.base
        return split_depth

    def scenarios_per_task(self, split_depth: int) -> int:
        return self.base ** (self.num_matches - split_depth)

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


# Global progress state
_thread_counter = 0
_counter_lock = threading.Lock()


def worker_func_numba(task_data, scenarios_per_task: int):
    global _thread_counter
    res = numba_simulate_task(*task_data)
    with _counter_lock:
        _thread_counter += scenarios_per_task
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

        bar_chars = [
            "="
            if i < filled
            else ">"
            if i == filled and done < total_scenarios
            else " "
            for i in range(bar_width)
        ]
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
    ) -> np.ndarray:
        global _thread_counter
        _thread_counter = 0
        total_res = np.zeros((4, 4, MAX_TEAMS), dtype=np.uint64)

        if len(tasks) == 0:
            return total_res

        # Serialize task objects to tuples of standard python types & arrays suitable for Numba
        tasks_data = [
            (
                t.next_match,
                t.slot,
                np.array(t.state.score, dtype=np.uint16),
                self.simulator.num_matches,
                self.simulator.matches_arr,
                self.simulator.team_count,
                self.simulator.seat_scale,
                self.simulator.allow_no_results,
            )
            for t in tasks
        ]

        start_time = time.time()

        if self.num_threads <= 1:
            last_drawn = -1
            for t_data in tasks_data:
                total_res += numba_simulate_task(*t_data)
                _thread_counter += scenarios_per_task
                if interactive and _thread_counter != last_drawn:
                    self.draw_progress(
                        _thread_counter, total_scenarios, start_time, colors
                    )
                    last_drawn = _thread_counter
            if interactive:
                print("\n")
            return total_res

        with ThreadPoolExecutor(max_workers=self.num_threads) as pool:
            futures = [
                pool.submit(worker_func_numba, td, scenarios_per_task)
                for td in tasks_data
            ]

            if interactive:
                last_drawn = -1
                done_count = 0
                while done_count < total_scenarios:
                    with _counter_lock:
                        done_count = min(_thread_counter, total_scenarios)
                    if done_count != last_drawn:
                        self.draw_progress(
                            done_count, total_scenarios, start_time, colors
                        )
                        last_drawn = done_count
                    if done_count >= total_scenarios:
                        break
                    time.sleep(0.1)

                # Final catch up draw
                self.draw_progress(total_scenarios, total_scenarios, start_time, colors)
                print("\n")

            for future in futures:
                total_res += future.result()

        return total_res


# ================================================================
# REPORTING & UTILS
# ================================================================


# [All parsing logic remains exactly same]
def canonical(name: str) -> str:
    return "".join(c.upper() for c in name if c.isalnum())


def parse_match_line(line: str) -> Tuple[str, str]:
    lower = line.lower()
    for splitter in [" vs ", " v ", ","]:
        if splitter in lower:
            pos = lower.find(splitter)
            return line[:pos].strip(), line[pos + len(splitter) :].strip()
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
    raise ValueError(f"Invalid outcome '{outcome_str.strip()}' in line '{line}'.")


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

        def get_or_insert(name):
            key = canonical(name)
            if key in team_map:
                return team_map[key]
            idx = len(parsed.team_names)
            parsed.team_names.append(name)
            team_map[key] = idx
            return idx

        a, b = get_or_insert(a_name), get_or_insert(b_name)

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


class Reporter:
    def __init__(self, parsed: ParsedInput, interactive: bool):
        self.colors = Colors(interactive)
        self.team_w = (
            max([len(n) for n in parsed.team_names] + [6]) if parsed.team_names else 6
        )
        self.pos_w, self.stat_w, self.pct_w = 8, 4, 20

    def print_current_standings(self, parsed: ParsedInput):
        w = self.pos_w + self.team_w + (5 * self.stat_w) + 6
        print(
            f"{self.colors.cyan}{f'========= Current Standings after Match {parsed.completed_matches} =========':^{w}}{self.colors.reset}"
        )
        print(
            f"{self.colors.yellow}{'':>{self.pos_w}} {'Team':<{self.team_w}} {'M':>{self.stat_w}} {'W':>{self.stat_w}} {'L':>{self.stat_w}} {'NR':>{self.stat_w}} {'Pts':>{self.stat_w}}{self.colors.reset}"
        )

        scores = np.array(parsed.initial_state.score, dtype=np.uint16)
        order = numba_sort_teams(parsed.team_count, scores)  # Use numba sort
        for i, team_idx in enumerate(order[: parsed.team_count]):
            print(
                f"{i + 1:>{self.pos_w}} {self.colors.green}{parsed.team_names[team_idx]:<{self.team_w}}{self.colors.reset} {parsed.matches_played[team_idx]:>{self.stat_w}} {parsed.initial_state.wins(team_idx):>{self.stat_w}} {parsed.losses[team_idx]:>{self.stat_w}} {parsed.no_results[team_idx]:>{self.stat_w}} {parsed.initial_state.points(team_idx):>{self.stat_w}}"
            )

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
            f" {self.colors.magenta}Matches Remaining :{self.colors.reset} {simulator.num_matches}"
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
        self,
        parsed: ParsedInput,
        counts_array: np.ndarray,
        total_scenarios: int,
        title: str,
    ):
        w = self.pos_w + self.team_w + (4 * self.pct_w) + 5
        print(
            f"{self.colors.cyan}{f'========= {title} =========':^{w}}{self.colors.reset}"
        )
        print(
            f"{self.colors.yellow}{'':>{self.pos_w}} {'Team':<{self.team_w}} {'Top 2 Pts':>{self.pct_w}} {'Top 2 Pts+Good NRR':>{self.pct_w}} {'Top 4 Pts':>{self.pct_w}} {'Top 4 Pts+Good NRR':>{self.pct_w}}{self.colors.reset}"
        )

        rows = [
            {
                "team": parsed.team_names[i],
                "t2p": counts_array[0, i],
                "t2g": counts_array[1, i],
                "t4p": counts_array[2, i],
                "t4g": counts_array[3, i],
            }
            for i in range(parsed.team_count)
        ]
        rows.sort(
            key=lambda x: (
                -int(x["t2p"]),
                -int(x["t2g"]),
                -int(x["t4p"]),
                -int(x["t4g"]),
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
            print(
                f"{'':>{self.pos_w}} {self.colors.green}{r['team']:<{self.team_w}}{self.colors.reset} {fmt_pct(r['t2p']):>{self.pct_w}} {fmt_scaled(r['t2g']):>{self.pct_w}} {fmt_pct(r['t4p']):>{self.pct_w}} {fmt_scaled(r['t4g']):>{self.pct_w}}"
            )


def run():
    parser = argparse.ArgumentParser(description="IPL Playoff Calculator")
    parser.add_argument("file_path", help="Path to the matches file.")
    parser.add_argument(
        "--allow-no-results", action="store_true", help="Include ties/washouts."
    )
    args = parser.parse_args()

    interactive = sys.stdout.isatty()
    with open(args.file_path, "r") as f:
        parsed = parse_inputs(f.read())

    reporter = Reporter(parsed, interactive)
    simulator = Simulator(parsed, args.allow_no_results)
    num_threads = mp.cpu_count()

    reporter.print_current_standings(parsed)
    reporter.print_simulation_header(
        parsed, simulator, simulator.total_scenarios(), num_threads
    )

    if simulator.num_matches == 0:
        return

    # Compile the Numba backend ahead of time so timing captures only execution speed
    warmup_numba()

    start_time = time.time()

    split_depth = simulator.choose_split_depth(num_threads)
    tasks = simulator.build_tasks(split_depth, parsed.initial_state)
    scenarios_per_task = simulator.scenarios_per_task(split_depth)

    parallel = ParallelSimulator(simulator, num_threads)
    # total_res shape is (4, 4, MAX_TEAMS) representing [Bucket][Metric][Team]
    total_res = parallel.run(
        tasks,
        simulator.total_scenarios(),
        scenarios_per_task,
        interactive,
        reporter.colors,
    )

    elapsed = time.time() - start_time
    print(f"Simulation completed in {elapsed:.2f}s\n")

    reporter.print_results(
        parsed,
        total_res[0],
        simulator.total_scenarios(),
        f"Current Probabilities after Match {parsed.completed_matches}",
    )

    if len(parsed.matches) > 0:
        a, b = parsed.matches[0]
        cond_total = simulator.total_scenarios() // simulator.base
        w = reporter.pos_w + reporter.team_w + (4 * reporter.pct_w) + 5
        print(
            f"\n{reporter.colors.magenta}{f'========= Impact of Next Match {parsed.completed_matches + 1}: {parsed.team_names[a]} vs {parsed.team_names[b]} =========':^{w}}{reporter.colors.reset}\n"
        )

        reporter.print_results(
            parsed,
            total_res[1],
            cond_total,
            f"If {parsed.team_names[a]} beats {parsed.team_names[b]}",
        )
        print()
        reporter.print_results(
            parsed,
            total_res[2],
            cond_total,
            f"If {parsed.team_names[b]} beats {parsed.team_names[a]}",
        )

        if args.allow_no_results:
            print()
            reporter.print_results(
                parsed,
                total_res[3],
                cond_total,
                f"If {parsed.team_names[a]} vs {parsed.team_names[b]} ends in NR",
            )


if __name__ == "__main__":
    run()
