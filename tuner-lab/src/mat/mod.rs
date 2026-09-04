//! Harnesses for the Worker's (f₀, B) estimator.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand};

mod recovery;
mod regenerate;
mod repeats;
mod validate;

#[derive(Args)]
pub struct Cmd {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand)]
enum Mode {
    /// Measured inharmonicity against the Rigaud prior for both trajectory
    /// orders, plus the ground-truth-free goodness-of-fit comparison.
    Validate {
        /// Capture-set root.
        #[arg(default_value = "diagnostics")]
        root: PathBuf,
    },
    /// Each capture re-measured on same-length windows cut at offsets from the
    /// physical onset — what the gatekeeper's `Stable` start costs the B fit
    /// (ADR 0009 analysis 9). Needs `audio_full_event.raw`.
    Offset {
        /// Capture-set root.
        #[arg(default_value = "diagnostics")]
        root: PathBuf,
        /// Offsets from the physical onset, in ms, e.g. `0,116,300`.
        #[arg(long, value_name = "MS", value_delimiter = ',', required = true)]
        offsets: Vec<u32>,
    },
    /// The repeat-capture noise decomposition — σ_lnB, ρ reproducibility and
    /// strike strength (ADR 0009).
    Repeats {
        /// A `mat regen` dump.
        partials: PathBuf,
    },
    /// Re-derives per-key partials from the kept audio with the current
    /// estimator, one JSON dump on stdout. **The required entry point for
    /// piano #2 data** (`06`).
    Regen {
        /// Capture-set root.
        #[arg(default_value = "diagnostics")]
        root: PathBuf,
    },
    /// MAT against *known* synthetic B, swept 1×–25× the prior with and
    /// without a fundamental — the characterisation behind the deep-bass
    /// measurement argument (ADR 0006). Its assertion is
    /// `tuner-core/tests/mat_b_recovery.rs`.
    Recovery,
}

pub fn run(cmd: Cmd) -> Result<()> {
    match cmd.mode {
        Mode::Validate { root } => validate::run(&root, None),
        Mode::Offset { root, offsets } => validate::run(&root, Some(&offsets)),
        Mode::Repeats { partials } => {
            repeats::run(&partials);
            Ok(())
        }
        Mode::Regen { root } => regenerate::run(&root),
        Mode::Recovery => {
            recovery::run();
            Ok(())
        }
    }
}
