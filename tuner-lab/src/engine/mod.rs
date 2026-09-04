//! Harnesses for discovery: the TWM scorer and the auto lock.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand};

mod dump;
mod lock;
mod mobo;
mod reach;

#[derive(Args)]
pub struct Cmd {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand)]
enum Mode {
    /// End-to-end auto-lock validation, M-of-N rule included. Piano #1
    /// reproduces 81/87 (ADR 0010); a change there is an integration bug.
    Lock {
        /// Capture-set root.
        #[arg(default_value = "diagnostics_piano_1")]
        root: PathBuf,
    },
    /// Stage-A's winner scored on every hop from the onset, gate ignored —
    /// what the gatekeeper's `Stable` wait buys, per register (ADR 0003).
    FromOnset {
        /// Capture-set root.
        #[arg(default_value = "diagnostics_piano_1")]
        root: PathBuf,
    },
    /// Per-frame STFT, peak and TWM dump for one capture, in manual mode.
    Dump(dump::DumpArgs),
    /// 1 ¢-resolution detuning reach of the discovery lock (ADR 0006).
    Reach {
        /// Extra candidate configurations, as repeated `name q r rho` quads.
        candidates: Vec<String>,
    },
    /// The synthetic dataset generator and discovery fitness harness behind
    /// the MOBO parameter sweep (ADR 0001).
    Mobo {
        /// Serve trials on stdin/stdout for `scripts/optimize_twm.py`.
        #[arg(long)]
        serve: bool,
    },
}

pub fn run(cmd: Cmd) -> Result<()> {
    match cmd.mode {
        Mode::Lock { root } => lock::run(&root, false),
        Mode::FromOnset { root } => lock::run(&root, true),
        Mode::Dump(args) => dump::run(args),
        Mode::Reach { candidates } => {
            reach::run(&candidates);
            Ok(())
        }
        Mode::Mobo { serve } => {
            mobo::run(serve);
            Ok(())
        }
    }
}
