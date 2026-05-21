pub mod cli;
pub mod models;
pub mod parser;
pub mod ranking;
pub mod reporter;
pub mod simulate;
pub mod terminal;
pub mod utils;

pub use cli::{CliArgs, parse_args};
pub use models::{Algorithm, AllCounts, AppError, Counts, ParsedInput, StandingState};
pub use parser::{parse_inputs, read_matches_file};
pub use ranking::Ranker;
pub use reporter::Reporter;
pub use simulate::{SimulationRunner, auto::AutoSimulator, dfs::DfsSimulator, dp::DpSimulator};
pub use terminal::{Colors, Terminal};
