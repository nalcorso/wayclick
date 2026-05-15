// SPDX-License-Identifier: MIT
//! Library entry point for the wayclick macro recorder.
//!
//! Most users invoke this crate through the `wayclick-recorder` binary
//! defined in `src/main.rs`. The library surface is exposed so the
//! integration tests in `tests/e2e/` can drive a full recording session
//! in-process against a test daemon without forking a child binary.

pub mod cli;
pub mod emitter;
pub mod filter;
pub mod keymap;
pub mod recorder;

pub use cli::Cli;
pub use emitter::{CapturedEvent, Emitter, OutputFormat};
pub use filter::{EventClass, FilterSet};
pub use recorder::{run, RecorderError, RecordingSummary, TargetSpec};
