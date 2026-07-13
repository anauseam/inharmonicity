//! # Additive resynthesis of a tuning curve to audio (headless, cold-path)
//!
//! Renders short piano-like audio from a [`TuningCurve`] plus the per-key
//! measured timbre in a [`CurveInput`], by **additive sine summation**. Its
//! purpose is auralization: hearing how a candidate curve sounds *before*
//! tuning a piano to it — the perceptual A/B that no statistic can decide
//! (there is no ground-truth-free "best" stretch; ADR 0009).
//!
//! This module is **pure, thread-free, and NOT on the real-time hot path**: it
//! runs on no pipeline thread, holds no shared state, allocates freely, and
//! produces a buffer of `f32` samples (or writes a WAV). It owns **no audio
//! stream** — playing the samples through a speaker is the caller's job.
//!
//! ## Method (design note §1/§5/§7 conventions)
//!
//! For a note on key `m`, partial `n` is placed at
//! `f_n = n·f₀·√(1 + B·n²)`, with `f₀` chosen so the audible first partial
//! `f₁ = f₀·√(1+B)` equals the curve's target for that key
//! ([`TuningCurve::target_f1`]), using the key's **raw measured B**. So the
//! partials sit where the physical string's would when tuned to the curve —
//! the same placement [`TuningCurve::strobe_partials`] uses. Amplitudes are
//! the measured partial amplitudes (the timbre). A per-partial exponential
//! decay ([`EnvelopeParams`]) plus a short onset ramp shape each note.
//!
//! The envelope is a **plausible heuristic**, not a measured model: it exists
//! only to sustain notes long enough that coincident-partial beats are
//! audible. The beat *rates* come entirely from the curve, never the envelope.
//!
//! Oscillators are rotating phasors (two multiplies per sample, no `sin`/`exp`
//! in the inner loop; periodically renormalized), and note amplitudes are
//! equal-power-normalized so level does not depend on the measurement's
//! absolute magnitude scale. The returned buffer is **un-normalized** — the
//! caller sets the level (loudness-match a set with one shared scale via
//! [`peak`], or peak-normalize a single buffer).

use std::f64::consts::TAU;

use crate::audio::SAMPLE_RATE;
use crate::models::{CurveInput, TuningCurve};

/// Sample rate as `f64`, for the cold-path math here.
const SR: f64 = SAMPLE_RATE as f64;
/// Nyquist frequency — partials at or above this are dropped.
const NYQUIST_HZ: f64 = SR / 2.0;
/// Phasor magnitude is renormalized this often (samples) to bound drift.
const RENORM_EVERY: usize = 4096;

/// One note event on the synthesis timeline.
#[derive(Debug, Clone, Copy)]
pub struct Note {
    /// 88-key piano index (0 = A0 … 87 = C8). External MIDI sources convert
    /// via [`crate::models::key_from_midi`].
    pub key: u8,
    /// Loudness in `0.0..=1.0` (a normalized velocity; MIDI velocity maps as
    /// `velocity / 127`). Scales the note's equal-power amplitude.
    pub intensity: f32,
    /// Onset time in seconds from the start of the render.
    pub start_s: f64,
    /// Sounding duration in seconds.
    pub dur_s: f64,
}

/// Per-partial decay + onset envelope (a heuristic auralization envelope, not
/// a measured model — see the module doc). [`Default`] reproduces the shipped
/// values; expose it to a UI later if a brightness/sustain knob is wanted.
#[derive(Debug, Clone, Copy)]
pub struct EnvelopeParams {
    /// Fundamental decay time constant τ₀ (s) at the bottom of the compass
    /// (A0). Bass strings ring longest.
    pub tau0_bass_s: f64,
    /// τ₀ (s) at the top of the compass (C8); treble rings shortest. τ₀ is
    /// linearly interpolated across the 88 keys between the two.
    pub tau0_treble_s: f64,
    /// Higher partials decay faster: `τ_n = τ₀ · n^(−partial_exponent)`.
    pub partial_exponent: f64,
    /// Linear onset ramp (s) to declick the note start.
    pub attack_s: f64,
}

impl Default for EnvelopeParams {
    fn default() -> Self {
        Self {
            tau0_bass_s: 3.0,
            tau0_treble_s: 0.8,
            partial_exponent: 0.6,
            attack_s: 0.006,
        }
    }
}

impl EnvelopeParams {
    /// τ₀ for a key, linearly interpolated bass → treble across the compass.
    fn tau0_for_key(&self, key: u8) -> f64 {
        let frac = key as f64 / 87.0;
        self.tau0_bass_s + (self.tau0_treble_s - self.tau0_bass_s) * frac
    }
}

/// Frequency of stiff-string partial `n` for a string whose audible first
/// partial is `f1` with inharmonicity `b`:
///
///   `f_n = n·f₀·√(1 + B·n²)`,  `f₀ = f₁/√(1+B)`  (design note §7).
///
/// The same placement [`TuningCurve::strobe_partials`] uses; exposed so
/// callers (e.g. beat-rate screens) can reproduce the synth's partial layout.
pub fn partial_freq(f1: f64, b: f64, n: u32) -> f64 {
    let f0 = f1 / (1.0 + b).sqrt();
    let n = n as f64;
    n * f0 * (1.0 + b * n * n).sqrt()
}

/// Peak absolute sample value of a buffer — a caller's normalization aid
/// (e.g. `scale = 0.9 / peak(buf)` for a single buffer, or the max of this
/// over several buffers to loudness-match a set).
pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, &s| m.max(s.abs()))
}

/// Render `notes` into a mono `f32` buffer at [`SAMPLE_RATE`] by additive
/// resynthesis (see the module doc). The buffer spans the full timeline
/// (`max(start_s + dur_s)`). Notes whose key carries no trusted measurement
/// in `input` are silently skipped (they render silence). The output is
/// un-normalized — apply the caller's level policy before quantizing/playing.
pub fn render(
    curve: &TuningCurve,
    input: &CurveInput,
    notes: &[Note],
    env: &EnvelopeParams,
) -> Vec<f32> {
    let total_s = notes
        .iter()
        .map(|n| n.start_s + n.dur_s)
        .fold(0.0, f64::max);
    let total_samples = (total_s * SR) as usize + 1;
    // Accumulate in f64 for headroom, then narrow to the f32 output.
    let mut master = vec![0.0f64; total_samples];
    for note in notes {
        render_one(&mut master, note, curve, input, env);
    }
    master.into_iter().map(|s| s as f32).collect()
}

/// A single partial's running oscillator state (rotating phasor + decaying
/// envelope) — no `sin`/`exp` in the per-sample loop.
struct Osc {
    re: f64,
    im: f64, // the sin() component read out each sample
    cos_d: f64,
    sin_d: f64,
    env: f64,   // current amplitude (starts at the equal-power-scaled measured amp)
    decay: f64, // per-sample multiplicative decay = exp(-1/(SR·τ_n))
}

/// Render one note into `master` at its sample offset. Returns `false` (and
/// writes nothing) if the key has no trusted measurement.
fn render_one(
    master: &mut [f64],
    note: &Note,
    curve: &TuningCurve,
    input: &CurveInput,
    env: &EnvelopeParams,
) -> bool {
    let Some(kd) = input.keys[note.key as usize].as_ref() else {
        return false;
    };
    let f1 = curve.target_f1(note.key) as f64;
    let b = kd.b;

    // Audible partials: measured (n, amplitude) placed at the curve/raw-B
    // frequency, dropping anything at or past Nyquist.
    let parts: Vec<(u32, f64, f64)> = kd
        .partials
        .iter()
        .filter_map(|&(n, _f, a)| {
            let fq = partial_freq(f1, b, n);
            (fq < NYQUIST_HZ && a > 0.0).then_some((n, fq, a))
        })
        .collect();
    if parts.is_empty() {
        return false;
    }
    // Equal-power normalization so level is independent of the measurement's
    // absolute magnitude scale; `intensity` then sets the note's loudness.
    let norm = parts.iter().map(|&(_, _, a)| a * a).sum::<f64>().sqrt();
    let scale = note.intensity as f64 / norm;

    let tau0 = env.tau0_for_key(note.key);
    let mut oscs: Vec<Osc> = parts
        .iter()
        .map(|&(n, fq, a)| {
            let dtheta = TAU * fq / SR;
            let tau_n = (tau0 / (n as f64).powf(env.partial_exponent)).max(0.03);
            Osc {
                re: 1.0,
                im: 0.0,
                cos_d: dtheta.cos(),
                sin_d: dtheta.sin(),
                env: a * scale,
                decay: (-1.0 / (SR * tau_n)).exp(),
            }
        })
        .collect();

    let start = (note.start_s * SR) as usize;
    let n_samples = (note.dur_s * SR) as usize;
    let attack = (env.attack_s * SR) as usize;

    for i in 0..n_samples {
        let idx = start + i;
        if idx >= master.len() {
            break;
        }
        let mut s = 0.0;
        for o in &mut oscs {
            s += o.env * o.im;
            o.env *= o.decay;
            let nre = o.re * o.cos_d - o.im * o.sin_d;
            let nim = o.re * o.sin_d + o.im * o.cos_d;
            o.re = nre;
            o.im = nim;
        }
        let a = if attack > 0 && i < attack {
            i as f64 / attack as f64
        } else {
            1.0
        };
        master[idx] += a * s;

        if i % RENORM_EVERY == RENORM_EVERY - 1 {
            for o in &mut oscs {
                let m = (o.re * o.re + o.im * o.im).sqrt();
                if m > 0.0 {
                    o.re /= m;
                    o.im /= m;
                }
            }
        }
    }
    true
}

/// Serialize `samples` to a mono 16-bit PCM WAV byte buffer at
/// [`SAMPLE_RATE`], applying `scale` to each sample before quantizing (the
/// caller's level policy — see [`render`]). Hand-rolled RIFF; no dependency.
fn wav_bytes(samples: &[f32], scale: f32) -> Vec<u8> {
    let sr = SAMPLE_RATE;
    let n_bytes = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + n_bytes).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sr.to_le_bytes());
    out.extend_from_slice(&(sr * 2).to_le_bytes()); // byte rate (mono·16-bit)
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits/sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&n_bytes.to_le_bytes());
    for &s in samples {
        let q = (s * scale * 32767.0).round().clamp(-32768.0, 32767.0) as i16;
        out.extend_from_slice(&q.to_le_bytes());
    }
    out
}

/// Write `samples` to a mono 16-bit PCM WAV file at [`SAMPLE_RATE`], applying
/// `scale` before quantizing (see [`render`]/[`peak`] for the level policy).
pub fn write_wav(path: &str, samples: &[f32], scale: f32) -> std::io::Result<()> {
    std::fs::write(path, wav_bytes(samples, scale))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CurveKeyData, CurveKeyFlags};

    fn flat_curve() -> TuningCurve {
        TuningCurve {
            cents: [0.0; 88],
            d_g: 0.0,
            flags: [CurveKeyFlags::default(); 88],
        }
    }

    #[test]
    fn partial_freq_fundamental_is_f1() {
        // Partial 1 sits exactly at the audible first partial, for any B.
        for &b in &[0.0, 1e-4, 5e-3] {
            assert!((partial_freq(440.0, b, 1) - 440.0).abs() < 1e-9);
        }
    }

    #[test]
    fn partial_freq_stretches_when_inharmonic() {
        // With B > 0 the n-th partial is sharp of the n·f1 harmonic.
        let (f1, b) = (100.0, 2e-3);
        assert!(partial_freq(f1, b, 2) > 2.0 * f1);
        assert!(partial_freq(f1, b, 5) > 5.0 * f1);
    }

    #[test]
    fn render_produces_audio_for_measured_key() {
        let curve = flat_curve();
        let mut input = CurveInput::default();
        input.keys[48] = Some(CurveKeyData {
            b: 1e-3,
            f0: 440.0 / (1.0 + 1e-3f64).sqrt(),
            partials: vec![(1, 440.0, 1.0), (2, 880.0, 0.4)],
        });
        let notes = [Note {
            key: 48,
            intensity: 0.8,
            start_s: 0.0,
            dur_s: 0.5,
        }];
        let buf = render(&curve, &input, &notes, &EnvelopeParams::default());
        assert_eq!(buf.len(), (0.5 * SR) as usize + 1);
        assert!(peak(&buf) > 0.0, "a measured note must not be silent");
    }

    #[test]
    fn render_skips_unmeasured_key() {
        let curve = flat_curve();
        let input = CurveInput::default(); // nothing measured
        let notes = [Note {
            key: 48,
            intensity: 1.0,
            start_s: 0.0,
            dur_s: 0.2,
        }];
        let buf = render(&curve, &input, &notes, &EnvelopeParams::default());
        assert!(
            buf.iter().all(|&s| s == 0.0),
            "unmeasured key renders silence"
        );
    }

    #[test]
    fn wav_header_is_wellformed() {
        let bytes = wav_bytes(&[0.0, 0.5, -0.5, 1.0], 1.0);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(bytes.len(), 44 + 4 * 2); // header + 4 mono 16-bit samples
        let data_len = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]);
        assert_eq!(data_len, 8);
    }
}
