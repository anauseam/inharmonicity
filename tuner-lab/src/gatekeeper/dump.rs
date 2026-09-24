//! Replays a capture through the signal validator and dumps its
//! per-frame metrics to `gatekeeper.csv` beside the audio.

use anyhow::{Result, anyhow};
use std::fs::File;
use std::io::Write;
use std::path::Path;

use tuner_core::audio::{BASS_WINDOW_SIZE, SAMPLE_RATE};
use tuner_core::gatekeeper::SignalState;

use super::replay::{self, Overrides, Source, Thresholds};
use crate::raw;

pub fn run(file_path: &Path) -> Result<()> {
    let parent_dir = file_path.parent().unwrap_or(Path::new(""));
    let thresholds = Thresholds::resolve(parent_dir, &Overrides::default());

    if thresholds.silence == Source::GateDefault {
        println!(
            "Warning: Failed to read 'noise_floor' from analysis.json. Defaulting to 0.005 (-46 dBFS)."
        );
    }

    println!("Loading file: {}", file_path.display());
    println!(
        "Using noise floor: {:.6} (from analysis.json)",
        thresholds.config.silence_threshold
    );
    if thresholds.nhwrsf == Source::Logged {
        println!(
            "Using NHWRSF threshold: {:.3} (from analysis.json)",
            thresholds.config.nhwrsf_threshold
        );
    }
    if thresholds.sustain == Source::Logged {
        println!(
            "Using sustain-stability threshold: {:.1} (from analysis.json)",
            thresholds.config.sustain_stability_threshold
        );
    }

    let audio = raw::read(file_path)
        .ok_or_else(|| anyhow!("{} is not a readable f32 dump", file_path.display()))?;
    let num_samples = audio.len();

    println!(
        "Loaded {} samples ({:.2} seconds at {}Hz)",
        num_samples,
        num_samples as f32 / SAMPLE_RATE as f32,
        SAMPLE_RATE
    );

    if num_samples < BASS_WINDOW_SIZE {
        return Err(anyhow!("File too short for one frame"));
    }

    // Setup output files
    let mut gatekeeper_csv = File::create(parent_dir.join("gatekeeper.csv"))?;
    writeln!(
        gatekeeper_csv,
        "frame_idx,time_ms,rms_ema,nhwrsf,sustain_stability_ema,sustain_stability_raw,state_enum,is_new_onset,state_name"
    )?;

    println!("==================================================");
    println!("Gatekeeper Execution Timeline");
    println!("==================================================");

    for (frame_idx, hop) in replay::run(&audio, thresholds.config).iter().enumerate() {
        let gate_result = hop.result;
        // Stamped at the start of the window the gate analysed.
        let time_ms = (hop.window_start as f32 / SAMPLE_RATE as f32) * 1000.0;

        let state_enum = match gate_result.state {
            SignalState::Silence => 0,
            SignalState::Unstable => 1,
            SignalState::Stable => 2,
        };

        let state_name = match gate_result.state {
            SignalState::Silence => "Silence",
            SignalState::Unstable => "Unstable",
            SignalState::Stable => "Stable",
        };

        if gate_result.is_new_onset {
            println!(
                "\n>>> [{:>6.1} ms] ONSET DETECTED (NHWRSF: {:.3}) <<<",
                time_ms, gate_result.nhwrsf
            );
        }

        // Only print interesting frames to terminal to avoid spam
        if gate_result.state != SignalState::Silence || gate_result.is_new_onset {
            println!(
                "Frame {:4} | {:6.1} ms | {:8} | RMS EMA: {:.5} | Sustain EMA: {:.1}",
                frame_idx,
                time_ms,
                state_name,
                gate_result.rms_ema,
                gate_result.sustain_stability_ema
            );
        }

        writeln!(
            gatekeeper_csv,
            "{},{:.2},{:.5},{:.3},{:.3},{:.3},{},{},{}",
            frame_idx,
            time_ms,
            gate_result.rms_ema,
            gate_result.nhwrsf,
            gate_result.sustain_stability_ema,
            gate_result.sustain_stability_raw,
            state_enum,
            gate_result.is_new_onset,
            state_name
        )?;
    }

    println!("\nDiagnostics complete.");
    println!("Generated gatekeeper.csv in the audio directory.");

    Ok(())
}
