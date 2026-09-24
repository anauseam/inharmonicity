//! End-to-end validation of the shipped engine auto-lock path.
//!
//! The Python replicas validate the M-of-N rule (report 0010), not its
//! integration, since `cargo lab engine dump` drives the engine in manual mode.
//! This drives the real `Engine::process` in auto mode (`target_note = None`)
//! frame by frame over each capture, feeding the gatekeeper's verdicts exactly as
//! the live pipeline does, and records the first key the engine latches. The auto
//! path always refines, so with the shipped `LOCK_VOTES_M = 7 / LOCK_WINDOW_N = 8`
//! this must reproduce the replica's refined (7,8) figure, piano-1 81/87; any
//! deviation is an integration bug.
//!
//! The `from-onset` mode runs the counterfactual: the gatekeeper withholds the
//! engine's vote until `Stable`, five hops (116 ms) after the NHWRSF onset, on the
//! ground that the attack is broadband. Here the Stage-A scan, the same
//! `discovery::discover` call `Engine::process` makes on the same 8192-point bass
//! spectrum, runs on every hop from the onset whatever the gate says, and each
//! hop's winner is scored against the capture's key, bucketed by hops since
//! onset. The gate's own verdict is carried per hop, so the shipped policy sits
//! inside the table rather than bounding it.
//!
//! Usage: `cargo lab engine lock [BASE_DIR]`
//!        `cargo lab engine from-onset diagnostics_piano2`

use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use realfft::{RealFftPlanner, RealToComplex};

use crate::{capture, raw};
use tuner_core::algorithms::spectral::{fft, magnitude_spectrum};
use tuner_core::algorithms::{discovery, peaks, twm};
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE, SAMPLE_RATE, WINDOW_SIZE};
use tuner_core::engine::Engine;
use tuner_core::gatekeeper::{GateResult, Gatekeeper, SignalState};
use tuner_core::models::{KeyProfile, NOTES, SpectralPeak, get_expected_beta};
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

/// Finer split for the onset sweep than [`register`]: what the gate's wait buys
/// separates on attack timbre, which is a treble-versus-everything-else story
/// that the lock report's three-way split blurs.
const ONSET_REGISTERS: [(&str, u8, u8); 4] = [
    ("bass A0-B1", 0, 14),
    ("tenor C2-B3", 15, 38),
    ("mid C4-B5", 39, 62),
    ("treble C6-C8", 63, 87),
];

/// Hops after the onset hop that get their own bucket; later hops pool.
const MAX_HOP: usize = 24;

/// The capture's logged silence threshold (`metadata.noise_floor`), or 0.001
/// where the dump carries none.
fn capture_noise_floor(dir: &Path) -> f32 {
    fs::read_to_string(dir.join("analysis.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|j| j["metadata"]["noise_floor"].as_f64())
        .map(|v| v as f32)
        .filter(|v| *v > 0.0)
        .unwrap_or(0.001)
}

/// Populate `frame` from one hop's bass window and run the gatekeeper over it,
/// exactly as `AudioPipeline::process_cola_hop` does: the gate's verdict comes
/// from a treble FFT over the newest `WINDOW_SIZE` samples.
fn gate_hop(
    frame: &mut ProcessingFrame,
    win: &[f32],
    gk: &mut Gatekeeper,
    fft_gate: &Arc<dyn RealToComplex<f32>>,
) -> GateResult {
    frame.audio_buffer[..BASS_WINDOW_SIZE].copy_from_slice(win);
    let newest = BASS_WINDOW_SIZE - WINDOW_SIZE;
    fft(
        &frame.audio_buffer[newest..BASS_WINDOW_SIZE],
        &mut frame.time_buffer[..WINDOW_SIZE],
        &mut frame.frequency_buffer[..],
        fft_gate,
        WINDOW_SIZE,
    );
    gk.process_frame(frame)
}

/// The bass FFT and magnitude spectrum discovery reads. Split out of
/// [`gate_hop`] so the sweep can skip it on hops it will not score.
fn bass_spectrum(frame: &mut ProcessingFrame, fft_bass: &Arc<dyn RealToComplex<f32>>) {
    fft(
        &frame.audio_buffer[..BASS_WINDOW_SIZE],
        &mut frame.time_buffer[..BASS_WINDOW_SIZE],
        &mut frame.bass_frequency_buffer[..],
        fft_bass,
        BASS_WINDOW_SIZE,
    );
    magnitude_spectrum(
        &frame.bass_frequency_buffer,
        BASS_WINDOW_SIZE,
        &mut frame.bass_magnitude_buffer[..BASS_WINDOW_SIZE / 2],
    );
}

/// One scored hop of the onset sweep.
struct Hop {
    key: u8,
    since_onset: usize,
    stable: bool,
    winner: u8,
}

/// Stage-A's winner on every hop from the onset, gate state ignored.
fn score_from_onset(
    dir: &Path,
    key: u8,
    fft_bass: &Arc<dyn RealToComplex<f32>>,
    fft_gate: &Arc<dyn RealToComplex<f32>>,
    profiles: &[KeyProfile; 88],
) -> Vec<Hop> {
    let mut out = Vec::new();
    let Some(audio) = raw::full_event(dir) else {
        return out;
    };
    if audio.len() < BASS_WINDOW_SIZE {
        return out;
    }
    let nf = capture_noise_floor(dir);
    let mut frame = ProcessingFrame::new();
    let mut gk = Gatekeeper::new();
    gk.config.silence_threshold = nf;
    let mut scratch = vec![SpectralPeak::default(); 64];
    let cfg = twm::TwmConfig::default();
    // `Engine::process`'s Neyman–Pearson magnitude gate, P_fa = 0.001.
    let p_bin = nf * nf * 0.375 * BASS_WINDOW_SIZE as f32;
    let min_magnitude = if p_bin > 0.0 {
        (-p_bin * 0.001_f32.ln()).sqrt()
    } else {
        0.0
    };

    let mut onset_hop: Option<usize> = None;
    let mut hop = 0usize;
    let mut cursor = 0usize;
    while cursor + BASS_WINDOW_SIZE <= audio.len() {
        let win = &audio[cursor..cursor + BASS_WINDOW_SIZE];
        cursor += HOP_SIZE;
        hop += 1;
        let gate = gate_hop(&mut frame, win, &mut gk, fft_gate);
        if gate.is_new_onset && onset_hop.is_none() {
            onset_hop = Some(hop);
        }
        let Some(h0) = onset_hop else {
            continue;
        };
        if gate.state == SignalState::Silence {
            break;
        }
        bass_spectrum(&mut frame, fft_bass);
        let n = peaks::extract_peaks(
            &frame.bass_magnitude_buffer[..BASS_WINDOW_SIZE / 2],
            &frame.bass_frequency_buffer[..],
            SAMPLE_RATE,
            BASS_WINDOW_SIZE,
            min_magnitude,
            &mut scratch,
        );
        let valid = peaks::mask_peaks(&mut scratch[..n.min(64)]);
        out.push(Hop {
            key,
            since_onset: hop - h0,
            stable: gate.state == SignalState::Stable,
            winner: discovery::discover(&scratch[..valid], profiles, &cfg, true).key_index,
        });
    }
    out
}

fn from_onset_report(base: &Path, captures: usize, hops: &[Hop]) {
    println!(
        "\n{}: {captures} captures, {} scored hops. Stage-A winner == key, % per hop since onset (hop = 23.2 ms; 'S' = gate Stable on ≥ half of that bucket's hops)",
        base.display(),
        hops.len()
    );
    print!("{:14}", "register");
    for h in 0..=MAX_HOP {
        print!("{h:>6}");
    }
    println!("{:>7}", "25+");
    for (name, lo, hi) in ONSET_REGISTERS {
        let sel: Vec<&Hop> = hops.iter().filter(|h| h.key >= lo && h.key <= hi).collect();
        if sel.is_empty() {
            continue;
        }
        print!("{name:14}");
        let bucket = |pred: &dyn Fn(usize) -> bool| {
            let b: Vec<&&Hop> = sel.iter().filter(|h| pred(h.since_onset)).collect();
            if b.is_empty() {
                return None;
            }
            let correct = b.iter().filter(|h| h.winner == h.key).count();
            let stable = b.iter().filter(|h| h.stable).count();
            Some((
                100.0 * correct as f32 / b.len() as f32,
                stable * 2 >= b.len(),
            ))
        };
        for h in 0..=MAX_HOP {
            match bucket(&|x| x == h) {
                Some((pct, st)) => print!("{:>5.0}{}", pct, if st { "S" } else { " " }),
                None => print!("{:>6}", "-"),
            }
        }
        match bucket(&|x| x > MAX_HOP) {
            Some((pct, st)) => println!("{:>6.0}{}", pct, if st { "S" } else { " " }),
            None => println!("{:>7}", "-"),
        }
    }
}

/// Drive the real auto-mode engine over one capture; return the first latched key.
fn first_lock(
    key_dir: &Path,
    fft_bass: &Arc<dyn RealToComplex<f32>>,
    fft_gate: &Arc<dyn RealToComplex<f32>>,
    profiles: &[KeyProfile; 88],
) -> Result<Option<Option<u8>>> {
    let Some(audio) = raw::full_event(key_dir).or_else(|| raw::stable(key_dir)) else {
        return Ok(None);
    };
    let noise_floor = capture_noise_floor(key_dir);
    let n = audio.len();
    if n < BASS_WINDOW_SIZE {
        return Ok(None);
    }

    let mut frame = ProcessingFrame::new();
    let mut gk = Gatekeeper::new();
    gk.config.silence_threshold = noise_floor;
    let mut engine = Engine::new(44100);
    engine.noise_floor = noise_floor;

    let mut cursor = 0usize;
    while cursor + BASS_WINDOW_SIZE <= n {
        let win = &audio[cursor..cursor + BASS_WINDOW_SIZE];
        cursor += HOP_SIZE;

        let gate = gate_hop(&mut frame, win, &mut gk, fft_gate);
        bass_spectrum(&mut frame, fft_bass);

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

pub fn run(base: &Path, from_onset: bool) -> Result<()> {
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

    let dirs = capture::find(base).with_context(|| format!("read dir {}", base.display()))?;

    if from_onset {
        let mut hops = Vec::new();
        for d in &dirs {
            let Some(key) = capture::key_of(d) else {
                continue;
            };
            hops.extend(score_from_onset(d, key, &fft_bass, &fft_gate, &profiles));
        }
        from_onset_report(base, dirs.len(), &hops);
        return Ok(());
    }

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
        let expected = capture::key_of(d).ok_or_else(|| anyhow!("bad dir name"))? as usize;
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
