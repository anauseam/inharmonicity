//! End-to-end validation of the SHIPPED engine auto-lock path.
//!
//! `diagnose_engine` drives the engine in *manual* mode (`Some(winning_key)`),
//! so `Engine::process`'s auto M-of-N acquisition lock (ADR 0010) is never
//! exercised by the Python replicas — those validate the *rule*, not the
//! *integration*. This harness closes that gap: it drives the real
//! `Engine::process` in **auto mode** (`target_note = None`) frame-by-frame over
//! each capture, feeding the gatekeeper's `is_silence/is_stable/is_new_onset/
//! is_transient_bypass` exactly as the live pipeline does, and records the first
//! key the engine latches (`identified_key` None→Some). The engine's auto path
//! always refines (`discover(refine=true)`), so with the shipped
//! `LOCK_VOTES_M = 7 / LOCK_WINDOW_N = 8` this MUST reproduce the refined
//! (7,8) number the Python replica reports — piano-1 **81/87**. Any deviation is
//! an integration bug between the engine and the replayed semantics.
//!
//! Usage: cargo run --release --example validate_engine_lock -- [BASE_DIR]

use anyhow::{Context, Result, anyhow};
use realfft::{RealFftPlanner, RealToComplex};
use std::env;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use crossbeam_queue::ArrayQueue;
use tuner_core::algorithms::spectral::{fft, magnitude_spectrum};
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE, WINDOW_SIZE};
use tuner_core::engine::Engine;
use tuner_core::gatekeeper::{Gatekeeper, SignalState};
use tuner_core::models::{KeyProfile, NOTES, get_expected_beta};
use tuner_core::pipeline::ProcessingFrame;

fn register(key: usize) -> usize {
    if key <= 26 {
        0
    } else if key <= 59 {
        1
    } else {
        2
    }
}

/// Drive the real auto-mode engine over one capture; return the first latched key.
fn first_lock(
    key_dir: &Path,
    fft_bass: &Arc<dyn RealToComplex<f32>>,
    fft_gate: &Arc<dyn RealToComplex<f32>>,
    profiles: &[KeyProfile; 88],
) -> Result<Option<Option<u8>>> {
    let mut raw = key_dir.join("audio_full_event.raw");
    if !raw.exists() {
        raw = key_dir.join("audio.raw");
    }
    if !raw.exists() {
        return Ok(None);
    }

    let mut noise_floor = 0.0f32;
    if let Ok(json_str) = fs::read_to_string(key_dir.join("analysis.json"))
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(&json_str)
        && let Some(nf) = json["metadata"]["noise_floor"].as_f64()
    {
        noise_floor = nf as f32;
    }
    if noise_floor <= 0.0 {
        noise_floor = 0.001;
    }

    let bytes = fs::read(&raw).context("read raw")?;
    if bytes.len() % 4 != 0 {
        return Err(anyhow!("raw not f32-aligned"));
    }
    let n = bytes.len() / 4;
    let mut audio = vec![0.0f32; n];
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), audio.as_mut_ptr() as *mut u8, bytes.len());
    }
    if n < BASS_WINDOW_SIZE {
        return Ok(None);
    }

    let mut frame = ProcessingFrame::new();
    let gk_pool = Arc::new(ArrayQueue::new(1));
    let mut gk = Gatekeeper::new(gk_pool);
    gk.config.silence_threshold = noise_floor;
    let mut engine = Engine::new(44100);
    engine.noise_floor = noise_floor;

    let mut cursor = 0usize;
    while cursor + BASS_WINDOW_SIZE <= n {
        let win = &audio[cursor..cursor + BASS_WINDOW_SIZE];
        cursor += HOP_SIZE;

        // Populate the frame exactly as AudioPipeline::process_cola_hop does:
        // full bass window in audio_buffer, treble FFT for the gatekeeper, bass
        // FFT + magnitude for discovery.
        frame.audio_buffer[..BASS_WINDOW_SIZE].copy_from_slice(win);
        let newest = BASS_WINDOW_SIZE - WINDOW_SIZE;
        fft(
            &frame.audio_buffer[newest..BASS_WINDOW_SIZE],
            &mut frame.time_buffer[..WINDOW_SIZE],
            &mut frame.frequency_buffer[..],
            fft_gate,
            WINDOW_SIZE,
        );
        let gate = gk.process_frame(&frame);

        fft(
            &frame.audio_buffer[..BASS_WINDOW_SIZE],
            &mut frame.time_buffer[..BASS_WINDOW_SIZE],
            &mut frame.bass_frequency_buffer[..],
            fft_bass,
            BASS_WINDOW_SIZE,
        );
        let mag_count = BASS_WINDOW_SIZE / 2;
        magnitude_spectrum(
            &frame.bass_frequency_buffer,
            BASS_WINDOW_SIZE,
            &mut frame.bass_magnitude_buffer[..mag_count],
        );

        let is_silence = gate.state == SignalState::Silence;
        let is_stable = gate.state == SignalState::Stable;
        engine.process(
            &frame,
            profiles,
            is_silence,
            is_stable,
            gate.is_new_onset,
            gate.is_transient_bypass,
            None, // AUTO mode — the path under test
        );

        // First latch wins (single-note captures; a later onset would reset it).
        if let Some(k) = engine.identified_key {
            return Ok(Some(Some(k)));
        }
    }
    Ok(Some(None)) // never locked
}

fn main() -> Result<()> {
    let base = env::args()
        .nth(1)
        .unwrap_or_else(|| "diagnostics_piano_1".to_string());
    let base = Path::new(&base);

    let mut planner = RealFftPlanner::<f32>::new();
    let fft_bass = planner.plan_fft_forward(BASS_WINDOW_SIZE);
    let fft_gate = planner.plan_fft_forward(WINDOW_SIZE);

    let mut profiles_vec = Vec::with_capacity(88);
    for i in 0..88 {
        profiles_vec.push(KeyProfile::new(
            NOTES[i].frequency,
            get_expected_beta(i as u8),
        ));
    }
    let profiles: [KeyProfile; 88] = profiles_vec.try_into().unwrap();

    let mut dirs: Vec<_> = fs::read_dir(base)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.starts_with("key_"))
                .unwrap_or(false)
        })
        .collect();
    dirs.sort();

    println!(
        "engine auto-lock validation | base={} | {} captures | shipped (M,N)=(7,8)",
        base.display(),
        dirs.len()
    );

    let mut n = 0usize;
    let mut ok = 0usize;
    let mut reg = [[0usize; 2]; 3]; // [reg][ok, total]
    let mut fails = Vec::new();

    for d in &dirs {
        let expected: usize = d
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| s.split('_').nth(1))
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| anyhow!("bad dir name"))?;
        let Some(lock) = first_lock(d, &fft_bass, &fft_gate, &profiles)? else {
            continue;
        };
        n += 1;
        let r = register(expected);
        reg[r][1] += 1;
        let pass = lock == Some(expected as u8);
        ok += pass as usize;
        reg[r][0] += pass as usize;
        if !pass {
            let got = match lock {
                Some(k) => format!("locked {k}"),
                None => "never locked".to_string(),
            };
            fails.push(format!(
                "{} -> {got}",
                d.file_name().unwrap().to_str().unwrap()
            ));
        }
    }

    println!(
        "\nENGINE AUTO LOCK: {ok}/{n}   bass {}/{}  mid {}/{}  treble {}/{}",
        reg[0][0], reg[0][1], reg[1][0], reg[1][1], reg[2][0], reg[2][1]
    );
    if !fails.is_empty() {
        println!("FAILURES ({}):", fails.len());
        for f in &fails {
            println!("  {f}");
        }
    }
    Ok(())
}
