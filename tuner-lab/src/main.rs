//! # `tuner-lab` — the measurement instruments
//!
//! The harnesses that produced this project's evidence. Run them through the
//! alias, which bakes in `--release`:
//!
//! ```bash
//! cargo lab engine lock diagnostics_piano_1
//! cargo lab strobe replay diagnostics
//! cargo lab --help
//! ```
//!
//! What belongs here rather than in `tuner-core/tests/` or `benches/`, and the
//! crate boundary the lab observes: `tuner-lab/README.md` and ADR 0016.

mod capture;
mod curve;
mod engine;
mod gatekeeper;
mod gates;
mod mat;
mod raw;
mod regen;
mod strobe;
mod truth;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "lab",
    about = "Measurement harnesses for the inharmonicity engine",
    long_about = "Offline instruments that replay captured audio through the shipped \
                  algorithms and report what they saw. Each subcommand names the \
                  subsystem it measures; `--help` at any level lists its modes.",
    version
)]
struct Cli {
    #[command(subcommand)]
    subsystem: Subsystem,
}

#[derive(Subcommand)]
enum Subsystem {
    /// Discovery: the TWM scorer, the auto lock, and its parameter sweeps.
    Engine(engine::Cmd),
    /// The 5-state signal validator.
    Gatekeeper(gatekeeper::Cmd),
    /// The Worker's (f₀, B) estimator and the dumps it consumes.
    Mat(mat::Cmd),
    /// The tuning-curve engines and their auralization.
    Curve(curve::Cmd),
    /// The strobe bank: rotation, unison lines, and the displayed readout.
    Strobe(strobe::Cmd),
    /// The detection thresholds, which span the engine and the strobe.
    Gates(gates::Cmd),
}

fn main() -> Result<()> {
    match Cli::parse().subsystem {
        Subsystem::Engine(c) => engine::run(c),
        Subsystem::Gatekeeper(c) => gatekeeper::run(c),
        Subsystem::Mat(c) => mat::run(c),
        Subsystem::Curve(c) => curve::run(c),
        Subsystem::Strobe(c) => strobe::run(c),
        Subsystem::Gates(c) => gates::run(c),
    }
}
