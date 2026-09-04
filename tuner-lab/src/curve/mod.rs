//! Harnesses for the tuning-curve engines.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand};

mod auralize;
mod compare;

#[derive(Args)]
pub struct Cmd {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand)]
enum Mode {
    /// All curve engines on one regenerated-partials dump: stretch tables,
    /// implied beat rates, leave-one-key-out error, Giordano cross-scoring,
    /// curvature and flag counts. The curve-side goldens.
    Compare {
        /// A `mat regen` dump.
        #[arg(default_value = "partials_current.json")]
        partials: PathBuf,
        /// Also write the machine-readable report here.
        #[arg(long)]
        json: Option<PathBuf>,
    },
    /// Renders each candidate curve to a loudness-matched WAV by offline
    /// additive resynthesis, so a stretch can be judged by ear.
    Auralize {
        /// A `mat regen` dump.
        #[arg(default_value = "partials_current.json")]
        partials: PathBuf,
        /// Output directory for the WAVs.
        #[arg(long, default_value = "auralize_out")]
        out: PathBuf,
    },
}

pub fn run(cmd: Cmd) -> Result<()> {
    match cmd.mode {
        Mode::Compare { partials, json } => compare::run(&partials, json.as_deref()),
        Mode::Auralize { partials, out } => auralize::run(&partials, &out),
    }
}
