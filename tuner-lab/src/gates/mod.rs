//! Harnesses for the detection thresholds.
//!
//! `config.silence_threshold` is calibrated once from ambient silence and
//! thresholded at three hot-path detectors, two in `engine.rs` and one in
//! `strobe.rs` (ADR 0015 §Context). The OS-CFAR family that could replace it
//! is measured against the same reference.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand};

mod ambient;
mod coarse;

/// Options shared by the coarse-gate studies, all defaulting to the shipped
/// value.
#[derive(Args, Clone)]
pub struct GateOpts {
    /// Capture-set root, or a single capture directory.
    #[arg(default_value = "diagnostics")]
    pub root: PathBuf,
    /// Only these keys, e.g. `0,1,2,4`.
    #[arg(long, value_delimiter = ',')]
    pub keys: Option<Vec<u8>>,
    /// Coarse-read search band, ± cents.
    #[arg(long, default_value_t = 100.0)]
    pub span: f32,
    /// Search-band floor, in bins.
    #[arg(long, default_value_t = 4.0)]
    pub min_bins: f32,
    /// Analysis FFT size.
    #[arg(long, default_value_t = 8192)]
    pub fft: usize,
}

#[derive(Args)]
pub struct Cmd {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand)]
enum Mode {
    /// The one ambient scalar at all three sites that threshold against it:
    /// per hop and per partial, signal and the noise beside it in the *same*
    /// window, scored over a σ sweep (ADR 0015).
    Ambient {
        /// A population to score, repeatable: `--set <label> <regen.json> <dump root>`.
        #[arg(long = "set", num_args = 3, value_names = ["LABEL", "REGEN", "ROOT"], required = true)]
        sets: Vec<String>,
        /// The live cut, in SNR. The default is the pre-registered value;
        /// anything else is exploratory and says so (ADR 0015 §6).
        #[arg(long)]
        live: Option<f32>,
        /// Candidate C's floor quantile. At 0.5 the Rayleigh median conversion
        /// applies; above it the quantile is the threshold directly (§12).
        #[arg(long, default_value_t = 0.5)]
        floor_q: f32,
    },
    /// Per-key × per-partial profile under the settled gate: what it admits,
    /// and how close each cell sits to flipping.
    Profile {
        #[command(flatten)]
        opts: GateOpts,
        /// Highest partial to profile.
        #[arg(long)]
        max_n: Option<usize>,
    },
    /// Realized false-alarm rate of the settled gate, on signal-free input.
    Pfa(GateOpts),
    /// Reference-set anatomy: is the selected order statistic a valley cell or
    /// a weak partial's lobe, and does the guard buy anything (Rohling §V)?
    Refset(GateOpts),
    /// The shipped gate against the harness's replica of it.
    Verify(GateOpts),
    /// The same bounded read under the shipped ambient-σ gate and four OS-CFAR
    /// variants.
    Ab {
        #[command(flatten)]
        opts: GateOpts,
        /// Which partial to test.
        #[arg(long, default_value_t = 1)]
        partial: usize,
    },
}

pub fn run(cmd: Cmd) -> Result<()> {
    match cmd.mode {
        Mode::Ambient {
            sets,
            live,
            floor_q,
        } => ambient::run(&sets, live, floor_q),
        Mode::Profile { opts, max_n } => coarse::profile(&opts, max_n),
        Mode::Pfa(o) => coarse::pfa(&o),
        Mode::Refset(o) => coarse::refset(&o),
        Mode::Verify(o) => coarse::verify(&o),
        Mode::Ab { opts, partial } => coarse::ab(&opts, partial),
    }
}
