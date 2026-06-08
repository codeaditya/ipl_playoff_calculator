use std::io::{self, Write};
use std::time::Instant;

use terminal_size::{Width, terminal_size};

use crate::utils::{current_rss_bytes, fmt_mem, format_with_commas};

// ================================================================
// TERMINAL COLORS
// ================================================================

#[derive(Clone, Copy)]
pub struct Colors {
    pub clear: &'static str,
    pub bold: &'static str,
    pub reset: &'static str,
    pub cyan: &'static str,
    pub yellow: &'static str,
    pub green: &'static str,
    pub magenta: &'static str,
}

impl Colors {
    pub fn new(enabled: bool) -> Self {
        if enabled {
            Self {
                clear: "\x1b[2K",
                bold: "\x1b[1m",
                reset: "\x1b[0m",
                cyan: "\x1b[36m",
                yellow: "\x1b[33m",
                green: "\x1b[32m",
                magenta: "\x1b[35m",
            }
        } else {
            Self {
                clear: "",
                bold: "",
                reset: "",
                cyan: "",
                yellow: "",
                green: "",
                magenta: "",
            }
        }
    }
}

// ================================================================
// TERMINAL CONTEXT
// ================================================================

#[derive(Clone, Copy)]
pub struct Terminal {
    pub colors: Colors,
    pub interactive: bool,
}

impl Terminal {
    pub fn new(interactive: bool) -> Self {
        Self {
            colors: Colors::new(interactive),
            interactive,
        }
    }
}

// ================================================================
// PROGRESS BAR
// ================================================================

// Progress bar layout constants
const PROGRESS_DEFAULT_BAR: usize = 20;
const PROGRESS_MIN_BAR: usize = 5;
const PROGRESS_LABEL: &str = "Progress: "; // 10 visible chars
const PROGRESS_LABEL_LEN: usize = PROGRESS_LABEL.len();
const PROGRESS_BRACKETS: usize = 2; // '[' + ']'
const PROGRESS_BAR_SEP: usize = 1; // space after ']'

pub enum ProgressPhase {
    Dfs {
        done: u64,
        total: u64,
    },
    DpSimulating {
        match_idx: usize,
        total_matches: usize,
        state_count: usize,
    },
    DpClassifying {
        states_done: usize,
        total_states: usize,
    },
}

pub fn draw_progress(
    phase: ProgressPhase,
    term: &Terminal,
    start_time: Instant,
    phase_start: Instant,
) {
    let colors = &term.colors;
    let term_width = terminal_width();

    let mem_str = current_rss_bytes()
        .map(fmt_mem)
        .unwrap_or_else(|| "N/A".to_string());

    let elapsed = start_time.elapsed().as_secs_f64();
    let phase_elapsed = phase_start.elapsed().as_secs_f64();
    let (pct, info) = phase_info(&phase, colors);
    let eta = if pct > 0.0 && pct < 1.0 {
        (phase_elapsed / pct) - phase_elapsed
    } else {
        0.0
    };

    // Plain-text suffix used only for visible width measurement.
    let suffix_plain = format!(
        "{:>5.1}% | {} | RAM: {} | Elapsed: {:.1}s | ETA: {:.1}s",
        pct * 100.0,
        info.plain,
        mem_str,
        elapsed,
        eta,
    );

    let layout = resolve_bar_layout(term_width, suffix_plain.len());
    let suffix = render_progress_suffix(pct, &info, &mem_str, elapsed, eta, colors);

    let mut line = String::with_capacity(term_width + 128 /* ANSI slack */);
    if layout.show_label {
        line.push_str(&format!(
            "{bold}{cyan}Progress:{reset} ",
            bold = colors.bold,
            cyan = colors.cyan,
            reset = colors.reset,
        ));
    }
    if layout.bar_width > 0 {
        line.push_str(&render_progress_bar(pct, layout.bar_width, colors));
    }
    line.push_str(&suffix);

    // Tier 4: hard-truncate if the suffix alone still overflows.
    let visible_len = if layout.show_label {
        PROGRESS_LABEL_LEN
    } else {
        0
    } + if layout.bar_width > 0 {
        PROGRESS_BRACKETS + layout.bar_width + PROGRESS_BAR_SEP
    } else {
        0
    } + suffix_plain.len();

    let output = if visible_len > term_width {
        truncate_ansi(&line, term_width, colors)
    } else {
        line
    };

    print!("\r{clear}{output}", clear = colors.clear, output = output);
    io::stdout().flush().unwrap();
}

/// Colored + plain versions of the phase info string.
struct PhaseInfo {
    colored: String,
    plain: String,
}

/// Layout decision for a given terminal width.
struct BarLayout {
    show_label: bool,
    bar_width: usize, // 0 = no bar
}

fn terminal_width() -> usize {
    terminal_size()
        .map(|(Width(w), _)| w as usize)
        .unwrap_or(80)
}

/// Decide what to show based on available terminal width.
fn resolve_bar_layout(term_width: usize, suffix_plain_len: usize) -> BarLayout {
    // Tier widths (visible characters only, no ANSI):
    //   Tier 0: "Progress: [bar(20)] PP.P% | ..."   (full)
    //   Tier 1: "[bar(20)] PP.P% | ..."             (no label)
    //   Tier 2: "[bar(5..20)] PP.P% | ..."          (no label, shrinking bar)
    //   Tier 3: "PP.P% | ..."                       (no bar)
    //   Tier 4: truncate to term_width
    let tier0 = PROGRESS_LABEL_LEN
        + PROGRESS_BRACKETS
        + PROGRESS_DEFAULT_BAR
        + PROGRESS_BAR_SEP
        + suffix_plain_len;
    let tier1 = PROGRESS_BRACKETS + PROGRESS_DEFAULT_BAR + PROGRESS_BAR_SEP + suffix_plain_len;
    let tier2 = PROGRESS_BRACKETS + PROGRESS_MIN_BAR + PROGRESS_BAR_SEP + suffix_plain_len;

    if term_width >= tier0 {
        BarLayout {
            show_label: true,
            bar_width: PROGRESS_DEFAULT_BAR,
        }
    } else if term_width >= tier1 {
        BarLayout {
            show_label: false,
            bar_width: PROGRESS_DEFAULT_BAR,
        }
    } else if term_width >= tier2 {
        let available =
            term_width.saturating_sub(PROGRESS_BRACKETS + PROGRESS_BAR_SEP + suffix_plain_len);
        BarLayout {
            show_label: false,
            bar_width: available.clamp(PROGRESS_MIN_BAR, PROGRESS_DEFAULT_BAR),
        }
    } else {
        // Tier 3 / 4: no bar
        BarLayout {
            show_label: false,
            bar_width: 0,
        }
    }
}

/// Build the colored `[████▓░░░] ` string.
fn render_progress_bar(pct: f64, bar_width: usize, colors: &Colors) -> String {
    let filled = ((pct * bar_width as f64) as usize).min(bar_width);
    let bar: String = (0..bar_width)
        .map(|i| {
            if i < filled {
                '█'
            } else if i == filled {
                '▓'
            } else {
                '░'
            }
        })
        .collect();
    format!("[{}{bar}{}] ", colors.magenta, colors.reset)
}

/// Build the colored suffix: `PP.P% | info | RAM | Elapsed | ETA`.
fn render_progress_suffix(
    pct: f64,
    info: &PhaseInfo,
    mem_str: &str,
    elapsed: f64,
    eta: f64,
    colors: &Colors,
) -> String {
    format!(
        "{bold}{pct:>5.1}%{reset} | {info} | RAM: {ram} | Elapsed: {cyan}{elapsed:.1}s{reset} | ETA: {magenta}{eta:.1}s{reset}",
        bold = colors.bold,
        cyan = colors.cyan,
        reset = colors.reset,
        magenta = colors.magenta,
        pct = pct * 100.0,
        info = info.colored,
        ram = mem_str,
        elapsed = elapsed,
        eta = eta,
    )
}

/// Truncate a string with embedded ANSI escape codes to `max_visible`
/// printable characters. Uses `colors.reset` to avoid color bleed.
fn truncate_ansi(s: &str, max_visible: usize, colors: &Colors) -> String {
    let mut out = String::with_capacity(s.len());
    let mut visible = 0;
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if visible >= max_visible {
            break;
        }
        if ch == '\x1b' {
            // Copy escape sequence verbatim without counting it.
            out.push(ch);
            if chars.peek() == Some(&'[') {
                out.push(chars.next().unwrap()); // '['
                for esc_ch in chars.by_ref() {
                    out.push(esc_ch);
                    if ('\x40'..='\x7e').contains(&esc_ch) {
                        break; // final byte of CSI sequence
                    }
                }
            }
        } else {
            out.push(ch);
            visible += 1;
        }
    }

    out.push_str(colors.reset);
    out
}

/// Extract (pct, PhaseInfo) from the current progress phase.
fn phase_info(phase: &ProgressPhase, colors: &Colors) -> (f64, PhaseInfo) {
    match phase {
        ProgressPhase::Dfs { done, total } => {
            let pct = if *total == 0 {
                1.0
            } else {
                *done as f64 / *total as f64
            };
            let d = format_with_commas(*done);
            let t = format_with_commas(*total);
            (
                pct,
                PhaseInfo {
                    plain: format!("Scenarios: {d}/{t}"),
                    colored: format!(
                        "Scenarios: {c}{d}{r}/{y}{t}{r}",
                        c = colors.cyan,
                        y = colors.yellow,
                        r = colors.reset,
                    ),
                },
            )
        }
        ProgressPhase::DpSimulating {
            match_idx,
            total_matches,
            state_count,
        } => {
            let p = if *total_matches == 0 {
                1.0
            } else {
                *match_idx as f64 / *total_matches as f64
            };
            // A power of 10 roughly maps the exponential state growth of the
            // DP algorithm based on observed cumulative sum data.
            let pct = p.powf(10.0);
            let states = format_with_commas(*state_count as u64);
            (
                pct,
                PhaseInfo {
                    plain: format!("Simulating: {match_idx}/{total_matches} | States: {states}"),
                    colored: format!(
                        "Simulating: {c}{match_idx}{r}/{y}{total_matches}{r} | States: {g}{states}{r}",
                        c = colors.cyan,
                        y = colors.yellow,
                        g = colors.green,
                        r = colors.reset,
                    ),
                },
            )
        }
        ProgressPhase::DpClassifying {
            states_done,
            total_states,
        } => {
            let pct = if *total_states == 0 {
                1.0
            } else {
                *states_done as f64 / *total_states as f64
            };
            let d = format_with_commas(*states_done as u64);
            let t = format_with_commas(*total_states as u64);
            (
                pct,
                PhaseInfo {
                    plain: format!("Classifying: {d}/{t}"),
                    colored: format!(
                        "Classifying: {c}{d}{r}/{y}{t}{r}",
                        c = colors.cyan,
                        y = colors.yellow,
                        r = colors.reset,
                    ),
                },
            )
        }
    }
}
