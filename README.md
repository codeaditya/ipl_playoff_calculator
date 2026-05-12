# IPL Playoff Calculator

A high-performance CLI tool that simulates all possible remaining match outcomes of an **IPL
season** and computes each team's probability of finishing in the **Top 2** and **Top 4**
positions - both on pure points and on points with Net Run Rate (NRR) tiebreaker.

## Algorithm

The tool implements three simulation strategies, dynamically selecting the best one based on the
number of remaining matches and available system RAM:

| Algorithm | Description                                                                         |
|-----------|-------------------------------------------------------------------------------------|
| **DFS**   | Depth-First Search. Low RAM (<5 MB), multi-threaded via work-stealing. Runtime      |
|           | doubles per additional match. Best for small match counts (less than 25).           |
| **DP**    | Dynamic Programming. Merges identical score states after each match. Very fast when |
|           | RAM is sufficient.                                                                  |
| **Auto**  | **(Default)** Profiles available system RAM and optimally partitions work - uses    |
|           | DFS for initial matches (reducing branching factor) and DP for the rest. Scales     |
|           | gracefully from few to 52+ remaining matches.                                       |

## Usage

```
Usage: ipl_playoff_calculator [--allow-no-results] [--algo=auto|dfs|dp] <matches-file>
```

### Arguments

| Argument             | Description                                                              |
|----------------------|--------------------------------------------------------------------------|
| `<matches-file>`     | Path to the text file containing the schedule.                           |
| `--allow-no-results` | Include ties/washouts (1 pt each) as possible future outcomes.           |
| `--algo=dfs`         | DFS simulation: low RAM, slower for large match counts.                  |
| `--algo=dp`          | DP simulation: fast for large match counts, high RAM usage.              |
| `--algo=auto`        | **(Default)** Dynamically scales between pure DP and Hybrid DFS-DP.      |

### Dev Only

| Flag             | Description                                                                  |
|------------------|------------------------------------------------------------------------------|
| `--calibrate-dp` | Run calibration to measure DP state counts, RAM usage, and compute time per  |
|                  | depth. Used to tune the Auto mode cost model.                                |

### Examples

```bash
# Run with current season data (automatically optimizes the best algo for your machine)
./ipl_playoff_calculator matches.txt

# Run using DFS (good for < 25 remaining matches)
./ipl_playoff_calculator --algo=dfs matches.txt

# Run using DP (good for higher remaining matches - might OOM crash if you do not have enough RAM)
./ipl_playoff_calculator --algo=dp matches.txt
```

### Match File Format

```
# Completed match:
CSK vs MI : CSK

# No Result (tie/washout):
KKR vs PBKS : NR

# Upcoming match:
RCB vs KKR
```

## Try it Online (Kaggle)

A [Kaggle Notebook](https://www.kaggle.com/code/codeadi007/ipl-playoff-calculator-demo) is
available to demo the calculator without building from source. It runs the Rust binary on a Linux
VM and includes preloaded match data files.

Kaggle provides **up to 30 GB RAM** to registered users (free account required), which is
especially useful for the DP algorithm when the number of remaining matches is high - the auto mode
can leverage the full RAM to run larger DP batches, drastically reducing simulation time.

## Build

### Prerequisites

- **Podman** (for containerized cross-platform builds)

### Containerized Cross-Platform Build

```bash
./build.sh --setup           # Set up builder container images
./build.sh --linux           # Build Linux binary
./build.sh --windows         # Build Windows binary (cross-compiled)
```

Binaries are placed in `dist/`.

## License

MIT
