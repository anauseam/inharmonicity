//! What `mat regen` emits, and the rules for consuming it.
//!
//! `capture-sets.md` binds two rules for piano #2: its
//! captures are consumed through the regenerated dump rather than the cached
//! `analysis.json`, and entries whose cached fundamental is tracker garbage are
//! dropped. [`load`] and [`profile`] apply the second and return the count they
//! dropped.

use std::path::Path;

use crate::raw;
use tuner_core::models::{InharmonicityProfile, KeyMeasurement, Partial};
use tuner_core::strobe::MAX_STROBE_REFS;

/// Cents from equal temperament past which a cached measurement is treated as
/// tracker garbage rather than a reading.
///
/// `capture-sets.md`: piano #2's deep-bass entries predate the `worker::MAT_SEED_TOLERANCE`
/// fix and carry rumble-seeded fundamentals — A0 "measured" at 7–14.7 Hz
/// against an ET 27.5. The audio is genuine; only the cached analysis is wrong.
pub const ET_PLAUSIBLE_CENTS: f32 = 200.0;

/// A0, the compass origin — key index 0.
const A0_HZ: f32 = 27.5;

/// Equal-tempered frequency of a key index.
pub fn et_hz(key: u8) -> f32 {
    A0_HZ * 2.0f32.powf(key as f32 / 12.0)
}

/// Whether a cached fundamental `f0` of `key` is a reading rather than tracker
/// garbage ([`ET_PLAUSIBLE_CENTS`]).
fn plausible(key: u8, f0: f32) -> bool {
    f0 > 0.0 && (1200.0 * (f0 / et_hz(key)).log2()).abs() <= ET_PLAUSIBLE_CENTS
}

/// A regenerated-partials dump's entries, as JSON.
fn rows(path: &Path) -> Vec<serde_json::Value> {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// Which of a key's strings sounded, as `mat regen` passes it through from
/// `analysis.json`'s `metadata.sounding_strings`.
///
/// `None` on the capture means the operator declared nothing, which is every
/// capture outside the mute-isolation set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sounding {
    /// How many strings the key is strung with, as declared.
    pub on_key: u8,
    /// Whether each of the key's strings sounded.
    pub sounding: [bool; 3],
}

impl Sounding {
    /// How many strings sounded.
    pub fn count(&self) -> u8 {
        self.sounding.iter().filter(|s| **s).count() as u8
    }

    /// Every string of the key: the note as it is played.
    pub fn is_open(&self) -> bool {
        self.count() > 0 && self.count() == self.on_key
    }

    /// Exactly one string sounding — the false-beat positive control, since a
    /// solo that still resolves two lines is a false beat by construction.
    /// True of a single-strung key's ordinary capture, which needs no mute.
    pub fn is_solo(&self) -> bool {
        self.count() == 1
    }
}

/// One capture as `mat regen` emits it, plus its audio on demand.
#[derive(Debug, Clone)]
pub struct Capture {
    pub key: u8,
    /// Dump directory name — the identity a repeat set needs.
    pub dir: String,
    /// MAT's refined fundamental.
    pub f0: f32,
    /// MAT's inharmonicity coefficient.
    pub b: f32,
    /// Measured partial frequency and amplitude by number (1-indexed).
    pub partials: Vec<Option<(f32, f32)>>,
    /// The operator's declaration, absent outside the isolation set.
    pub sounding: Option<Sounding>,
}

impl Capture {
    /// Whether the cached fundamental is a reading rather than tracker garbage
    /// ([`ET_PLAUSIBLE_CENTS`]).
    pub fn plausible(&self) -> bool {
        plausible(self.key, self.f0)
    }

    /// Measured stiff-string reference set, for driving the strobe bank over
    /// this capture. Returns how many references were written; stops at the
    /// first absent partial, as the bank's own contiguity requires.
    pub fn refs(&self, out: &mut [f32; MAX_STROBE_REFS]) -> usize {
        let mut count = 0;
        for n in 1..=MAX_STROBE_REFS {
            match self.partials.get(n).copied().flatten() {
                Some((f, _)) if f > 0.0 && f < tuner_core::audio::SAMPLE_RATE as f32 / 2.0 => {
                    out[n - 1] = f;
                    count = n;
                }
                _ => break,
            }
        }
        count
    }

    /// This capture's raw audio, read from its dump directory.
    pub fn audio(&self, root: &Path) -> Option<Vec<f32>> {
        raw::stable(&root.join(&self.dir))
    }
}

/// Loads a regenerated-partials dump, dropping implausible entries per
/// [`Capture::plausible`]. Returns the kept captures and how many were dropped.
pub fn load(path: &Path) -> (Vec<Capture>, usize) {
    let rows = rows(path);
    let mut out = Vec::with_capacity(rows.len());
    let mut dropped = 0;
    for r in rows {
        let key = r["key_index"].as_u64().unwrap_or(0) as u8;
        let mut partials = vec![None; MAX_STROBE_REFS + 1];
        if let Some(arr) = r["partials"].as_array() {
            for p in arr {
                let (Some(n), Some(f)) = (p["number"].as_u64(), p["frequency"].as_f64()) else {
                    continue;
                };
                if (n as usize) <= MAX_STROBE_REFS {
                    let amp = p["amplitude"].as_f64().unwrap_or(0.0) as f32;
                    partials[n as usize] = Some((f as f32, amp));
                }
            }
        }
        let sounding = r["sounding_strings"].as_object().map(|s| {
            let arr = s["sounding"].as_array().cloned().unwrap_or_default();
            let mut sounding = [false; 3];
            for (i, v) in arr.iter().take(3).enumerate() {
                sounding[i] = v.as_bool().unwrap_or(false);
            }
            Sounding {
                on_key: s["on_key"].as_u64().unwrap_or(3) as u8,
                sounding,
            }
        });
        let cap = Capture {
            key,
            dir: r["source_dir"].as_str().unwrap_or_default().to_string(),
            f0: r["mat_f0"].as_f64().unwrap_or(0.0) as f32,
            b: r["calculated_b"].as_f64().unwrap_or(0.0) as f32,
            partials,
            sounding,
        };
        if cap.plausible() {
            out.push(cap);
        } else {
            dropped += 1;
        }
    }
    (out, dropped)
}

/// The profile a regenerated-partials dump describes, as the app would hold it:
/// every plausible entry recorded in the dump's order as a named capture of its
/// key, with its declaration, so [`InharmonicityProfile::record`]'s retention
/// applies and a solo stays untrusted. Returns the profile and how many entries
/// the plausibility rule dropped.
pub fn profile(path: &Path) -> (InharmonicityProfile, usize) {
    let mut profile = InharmonicityProfile::default();
    let mut dropped = 0;
    for r in rows(path) {
        let key = r["key_index"].as_u64().unwrap_or(0) as u8;
        if !plausible(key, r["mat_f0"].as_f64().unwrap_or(0.0) as f32) {
            dropped += 1;
            continue;
        }
        let partials = r["partials"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| {
                Some(Partial {
                    number: p["number"].as_u64()? as u32,
                    frequency: p["frequency"].as_f64()? as f32,
                    amplitude: p["amplitude"].as_f64().unwrap_or(0.0) as f32,
                })
            })
            .collect();
        let dir = r["source_dir"].as_str().unwrap_or_default();
        profile.record(KeyMeasurement {
            key_index: key,
            measured_f0: r["measured_f0"].as_f64().unwrap_or(0.0) as f32,
            partials,
            calculated_b: r["calculated_b"].as_f64().map(|b| b as f32),
            // The directory's epoch suffix: the identity the app writes.
            last_captured: dir.rsplit('_').next().unwrap_or(dir).to_string(),
            captured_in_auto: false,
            sounding_strings: serde_json::from_value(r["sounding_strings"].clone())
                .ok()
                .flatten(),
        });
    }
    (profile, dropped)
}
