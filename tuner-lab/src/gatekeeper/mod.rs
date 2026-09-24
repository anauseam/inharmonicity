//! Harnesses for the signal validator.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use tuner_core::models::InharmonicityProfile;

mod dump;
mod plot;
mod replay;
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
    /// The gate's verdict on each capture, drawn: the waveform shaded by state,
    /// then NHWRSF, RMS and sustain stability against their thresholds. Writes
    /// `gatekeeper.png` beside each capture and prints the wait from onset to
    /// `Stable` per register.
    Plot {
        /// One capture directory, or a capture-set root.
        path: PathBuf,
        /// Write the images here, named for their captures, instead of beside them.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Fill in the NHWRSF and sustain thresholds a capture did not log from
        /// this instrument profile.
        #[arg(long)]
        profile: Option<PathBuf>,
        /// Replay at this silence threshold, whatever the capture logged.
        #[arg(long)]
        silence: Option<f32>,
        /// Replay at this NHWRSF threshold, whatever the capture logged.
        #[arg(long)]
        nhwrsf: Option<f32>,
        /// Replay at this sustain-stability threshold, whatever the capture logged.
        #[arg(long)]
        sustain: Option<f32>,
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
        Mode::Plot {
            path,
            out,
            profile,
            silence,
            nhwrsf,
            sustain,
        } => {
            let profile = profile
                .map(|p| {
                    InharmonicityProfile::from_file(&p)
                        .map(|profile| profile.settings)
                        .with_context(|| format!("read profile {}", p.display()))
                })
                .transpose()?;
            let overrides = replay::Overrides {
                silence,
                nhwrsf,
                sustain,
                profile,
            };
            plot::run(&path, out.as_deref(), &overrides)
        }
        Mode::Sparsity { root } => sparsity::run(&root),
    }
}
