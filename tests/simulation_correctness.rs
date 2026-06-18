use std::fs;

use ipl_playoff_calculator::{
    AllCounts, AutoSimulator, Counts, DfsSimulator, DpSimulator, Terminal, parse_inputs,
};

const FIXTURES_DIR: &str = "tests/fixtures";

fn load_fixture(name: &str) -> String {
    fs::read_to_string(format!("{}/{}.txt", FIXTURES_DIR, name)).unwrap()
}

fn run_dfs(parsed: &ipl_playoff_calculator::ParsedInput) -> AllCounts {
    let sim = DfsSimulator::new(parsed, false);
    let term = Terminal::new(false);
    sim.run(
        &parsed.initial_state,
        &ipl_playoff_calculator::Reporter::new(parsed, &term, false),
        &term,
    )
}

fn run_dp(parsed: &ipl_playoff_calculator::ParsedInput) -> AllCounts {
    let sim = DpSimulator::new(parsed, false);
    let term = Terminal::new(false);
    sim.run(
        &parsed.initial_state,
        &ipl_playoff_calculator::Reporter::new(parsed, &term, false),
        &term,
    )
}

fn run_auto(parsed: &ipl_playoff_calculator::ParsedInput) -> AllCounts {
    let sim = AutoSimulator::new(parsed, false);
    let term = Terminal::new(false);
    sim.run(
        &parsed.initial_state,
        &ipl_playoff_calculator::Reporter::new(parsed, &term, false),
        &term,
    )
}

fn run_dfs_nr(parsed: &ipl_playoff_calculator::ParsedInput) -> AllCounts {
    let sim = DfsSimulator::new(parsed, true);
    let term = Terminal::new(false);
    sim.run(
        &parsed.initial_state,
        &ipl_playoff_calculator::Reporter::new(parsed, &term, true),
        &term,
    )
}

fn run_dp_nr(parsed: &ipl_playoff_calculator::ParsedInput) -> AllCounts {
    let sim = DpSimulator::new(parsed, true);
    let term = Terminal::new(false);
    sim.run(
        &parsed.initial_state,
        &ipl_playoff_calculator::Reporter::new(parsed, &term, true),
        &term,
    )
}

fn run_auto_nr(parsed: &ipl_playoff_calculator::ParsedInput) -> AllCounts {
    let sim = AutoSimulator::new(parsed, true);
    let term = Terminal::new(false);
    sim.run(
        &parsed.initial_state,
        &ipl_playoff_calculator::Reporter::new(parsed, &term, true),
        &term,
    )
}

// ===================================================================
// Known correct values generated using src/bin/dump_golden.rs
// ===================================================================

fn golden_9() -> AllCounts {
    AllCounts {
        overall: Counts {
            top2_pts: [56, 432, 0, 0, 0, 24, 168, 0, 0, 0],
            top2_good_nrr_units: [361200, 1219680, 0, 0, 102480, 173040, 724080, 0, 0, 0],
            top4_pts: [354, 512, 36, 0, 100, 242, 480, 100, 0, 4],
            top4_good_nrr_units: [
                1007580, 1290240, 128520, 0, 391860, 726600, 1268400, 289800, 0, 57960,
            ],
        },
        if_a_wins: Counts {
            top2_pts: [32, 224, 0, 0, 0, 0, 96, 0, 0, 0],
            top2_good_nrr_units: [196560, 618240, 0, 0, 62160, 18480, 394800, 0, 0, 0],
            top4_pts: [186, 256, 25, 0, 60, 60, 248, 69, 0, 4],
            top4_good_nrr_units: [
                517020, 645120, 89460, 0, 216300, 214200, 640080, 200340, 0, 57960,
            ],
        },
        if_b_wins: Counts {
            top2_pts: [24, 208, 0, 0, 0, 24, 72, 0, 0, 0],
            top2_good_nrr_units: [164640, 601440, 0, 0, 40320, 154560, 329280, 0, 0, 0],
            top4_pts: [168, 256, 11, 0, 40, 182, 232, 31, 0, 0],
            top4_good_nrr_units: [
                490560, 645120, 39060, 0, 175560, 512400, 628320, 89460, 0, 0,
            ],
        },
        if_nr: Counts::default(),
    }
}

fn golden_9_nr() -> AllCounts {
    AllCounts {
        overall: Counts {
            top2_pts: [2646, 18846, 0, 0, 0, 918, 10422, 0, 0, 0],
            top2_good_nrr_units: [
                11130210, 48886740, 0, 0, 2001510, 4144770, 33039090, 0, 0, 0,
            ],
            top4_pts: [15351, 19683, 555, 0, 3993, 10005, 19494, 2034, 0, 288],
            top4_good_nrr_units: [
                42322266, 49601160, 2957850, 0, 14370930, 29358000, 49459410, 9000810, 0, 1334214,
            ],
        },
        if_a_wins: Counts {
            top2_pts: [972, 6318, 0, 0, 0, 0, 3834, 0, 0, 0],
            top2_good_nrr_units: [3963330, 16329600, 0, 0, 788130, 153090, 11833290, 0, 0, 0],
            top4_pts: [5367, 6561, 306, 0, 1653, 1629, 6534, 1071, 0, 288],
            top4_good_nrr_units: [
                14664762, 16533720, 1427832, 0, 5798142, 5732622, 16516710, 4144392, 0, 1316700,
            ],
        },
        if_b_wins: Counts {
            top2_pts: [756, 6210, 0, 0, 0, 756, 2916, 0, 0, 0],
            top2_good_nrr_units: [3333960, 16227540, 0, 0, 487620, 3265920, 9752400, 0, 0, 0],
            top4_pts: [4749, 6561, 57, 0, 828, 5145, 6426, 225, 0, 0],
            top4_good_nrr_units: [
                13417992, 16533720, 481572, 0, 3504312, 14190372, 16431660, 1575252, 0, 0,
            ],
        },
        if_nr: Counts {
            top2_pts: [918, 6318, 0, 0, 0, 162, 3672, 0, 0, 0],
            top2_good_nrr_units: [3832920, 16329600, 0, 0, 725760, 725760, 11453400, 0, 0, 0],
            top4_pts: [5235, 6561, 192, 0, 1512, 3231, 6534, 738, 0, 0],
            top4_good_nrr_units: [
                14239512, 16533720, 1048446, 0, 5068476, 9435006, 16511040, 3281166, 0, 17514,
            ],
        },
    }
}

fn golden_15() -> AllCounts {
    AllCounts {
        overall: Counts {
            top2_pts: [12612, 12408, 766, 0, 2732, 1477, 12284, 8823, 0, 0],
            top2_good_nrr_units: [
                39804702, 39932382, 2506140, 0, 11546052, 8792532, 39759132, 22809780, 0, 0,
            ],
            top4_pts: [22680, 22732, 3202, 0, 9668, 9206, 22764, 19865, 0, 28],
            top4_good_nrr_units: [
                66027192, 66124548, 9610020, 0, 35257908, 34506528, 66194688, 51600780, 0, 979776,
            ],
        },
        if_a_wins: Counts {
            top2_pts: [2684, 6208, 385, 0, 1284, 708, 10080, 4351, 0, 0],
            top2_good_nrr_units: [
                10280130, 19833450, 1228500, 0, 5663070, 4292820, 30054570, 11222820, 0, 0,
            ],
            top4_pts: [8464, 11264, 1507, 0, 4848, 4602, 14348, 9975, 0, 14],
            top4_good_nrr_units: [
                27127632, 32919852, 4544820, 0, 17679732, 17311812, 39196332, 25884180, 0, 486360,
            ],
        },
        if_b_wins: Counts {
            top2_pts: [9928, 6200, 381, 0, 1448, 769, 2204, 4472, 0, 0],
            top2_good_nrr_units: [
                29524572, 20098932, 1277640, 0, 5882982, 4499712, 9704562, 11586960, 0, 0,
            ],
            top4_pts: [14216, 11468, 1695, 0, 4820, 4604, 8416, 9890, 0, 14],
            top4_good_nrr_units: [
                38899560, 33204696, 5065200, 0, 17578176, 17194716, 26998356, 25716600, 0, 493416,
            ],
        },
        if_nr: Counts::default(),
    }
}

fn golden_15_nr() -> AllCounts {
    AllCounts {
        overall: Counts {
            top2_pts: [
                6432765, 6579547, 65886, 0, 1161167, 787086, 6497083, 2532889, 0, 0,
            ],
            top2_good_nrr_units: [
                18769783182,
                19115125158,
                325243908,
                0,
                4234399452,
                3081509532,
                18976560060,
                7815869988,
                0,
                0,
            ],
            top4_pts: [
                11522043, 11620600, 587470, 0, 4649622, 4525284, 11579736, 7901180, 0, 26208,
            ],
            top4_good_nrr_units: [
                30722272020,
                30870891834,
                2117626818,
                0,
                14202316050,
                13837381788,
                30849524922,
                21902588154,
                0,
                134380974,
            ],
        },
        if_a_wins: Counts {
            top2_pts: [
                883944, 2140481, 18390, 0, 327784, 222183, 3677414, 805600, 0, 0,
            ],
            top2_good_nrr_units: [
                3016070736,
                6255515406,
                93685704,
                0,
                1275081360,
                931565376,
                10082261616,
                2451983562,
                0,
                0,
            ],
            top4_pts: [
                3026639, 3851976, 185612, 0, 1573349, 1525326, 4698492, 2682666, 0, 8736,
            ],
            top4_good_nrr_units: [
                8469638940,
                10242379014,
                661755486,
                0,
                4805477418,
                4667434842,
                11950283394,
                7367614548,
                0,
                47743878,
            ],
        },
        if_b_wins: Counts {
            top2_pts: [
                3551517, 2177243, 24609, 0, 355703, 248814, 806859, 874290, 0, 0,
            ],
            top2_good_nrr_units: [
                9782487582, 6367063584, 109902744, 0, 1365684516, 1020102930, 2835995316,
                2624927088, 0, 0,
            ],
            top4_pts: [
                4660294, 3907809, 220557, 0, 1553788, 1507527, 3020832, 2656761, 0, 8736,
            ],
            top4_good_nrr_units: [
                11891956230,
                10375347444,
                755881728,
                0,
                4755411738,
                4622267412,
                8461173420,
                7302499890,
                0,
                47789658,
            ],
        },
        if_nr: Counts {
            top2_pts: [
                1997304, 2261823, 22887, 0, 477680, 316089, 2012810, 852999, 0, 0,
            ],
            top2_good_nrr_units: [
                5971224864, 6492546168, 121655460, 0, 1593633576, 1129841226, 6058303128,
                2738959338, 0, 0,
            ],
            top4_pts: [
                3835110, 3860815, 181301, 0, 1522485, 1492431, 3860412, 2561753, 0, 8736,
            ],
            top4_good_nrr_units: [
                10360676850,
                10253165376,
                699989604,
                0,
                4641426894,
                4547679534,
                10438068108,
                7232473716,
                0,
                38847438,
            ],
        },
    }
}

fn golden_25() -> AllCounts {
    AllCounts {
        overall: Counts {
            top2_pts: [
                6637134, 11868950, 335784, 0, 482250, 5639630, 3790494, 25447824, 4904, 491220,
            ],
            top2_good_nrr_units: [
                23722076964,
                38215290912,
                1005565680,
                1125414,
                2825756160,
                21563640570,
                14447036658,
                64287906480,
                169683402,
                2876255040,
            ],
            top4_pts: [
                17925191, 23354124, 3012898, 2360, 3666126, 18021278, 12115774, 31092324, 292176,
                3532412,
            ],
            top4_good_nrr_units: [
                55363401978,
                66948258912,
                7909196400,
                322781772,
                15565137852,
                55865342664,
                40196065884,
                78669349920,
                1958118690,
                15431020488,
            ],
        },
        if_a_wins: Counts {
            top2_pts: [
                3177716, 5868624, 169640, 0, 219496, 2622198, 521280, 14826530, 2772, 229352,
            ],
            top2_good_nrr_units: [
                11594341425,
                19103135613,
                479997000,
                925923,
                1358590383,
                10358601147,
                2777115867,
                37415359800,
                85469487,
                1383631995,
            ],
            top4_pts: [
                9358572, 12036918, 1715462, 1706, 1983196, 9403628, 3545872, 16495272, 155458,
                1869404,
            ],
            top4_good_nrr_units: [
                28617217287,
                34191962427,
                4383202320,
                197899395,
                8356882563,
                28834415151,
                13563650991,
                41628323520,
                1098685887,
                8242097739,
            ],
        },
        if_b_wins: Counts {
            top2_pts: [
                3459418, 6000326, 166144, 0, 262754, 3017432, 3269214, 10621294, 2132, 261868,
            ],
            top2_good_nrr_units: [
                12127735539,
                19112155299,
                525568680,
                199491,
                1467165777,
                11205039423,
                11669920791,
                26872546680,
                84213915,
                1492623045,
            ],
            top4_pts: [
                8566619, 11317206, 1297436, 654, 1682930, 8617650, 8569902, 14597052, 136718,
                1663008,
            ],
            top4_good_nrr_units: [
                26746184691,
                32756296485,
                3525994080,
                124882377,
                7208255289,
                27030927513,
                26632414893,
                37041026400,
                859432803,
                7188922749,
            ],
        },
        if_nr: Counts::default(),
    }
}

fn golden_40() -> AllCounts {
    AllCounts {
        overall: Counts {
            top2_pts: [
                99725321044,
                279439879341,
                4297359414,
                22319449546,
                20195177036,
                281871700507,
                100443166004,
                801787203025,
                22972022352,
                193270174555,
            ],
            top2_good_nrr_units: [
                370159408474104,
                903163681127568,
                12404537190360,
                103867418870010,
                96402502131018,
                908626134225006,
                372925297590876,
                2022078943090080,
                106484271779202,
                645426409512816,
            ],
            top4_pts: [
                315038982648,
                602301837859,
                46423678716,
                114478943285,
                118888792239,
                602236644185,
                314766219651,
                1000165175892,
                114391467723,
                456144958674,
            ],
            top4_good_nrr_units: [
                1049083543660968,
                1779446533233660,
                120976457299740,
                452846696729178,
                463152015864186,
                1777803719992578,
                1049001255613650,
                2524405030183260,
                452794378215780,
                1413567577189080,
            ],
        },
        if_a_wins: Counts {
            top2_pts: [
                80490396452,
                142083127323,
                2165637619,
                11735157778,
                10474146099,
                142575947920,
                51991954741,
                403659744644,
                11789578778,
                51790373373,
            ],
            top2_good_nrr_units: [
                285692042050632,
                458672340962160,
                6247478001240,
                54316550484312,
                49817018928726,
                459504099156636,
                192258954719214,
                1018012627704240,
                54687012680526,
                191561177307834,
            ],
            top4_pts: [
                224053081194,
                301012495817,
                23249266621,
                57437635827,
                59619458394,
                301161733825,
                157496760969,
                499777879755,
                57068969201,
                154731683387,
            ],
            top4_good_nrr_units: [
                709530765234072,
                889578102792870,
                60567692861340,
                227228181309792,
                232149471818970,
                889084825095264,
                525064173335742,
                1261419797959020,
                226270833895944,
                520644759688026,
            ],
        },
        if_b_wins: Counts {
            top2_pts: [
                19234924592,
                137356752018,
                2131721795,
                10584291768,
                9721030937,
                139295752587,
                48451211263,
                398127458381,
                11182443574,
                141479801182,
            ],
            top2_good_nrr_units: [
                84467366423472,
                444491340165408,
                6157059189120,
                49550868385698,
                46585483202292,
                449122035068370,
                180666342871662,
                1004066315385840,
                51797259098676,
                453865232204982,
            ],
            top4_pts: [
                90985901454,
                301289342042,
                23174412095,
                57041307458,
                59269333845,
                301074910360,
                157269458682,
                500387296137,
                57322498522,
                301413275287,
            ],
            top4_good_nrr_units: [
                339552778426896,
                889868430440790,
                60408764438400,
                225618515419386,
                231002544045216,
                888718894897314,
                523937082277908,
                1262985232224240,
                226523544319836,
                892922817501054,
            ],
        },
        if_nr: Counts::default(),
    }
}

// ===================================================================
// Tests: each algorithm checked against golden values directly
// ===================================================================

#[test]
fn test_dfs_9_matches_golden() {
    let input = load_fixture("valid_9_remaining");
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(run_dfs(&parsed), golden_9(), "DFS 9 remaining mismatch");
}

#[test]
fn test_dp_9_matches_golden() {
    let input = load_fixture("valid_9_remaining");
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(run_dp(&parsed), golden_9(), "DP 9 remaining mismatch");
}

#[test]
fn test_auto_9_matches_golden() {
    let input = load_fixture("valid_9_remaining");
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(run_auto(&parsed), golden_9(), "Auto 9 remaining mismatch");
}

#[test]
fn test_dfs_9_matches_nr_golden() {
    let input = load_fixture("valid_9_remaining");
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(
        run_dfs_nr(&parsed),
        golden_9_nr(),
        "DFS 9 remaining (NR) mismatch"
    );
}

#[test]
fn test_dp_9_matches_nr_golden() {
    let input = load_fixture("valid_9_remaining");
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(
        run_dp_nr(&parsed),
        golden_9_nr(),
        "DP 9 remaining (NR) mismatch"
    );
}

#[test]
fn test_auto_9_matches_nr_golden() {
    let input = load_fixture("valid_9_remaining");
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(
        run_auto_nr(&parsed),
        golden_9_nr(),
        "Auto 9 remaining (NR) mismatch"
    );
}

#[test]
fn test_dfs_15_matches_golden() {
    let input = load_fixture("valid_15_remaining");
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(run_dfs(&parsed), golden_15(), "DFS 15 remaining mismatch");
}

#[test]
fn test_dp_15_matches_golden() {
    let input = load_fixture("valid_15_remaining");
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(run_dp(&parsed), golden_15(), "DP 15 remaining mismatch");
}

#[test]
fn test_auto_15_matches_golden() {
    let input = load_fixture("valid_15_remaining");
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(run_auto(&parsed), golden_15(), "Auto 15 remaining mismatch");
}

#[test]
fn test_dfs_15_matches_nr_golden() {
    let input = load_fixture("valid_15_remaining");
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(
        run_dfs_nr(&parsed),
        golden_15_nr(),
        "DFS 15 remaining (NR) mismatch"
    );
}

#[test]
fn test_dp_15_matches_nr_golden() {
    let input = load_fixture("valid_15_remaining");
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(
        run_dp_nr(&parsed),
        golden_15_nr(),
        "DP 15 remaining (NR) mismatch"
    );
}

#[test]
fn test_auto_15_matches_nr_golden() {
    let input = load_fixture("valid_15_remaining");
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(
        run_auto_nr(&parsed),
        golden_15_nr(),
        "Auto 15 remaining (NR) mismatch"
    );
}

#[test]
fn test_auto_25_matches_golden() {
    let input = load_fixture("valid_25_remaining");
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(run_auto(&parsed), golden_25(), "Auto 25 remaining mismatch");
}

#[test]
fn test_auto_forced_hybrid_25_matches_golden() {
    let (dp_ram_mb, _) = ipl_playoff_calculator::simulate::cost::estimate_dp_cost(24, 2);
    ipl_playoff_calculator::utils::set_system_ram_override(Some(dp_ram_mb));

    let input = load_fixture("valid_25_remaining");
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(
        run_auto(&parsed),
        golden_25(),
        "Auto (Forced Hybrid) 25 remaining mismatch"
    );

    ipl_playoff_calculator::utils::set_system_ram_override(None);
}

#[test]
fn test_auto_40_matches_golden() {
    let input = load_fixture("valid_40_remaining");
    let parsed = parse_inputs(&input).unwrap();
    assert_eq!(run_auto(&parsed), golden_40(), "Auto 40 remaining mismatch");
}
