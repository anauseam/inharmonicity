//! Harnesses for discovery: the TWM scorer and the auto lock.

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::regen;
use tuner_core::algorithms::curves::{self, BASS_BRIDGE_MIN_PARTIALS};
use tuner_core::models::{self, CurveInput, InharmonicityProfile};

mod dump;
mod lock;
mod nsga2;
mod reach;

#[derive(Args)]
pub struct Cmd {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand)]
enum Mode {
    /// End-to-end auto-lock validation, M-of-N rule included. Piano #1
    /// gives 82/87 (report 0020); a change there is an integration bug.
    Lock {
        /// Capture-set root.
        #[arg(default_value = "diagnostics_piano_1")]
        root: PathBuf,
        /// A `mat regen` dump of the instrument: the templates are the ones the
        /// app would push for its profile, not the prior (report 0021).
        #[arg(long, value_name = "REGEN")]
        regen: Option<PathBuf>,
    },
    /// Stage-A's winner scored on every hop from the onset, gate ignored —
    /// per-hop identification by register, `Stable` hops marked (report 0003).
    FromOnset {
        /// Capture-set root.
        #[arg(default_value = "diagnostics_piano_1")]
        root: PathBuf,
    },
    /// Per-frame STFT, peak and TWM dump for one capture, in manual mode.
    Dump(dump::DumpArgs),
    /// 1 ¢-resolution detuning reach of the discovery lock (report 0006).
    Reach {
        /// Extra candidate configurations, as repeated `name q r rho` quads.
        candidates: Vec<String>,
    },
    /// The synthetic dataset generator and discovery fitness harness behind
    /// the NSGA-II parameter sweep (report 0001).
    Nsga2 {
        /// Serve trials on stdin/stdout for `optimize_twm.py`.
        #[arg(long)]
        serve: bool,
    },
}

pub fn run(cmd: Cmd) -> Result<()> {
    match cmd.mode {
        Mode::Lock { root, regen } => lock::run(&root, false, regen.as_deref()),
        Mode::FromOnset { root } => lock::run(&root, true, None),
        Mode::Dump(args) => dump::run(args),
        Mode::Reach { candidates } => {
            reach::run(&candidates);
            Ok(())
        }
        Mode::Nsga2 { serve } => {
            nsga2::run(serve);
            Ok(())
        }
    }
}

/// Discovery's 88 template B: the Rigaud prior, or with a `mat regen` dump the
/// instrument's fitted curve, through the source the app ships.
fn template_b(regen: Option<&Path>) -> [f32; 88] {
    match regen {
        Some(path) => {
            let (profile, dropped) = regen::profile(path);
            if dropped > 0 {
                eprintln!("[TEMPLATES] {dropped} implausible regen entries dropped");
            }
            fitted_template_b(&profile, path)
        }
        None => core::array::from_fn(|k| models::get_expected_beta(k as u8)),
    }
}

/// Discovery's 88 template B for `profile`, through the source the app ships,
/// announced on stderr: the keys the fit admits and its pair.
fn fitted_template_b(profile: &InharmonicityProfile, source: &Path) -> [f32; 88] {
    let input = CurveInput::from_profile(profile);
    let measured = input
        .keys
        .iter()
        .flatten()
        .filter(|d| d.partials.len() >= BASS_BRIDGE_MIN_PARTIALS)
        .count();
    let bxi = curves::bass_bridge_fit(&input);
    eprintln!(
        "[TEMPLATES] {measured} of 88 keys measured from {}: s_B {:?}, y_B {:?}",
        source.display(),
        bxi.s_b,
        bxi.y_b
    );
    core::array::from_fn(|k| bxi.b_at_key(k) as f32)
}
