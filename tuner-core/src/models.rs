//! # Domain Data Models
//!
//! Domain types for the tuner: notes and the 88-key lookup tables ([`NOTES`],
//! [`Note`], [`NOTE_MAP`]), captured measurements ([`Partial`], [`KeyMeasurement`],
//! [`InharmonicityProfile`]), and the discovery templates ([`KeyProfile`]).
//!
//! It also holds the small body of *domain-specific* math that produces those types —
//! the Rigaud inharmonicity prior ([`get_expected_beta`]), the Railsback stretch curve
//! ([`railsback_stretch_curve`]), and the stiff-string partial law in
//! [`KeyProfile::new`]. That math lives here rather than in `algorithms/` because it
//! encodes piano-domain knowledge, not reusable buffer-level DSP primitives (see
//! `docs/internals/04-algorithms-and-models.md`).

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Highest representable partial frequency — the Nyquist limit.
///
/// Derived from [`crate::audio::SAMPLE_RATE`]: the one spot where `models` reaches up
/// into `audio` (which already depends on `models`), forming a small `models ↔ audio`
/// cycle. Accepted for now — the crate already has intra-crate cycles. A future refactor
/// extracts the shared DSP/stream constants into a leaf module (tracked in the README's
/// Project Work in Progress).
const NYQUIST_HZ: f32 = crate::audio::SAMPLE_RATE as f32 / 2.0;

/// Maximum number of partials modeled per key.
pub const MAX_PARTIALS: usize = 128;

/// A single measured partial (overtone) of a piano note.
///
/// The fundamental is `number = 1`. Overtones are `number = 2, 3, …`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Partial {
    /// The partial number ($n$). 1 = fundamental, 2 = first overtone, etc.
    pub number: u32,
    /// The measured frequency of this partial in Hz.
    pub frequency: f32,
    /// Amplitude of this partial (for spectral envelope analysis).
    pub amplitude: f32,
}

/// Stores all measured partials for a single piano key, plus the computed
/// inharmonicity constant ($B$).
///
/// Created by the capture processing pipeline after the Gatekeeper triggers
/// a successful capture and the Worker runs partial extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyMeasurement {
    /// The 88-key piano index (0 = A0, 87 = C8).
    pub key_index: u8,
    /// Measured fundamental frequency (Hz).
    pub measured_f0: f32,
    /// All measured partials for this key (fundamental + overtones).
    pub partials: Vec<Partial>,
    /// The computed inharmonicity coefficient, or `None` if not yet calculated
    /// or if there were insufficient partials.
    pub calculated_b: Option<f32>,
    /// UTC timestamp of the most recent capture (ISO format).
    pub last_captured: String,
}

/// Canonical on-disk filename for the persisted [`InharmonicityProfile`].
///
/// The pipeline loads this at startup (so a previously-calibrated instrument
/// benefits immediately) and the frontend persists to it. Defined here so the
/// load path and the save path agree on a single name rather than duplicating a
/// string literal across crates.
pub const PROFILE_PATH: &str = "tuning_profile.json";

/// The complete inharmonicity profile for a specific piano.
///
/// This is the top-level serializable object saved to and loaded from a JSON file.
/// It maps each measured key index to its [`KeyMeasurement`] data. A `BTreeMap`
/// keeps keys sorted automatically for clean serialization.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InharmonicityProfile {
    /// Maps a piano key index (0–87) to its measurement data.
    pub measurements: BTreeMap<u8, KeyMeasurement>,
}

impl InharmonicityProfile {
    /// Saves the inharmonicity profile to a JSON file.
    pub fn to_file(&self, path: &str) -> std::io::Result<()> {
        let json_string = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let mut file = std::fs::File::create(path)?;
        use std::io::Write;
        file.write_all(json_string.as_bytes())?;
        Ok(())
    }

    /// Loads an inharmonicity profile from a JSON file.
    pub fn from_file(path: &str) -> std::io::Result<Self> {
        let mut file = std::fs::File::open(path)?;
        let mut data = String::new();
        use std::io::Read;
        file.read_to_string(&mut data)?;
        let profile: Self = serde_json::from_str(&data).map_err(std::io::Error::other)?;
        Ok(profile)
    }
}

/// Represents a single musical note with its name and frequency.
#[derive(Debug, Clone)]
pub struct Note {
    /// Note name (e.g., "A4", "C#3", "Bb2")
    pub name: String,
    /// Frequency in Hz
    pub frequency: f32,
}

/// Statically computed notes for a standard 88-key piano (A0 to C8).
///
/// This lazy static contains all 88 piano keys with their corresponding
/// frequencies calculated using equal temperament tuning with A4 = 440 Hz.
/// The notes are computed once at startup for optimal performance.
pub static NOTES: Lazy<Vec<Note>> = Lazy::new(|| {
    const NOTE_NAMES: [&str; 12] = [
        "A", "A#", "B", "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#",
    ];
    let mut notes = Vec::with_capacity(88);

    for i in 0..88 {
        // A4 is the 49th key, which is index 48 in a 0-indexed loop.
        let frequency = 440.0 * 2.0_f32.powf((i as f32 - 48.0) / 12.0);

        let note_index = i % 12;
        let octave = (i + 9) / 12;
        let name = format!("{}{}", NOTE_NAMES[note_index], octave);

        notes.push(Note { name, frequency });
    }
    notes
});

/// Static map for quick note name to key index lookups.
///
/// This provides O(log n) lookup time for converting note names
/// (like "A4", "C#3") to their corresponding piano key indices.
pub static NOTE_MAP: Lazy<BTreeMap<String, u8>> = Lazy::new(|| {
    NOTES
        .iter()
        .enumerate()
        .map(|(i, note)| (note.name.clone(), i as u8))
        .collect()
});

/// Finds the closest musical note to a given frequency.
///
/// This function searches through all 88 piano keys to find the one
/// with the frequency closest to the input frequency. It's used for
/// automatic note detection in the tuner.
///
/// # Arguments
/// * `freq` - Input frequency in Hz
///
/// # Returns
/// * `(note_name, target_frequency)` - Closest note name and its target frequency
pub fn find_nearest_note(freq: f32) -> (String, f32) {
    let closest = NOTES
        .iter()
        .min_by(|a, b| {
            let diff_a = (a.frequency - freq).abs();
            let diff_b = (b.frequency - freq).abs();
            diff_a.partial_cmp(&diff_b).unwrap()
        })
        .unwrap(); // This is safe as NOTES is never empty.

    (closest.name.clone(), closest.frequency)
}

/// Finds a note's name and frequency by its 88-key piano index.
///
/// This function provides direct access to note information using
/// the piano key index (0-87, where 0 is A0 and 87 is C8).
///
/// # Arguments
/// * `key_index` - Piano key index (0-87)
///
/// # Returns
/// * `(note_name, frequency)` - Note name and frequency
pub fn find_nearest_note_by_index(key_index: u8) -> (String, f32) {
    let note = &NOTES[key_index as usize];
    (note.name.clone(), note.frequency)
}

/// Returns the 88-key piano index (0–87) of the note closest to `freq`.
///
/// Unlike [`find_nearest_note()`], this avoids a `String` allocation and is
/// suitable for use on the DSP hot path or in pipeline output types.
///
/// # Arguments
/// * `freq` - Input frequency in Hz
///
/// # Returns
/// * Piano key index (0 = A0, 87 = C8)
pub fn find_nearest_note_index(freq: f32) -> u8 {
    NOTES
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            let diff_a = (a.frequency - freq).abs();
            let diff_b = (b.frequency - freq).abs();
            diff_a.partial_cmp(&diff_b).unwrap()
        })
        .map(|(i, _)| i as u8)
        .unwrap() // Safe: NOTES is never empty.
}

/// Gets the 88-key piano index from a note name.
///
/// This function converts note names like "A4" or "C#3" to their
/// corresponding piano key indices for use in the GUI.
///
/// # Arguments
/// * `name` - Note name (e.g., "A4", "C#3", "Bb2")
///
/// # Returns
/// * Piano key index (0-87), defaults to 0 if note not found
pub fn get_key_index_from_name(name: &str) -> u8 {
    *NOTE_MAP.get(name).unwrap_or(&0)
}

/// Returns the expected physical inharmonicity coefficient (beta) for a given piano key.
///
/// Implements Rigaud's dual-exponential whole-compass model (Eqs. 7–8):
/// B(m) = e^(s_B·m + y_B) + e^(s_T·m + y_T), the sum of the bass- and
/// treble-bridge log-linear asymptotes, with m the MIDI note number. Here,
/// re-indexed to 1-indexed keys via m = n + 20 (A0: n = 1 ↔ m = 21):
///
///   B(n) = exp(-0.066n - 9.211) + exp(0.0926n - 11.788)
///
/// Constant provenance (faithfulness-audit-06):
/// * **Treble pair = the paper's universal fit**, verified exact:
///   (s_T, y_T) = (9.26e-2, −13.64) ⇒ 0.0926·(n+20) − 13.64 = 0.0926n − 11.788.
///   The paper fixes these across all pianos (after Young 1952).
/// * **Bass pair = OURS** — the paper defines (s_B, y_B) as *piano-specific*
///   free parameters (no universal value exists); ours (−6.6e-2, −7.891 in
///   MIDI domain) is a typical medium-piano default. Known domain limit: the
///   real upright's measured bass B runs 7–25× this default (ADR 0006) —
///   inherent to any fixed bass choice, which is why measured-B seeding
///   exists (gated off pending validation on a second instrument).
///
/// # Reference
/// 1. Rigaud, F., David, B., & Daudet, L. (2013). "A parametric model and estimation techniques
///    for the inharmonicity and tuning of the piano". JASA 133(5), pp. 3107-3118.
///    DOI: 10.1121/1.4802644 (Eqs. 7-8; treble universality §IV.)
pub fn get_expected_beta(key_index: u8) -> f32 {
    // Rigaud model uses a 1-indexed key number (A0 = 1).
    // key_index is 0-indexed (A0 = 0), so we offset by 1.
    let n = key_index as f32 + 1.0;
    (-0.066 * n - 9.211).exp() + (0.0926 * n - 11.788).exp()
}

/// Precomputed per-key discovery template: the predicted stiff-string partial
/// series the matcher scores observed peaks against.
#[derive(Debug, Clone)]
pub struct KeyProfile {
    /// Equal-temperament fundamental this template is centered on (Hz).
    pub f0_et: f32,
    /// Inharmonicity coefficient ($B$) used to stretch the partials.
    pub beta: f32,
    /// Predicted partial frequencies (Hz); valid entries are `[0..valid_partial_count]`.
    pub predicted_partials: [f32; MAX_PARTIALS],
    /// Number of partials that fall below Nyquist.
    pub valid_partial_count: usize,
}

impl KeyProfile {
    /// Builds a template from a fundamental and inharmonicity coefficient via the
    /// stiff-string law `f_n = n·f0·√(1 + B·n²)`, dropping partials above Nyquist.
    pub fn new(f0_et: f32, beta: f32) -> Self {
        let mut predicted_partials = [0.0; MAX_PARTIALS];
        let mut valid_partial_count = 0;

        for n in 1..=MAX_PARTIALS {
            let n_f32 = n as f32;
            let f_n = n_f32 * f0_et * (1.0 + beta * n_f32 * n_f32).sqrt();
            if f_n < NYQUIST_HZ {
                predicted_partials[n - 1] = f_n;
                valid_partial_count += 1;
            } else {
                break;
            }
        }

        Self {
            f0_et,
            beta,
            predicted_partials,
            valid_partial_count,
        }
    }

    /// The Rigaud-prior template for a key: equal-temperament center, expected $B$.
    pub fn prior(key_index: u8) -> Self {
        Self::new(NOTES[key_index as usize].frequency, get_expected_beta(key_index))
    }

    /// Builds a template from a measured key, using its measured inharmonicity $B$ in
    /// place of the prior. Returns `None` when $B$ is absent or non-physical (so the
    /// caller keeps the prior).
    ///
    /// The template is centered on equal temperament, not the measured `f0`: $B$ is the
    /// tuning-invariant string-shape parameter, whereas a stored `f0` goes stale as the
    /// string is tuned. Stage-B refinement absorbs the live pitch offset.
    pub fn from_measurement(m: &KeyMeasurement) -> Option<Self> {
        let beta = m.calculated_b?;
        if !beta.is_finite() || beta <= 0.0 {
            return None;
        }
        let f0_et = NOTES.get(m.key_index as usize)?.frequency;
        Some(Self::new(f0_et, beta))
    }
}

/// Builds the full 88-key prior template table.
pub fn build_default_profiles() -> Box<[KeyProfile; 88]> {
    let v: Vec<KeyProfile> = (0..88).map(|k| KeyProfile::prior(k as u8)).collect();
    Box::new(v.try_into().unwrap_or_else(|_| unreachable!()))
}

/// Abramowitz & Stegun 7.1.26 error-function approximation (|err| < 1.5e-7).
fn erf(x: f32) -> f32 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = ((((1.061_405_4 * t - 1.453_152_1) * t + 1.421_413_8) * t - 0.284_496_72) * t
        + 0.254_829_6)
        * t;
    sign * (1.0 - poly * (-x * x).exp())
}

/// Canonical Railsback stretch (cents vs equal temperament) for each of the 88
/// keys, from the Rigaud (2011/2013) inharmonicity-coupled octave-stretch model
/// with mean parameters (type-octave K≈4.51, m0≈64, α≈24; A4 anchored at 440).
///
/// Discovery currently scores templates at raw ET, which handicaps every note by
/// its stretch (worst in the extreme treble). Centering the per-key template on
/// this expected tuned pitch removes that systematic handicap; Stage-B refinement
/// then absorbs the residual per-instrument / pitch-raise offset. This is the same
/// model the synthetic generator uses, so the engine reference and the synthetic
/// dataset stay consistent.
///
/// # Reference
/// Rigaud, F., David, B., & Daudet, L. (2011). "A parametric model of piano
/// tuning". Proc. DAFx-11. (Eqs. 8, 12–14.)
pub fn railsback_stretch_curve() -> [f32; 88] {
    let b: [f32; 88] = core::array::from_fn(|k| get_expected_beta(k as u8));
    let et = |k: usize| -> f32 { NOTES[k].frequency };
    // Type-octave amount ρ(key), mean fit (decreasing bass→treble).
    let rho = |key: usize| -> f32 {
        let m = key as f32 + 21.0; // MIDI index
        (4.51 / 2.0) * (1.0 - erf((m - 64.0) / 24.0)) + 1.0
    };

    let mut f0 = [0.0f32; 88]; // flexible-string fundamentals
    let mut f1 = [0.0f32; 88]; // measured first partials = f0·√(1+B)
    // Anchor A4 (key 48): f1=440 ⇒ f0 = 440/√(1+B).
    f0[48] = 440.0 / (1.0 + b[48]).sqrt();
    for a in [60usize, 72, 84] {
        let r = rho(a);
        f0[a] = 2.0 * f0[a - 12] * ((1.0 + b[a - 12] * 4.0 * r * r) / (1.0 + b[a] * r * r)).sqrt();
    }
    for a in [36usize, 24, 12, 0] {
        let r = rho(a + 12);
        f0[a] =
            f0[a + 12] / (2.0 * ((1.0 + b[a] * 4.0 * r * r) / (1.0 + b[a + 12] * r * r)).sqrt());
    }
    for a in [0usize, 12, 24, 36, 48, 60, 72, 84] {
        f1[a] = f0[a] * (1.0 + b[a]).sqrt();
    }
    // Semitone fill inside each A–A octave (Eq. 12–14).
    let mut last_lambda = 0.0f32;
    for a in [0usize, 12, 24, 36, 48, 60, 72] {
        let b_sum: f32 = (1..=12).map(|p| b[a + p]).sum();
        let lambda = 24.0 * (f1[a + 12] / (2.0 * f1[a])).ln() / b_sum.max(1e-9);
        last_lambda = lambda;
        for p in 1..12 {
            f1[a + p] = f1[a + p - 1] * (2.0 + lambda * b[a + p]).powf(1.0 / 12.0);
        }
    }
    for k in 85..88 {
        f1[k] = f1[k - 1] * (2.0 + last_lambda * b[k]).powf(1.0 / 12.0);
    }

    core::array::from_fn(|k| 1200.0 * (f1[k] / et(k)).log2())
}
