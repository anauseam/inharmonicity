//! Harnesses for the detection thresholds.
//!
//! `config.silence_threshold` is calibrated once from ambient silence and
//! thresholded at three hot-path detectors, two in `Engine::process` and one in
//! `Strobe::process` (report 0015). The OS-CFAR family that could replace it is
//! measured against the same reference.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand};

pub(crate) mod ambient;
pub(crate) mod arms;
mod coarse;
mod distribution;
mod stats;
mod truth;

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
    /// per hop and per partial, signal and the noise beside it in the same
    /// window, scored over a σ sweep (report 0015).
    Ambient {
        /// A population to score, repeatable: `--set <label> <regen.json> <dump root>`.
        #[arg(long = "set", num_args = 3, value_names = ["LABEL", "REGEN", "ROOT"], required = true)]
        sets: Vec<String>,
        /// The live cut, in SNR. The default is the pre-registered value;
        /// anything else is exploratory and says so (report 0015 §6).
        #[arg(long)]
        live: Option<f32>,
        /// Candidate C's floor quantile. At 0.5 the Rayleigh median conversion
        /// applies; above it the quantile is the threshold directly (report 0015 §12).
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
    /// Synthetic truth: the AWGN control (`t1`) and known partials in real
    /// noise (`t2`), written as dump directories with the exact noise beside
    /// each capture.
    Truth(truth::Cmd),
    /// One candidate noise estimate or detection rule over one or more dump
    /// sets: per (capture, hop, partial), the estimate, the truth where known,
    /// the threshold, the decision and the two SNRs, in `ambient`'s columns.
    Arms {
        /// The arm: A, A@1e-3, B, C, N, or a candidate by its number or card.
        #[arg(long)]
        arm: String,
        /// A population to score, repeatable: `--set <label> <regen.json> <dump root>`.
        #[arg(long = "set", num_args = 3, value_names = ["LABEL", "REGEN", "ROOT"], required = true)]
        sets: Vec<String>,
        /// Write one CSV of rows per (arm, set) here.
        #[arg(long)]
        out: Option<PathBuf>,
        /// The operating point: the false-alarm probability every arm is set to.
        #[arg(long, default_value_t = 1.0e-3)]
        pfa: f32,
        /// Fix σ for every capture, in place of each one's recorded value.
        #[arg(long)]
        sigma: Option<f32>,
        /// ν/2 per frame averaged, as Stage B measured it; the analytic Hann
        /// value otherwise.
        #[arg(long)]
        nu_half: Option<f32>,
        /// Only these consumers, e.g. `G1,G3`.
        #[arg(long, value_delimiter = ',')]
        consumers: Option<Vec<String>>,
        /// How the record is found inside the full event: `aligned` (its opening
        /// samples matched; nearly every capture) or `strict` (the suffix only;
        /// report 0015's population).
        #[arg(long, default_value = "aligned")]
        preroll: String,
        /// Read a synthetic set's exact level as T2's is read, the per-hop running
        /// median of the known noise, even where the generator gives a closed form:
        /// on T1 this calibrates the T2 truth construction itself.
        #[arg(long)]
        exact_median: bool,
    },
    /// Stage B: the room's per-bin distribution on the screened pre-roll of
    /// each set and on any room recording, per bin and pooled per band.
    Distribution {
        /// A set's screened pre-roll, repeatable: `--set <label> <dump root> <sigma>`,
        /// σ the screen's silence threshold, or `-` for each capture's logged one
        /// (an external store's `calibration_candidates.json`).
        #[arg(long = "set", num_args = 3, value_names = ["LABEL", "ROOT", "SIGMA"])]
        sets: Vec<String>,
        /// A `cargo lab audio record` directory.
        #[arg(long)]
        recording: Option<PathBuf>,
        /// Write one CSV of per-bin statistics per set here.
        #[arg(long)]
        out: Option<PathBuf>,
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
        Mode::Truth(c) => truth::run(c),
        Mode::Arms {
            arm,
            sets,
            out,
            pfa,
            sigma,
            nu_half,
            consumers,
            preroll,
            exact_median,
        } => {
            let preroll = match preroll.as_str() {
                "aligned" => arms::Preroll::Aligned,
                "strict" => arms::Preroll::Strict,
                other => anyhow::bail!("--preroll takes aligned or strict, not {other}"),
            };
            arms::run(
                &arm,
                &sets,
                out.as_deref(),
                pfa,
                sigma,
                nu_half,
                consumers.as_deref(),
                preroll,
                exact_median,
            )
        }
        Mode::Distribution {
            sets,
            recording,
            out,
        } => distribution::run(&sets, recording.as_deref(), out.as_deref()),
    }
}
