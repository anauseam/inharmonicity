//! Harnesses for the 5-state signal validator.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand};

mod dump;
mod sparsity;

#[derive(Args)]
pub struct Cmd {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand)]
enum Mode {
    /// Per-frame validator metrics for one capture, to `gatekeeper.csv`.
    Dump {
        /// Path to an `audio.raw` or `audio_full_event.raw`.
        audio: PathBuf,
    },
    /// Our spectral-sparsity gate against faithful Mounir NINOS² variants
    /// (faithfulness audit 05).
    Sparsity {
        /// Capture-set root.
        #[arg(default_value = "diagnostics")]
        root: PathBuf,
    },
}

pub fn run(cmd: Cmd) -> Result<()> {
    match cmd.mode {
        Mode::Dump { audio } => dump::run(&audio),
        Mode::Sparsity { root } => sparsity::run(&root),
    }
}
