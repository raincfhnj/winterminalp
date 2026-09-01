mod error;
mod manager;
mod profile;
mod types;

pub use error::TerminalError;
pub use manager::{TerminalManager, TerminalSink};
pub use types::{TerminalEvent, TerminalEventKind, TerminalProfile, TerminalStarted};
