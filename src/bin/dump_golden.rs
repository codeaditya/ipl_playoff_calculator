use std::fs;

use ipl_playoff_calculator::{AutoSimulator, Reporter, Terminal, parse_inputs};

const FIXTURES_DIR: &str = "tests/fixtures";

fn load_fixture(name: &str) -> String {
    fs::read_to_string(format!("{}/{}.txt", FIXTURES_DIR, name)).unwrap()
}

fn dump_counts(label: &str, parsed: &ipl_playoff_calculator::ParsedInput, allow_no_results: bool) {
    let sim = AutoSimulator::new(parsed, allow_no_results);
    let term = Terminal::new(false);
    let result = sim.run(
        &parsed.initial_state,
        &Reporter::new(parsed, &term, allow_no_results),
        &term,
    );

    println!(
        "// {} ({} remaining, {} completed, allow_no_results={})",
        label,
        parsed.matches.len(),
        parsed.completed_matches,
        allow_no_results
    );
    println!("let expected = AllCounts {{");
    println!("    overall: Counts {{");
    println!("        top2_pts: {:?},", result.overall.top2_pts);
    println!(
        "        top2_good_nrr_units: {:?},",
        result.overall.top2_good_nrr_units
    );
    println!("        top4_pts: {:?},", result.overall.top4_pts);
    println!(
        "        top4_good_nrr_units: {:?},",
        result.overall.top4_good_nrr_units
    );
    println!("    }},");
    println!("    if_a_wins: Counts {{");
    println!("        top2_pts: {:?},", result.if_a_wins.top2_pts);
    println!(
        "        top2_good_nrr_units: {:?},",
        result.if_a_wins.top2_good_nrr_units
    );
    println!("        top4_pts: {:?},", result.if_a_wins.top4_pts);
    println!(
        "        top4_good_nrr_units: {:?},",
        result.if_a_wins.top4_good_nrr_units
    );
    println!("    }},");
    println!("    if_b_wins: Counts {{");
    println!("        top2_pts: {:?},", result.if_b_wins.top2_pts);
    println!(
        "        top2_good_nrr_units: {:?},",
        result.if_b_wins.top2_good_nrr_units
    );
    println!("        top4_pts: {:?},", result.if_b_wins.top4_pts);
    println!(
        "        top4_good_nrr_units: {:?},",
        result.if_b_wins.top4_good_nrr_units
    );
    println!("    }},");
    println!("    if_nr: Counts {{");
    println!("        top2_pts: {:?},", result.if_nr.top2_pts);
    println!(
        "        top2_good_nrr_units: {:?},",
        result.if_nr.top2_good_nrr_units
    );
    println!("        top4_pts: {:?},", result.if_nr.top4_pts);
    println!(
        "        top4_good_nrr_units: {:?},",
        result.if_nr.top4_good_nrr_units
    );
    println!("    }},");
    println!("}};");
    println!();
}

fn main() {
    let fixtures = [
        ("valid_9_remaining", "9 remaining", false),
        ("valid_9_remaining", "9 remaining (NR)", true),
        ("valid_15_remaining", "15 remaining", false),
        ("valid_15_remaining", "15 remaining (NR)", true),
        ("valid_25_remaining", "25 remaining", false),
        ("valid_40_remaining", "40 remaining", false),
    ];

    for (fixture, label, allow_no_results) in fixtures {
        let input = load_fixture(fixture);
        let parsed = parse_inputs(&input).unwrap();
        dump_counts(label, &parsed, allow_no_results);
    }
}
