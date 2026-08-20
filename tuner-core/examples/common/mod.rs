//! # Shared capture loading for the offline harnesses
//!
//! One path for the things every harness needs: finding capture directories,
//! reading raw `f32` audio, parsing what `regenerate_partials` emits, and
//! applying the consumption rules `docs/internals/06-capture-sets.md` states as
//! binding. Before this existed, `analysis.json` was parsed in nine harnesses,
//! capture discovery written six times, and the piano-2 ±200 ¢ rule spelled out
//! in eight — and **missing from others that read the same data**, which is how
//! a rule the docs call binding stops being enforced.
//!
//! Harness code only; nothing here ships in the crate. A `tuner-core` module
//! would be an architecture change (`CLAUDE.md`), and this is plumbing.

// Each example compiles its own copy of this module and uses a subset of it, so
// anything the *including* example does not call reads as dead. The alternative
// is per-item gating that changes every time a harness is added.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use tuner_core::strobe::MAX_STROBE_REFS;

/// Cents from equal temperament past which a cached measurement is treated as
/// tracker garbage rather than a reading.
///
/// `06`: piano #2's deep-bass entries predate the `worker::MAT_SEED_TOLERANCE`
/// fix and carry rumble-seeded fundamentals — A0 "measured" at 7–14.7 Hz
/// against an ET 27.5. The audio is genuine; only the cached analysis is wrong.
/// Applied through [`Capture::plausible`] so a caller cannot forget it.
pub const ET_PLAUSIBLE_CENTS: f32 = 200.0;

/// A0, the compass origin — key index 0.
const A0_HZ: f32 = 27.5;

/// Equal-tempered frequency of a key index.
pub fn et_hz(key: u8) -> f32 {
    A0_HZ * 2.0f32.powf(key as f32 / 12.0)
}

/// Which of a key's strings sounded, as `regenerate_partials` passes it through
/// from `analysis.json`'s `metadata.sounding_strings`.
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

    /// Index of the sounding string for a solo, else `None`.
    pub fn string_index(&self) -> Option<usize> {
        self.is_solo()
            .then(|| self.sounding.iter().position(|s| *s))?
    }
}

/// One capture as `regenerate_partials` emits it, plus its audio on demand.
#[derive(Debug, Clone)]
pub struct Capture {
    pub key: u8,
    /// Dump directory name — the identity a repeat set needs.
    pub dir: String,
    /// MAT's refined fundamental.
    pub f0: f32,
    /// MAT's inharmonicity coefficient.
    pub b: f32,
    /// The Rigaud prior for this key, for plausibility work.
    pub prior_b: f32,
    pub partial_count: usize,
    /// Measured partial frequency and amplitude by number (1-indexed).
    pub partials: Vec<Option<(f32, f32)>>,
    /// The operator's declaration, absent outside the isolation set.
    pub sounding: Option<Sounding>,
}

impl Capture {
    /// Whether the cached fundamental is a reading rather than tracker garbage
    /// ([`ET_PLAUSIBLE_CENTS`]).
    pub fn plausible(&self) -> bool {
        let et = et_hz(self.key);
        self.f0 > 0.0 && (1200.0 * (self.f0 / et).log2()).abs() <= ET_PLAUSIBLE_CENTS
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
        read_raw_f32(&root.join(&self.dir).join("audio.raw"))
    }
}

/// Reads a little-endian `f32` mono dump.
pub fn read_raw_f32(path: &Path) -> Option<Vec<f32>> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() % 4 != 0 {
        return None;
    }
    let mut out = vec![0.0f32; bytes.len() / 4];
    // SAFETY: `out` holds exactly `bytes.len()` bytes of `f32`, and `f32` has
    // no invalid bit patterns — every 4-byte group is a valid value.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.as_mut_ptr() as *mut u8, bytes.len());
    }
    Some(out)
}

/// Every `key_*` capture directory under `root`, sorted.
pub fn capture_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<_> = std::fs::read_dir(root)
        .unwrap_or_else(|e| panic!("read {}: {e}", root.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("key_"))
        })
        .collect();
    dirs.sort();
    dirs
}

/// Loads a `regenerate_partials` dump, **dropping implausible entries** per
/// [`Capture::plausible`]. Returns the kept captures and how many were dropped,
/// so a caller reports the exclusion rather than hiding it.
pub fn load_regen(path: &Path) -> (Vec<Capture>, usize) {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));

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
            prior_b: r["prior_b"].as_f64().unwrap_or(0.0) as f32,
            partial_count: r["partial_count"].as_u64().unwrap_or(0) as usize,
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

/// Register label for per-band reporting, matching `strobe_replay`'s bands so
/// figures stay comparable across harnesses.
pub fn register(key: u8) -> &'static str {
    match key {
        0..=27 => "bass",
        28..=51 => "tenor",
        52..=75 => "treble",
        _ => "high 76–87",
    }
}

/// Median of a slice, by value. Empty input yields `f32::NAN`.
pub fn median(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return f32::NAN;
    }
    v.sort_by(f32::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
}

/// What one reference resolved at the best record it reached.
#[derive(Debug, Clone, Copy, Default)]
pub struct Resolved {
    pub count: u8,
    pub resolution_hz: f32,
    pub lines: [tuner_core::models::UnisonLine; tuner_core::algorithms::peaks::MAX_UNISON_LINES],
    /// Record length in hops at that point.
    pub record: usize,
}

/// Drives the **shipped** bank over `audio` and returns, per reference, the best
/// record it reached — the hop with the longest unbroken record, which is what
/// the panel would be showing when the tuner looks at it.
///
/// `strobe_replay` carries its own copy of this; the two must stay behaviourally
/// identical until Prompt AF retires that one under its byte-diff protocol.
pub fn run_unison(
    audio: &[f32],
    refs: &[f32; MAX_STROBE_REFS],
    count: usize,
    spacing_hz: f32,
    noise_floor: f32,
) -> (Vec<Resolved>, tuner_core::strobe::unison::UnisonVerdict) {
    use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_RATE_HZ, HOP_SIZE, SAMPLE_RATE};
    use tuner_core::strobe::{Strobe, StrobeRefUpdate};

    let mut strobe = Strobe::new(SAMPLE_RATE);
    strobe.retarget(StrobeRefUpdate {
        count,
        refs: *refs,
        coarse_index: 0,
        spacing_hz,
    });

    let hops = audio.len().saturating_sub(BASS_WINDOW_SIZE) / HOP_SIZE;
    let mut best = vec![Resolved::default(); count];
    let mut verdict = tuner_core::strobe::unison::UnisonVerdict::Undetermined;
    let mut frame = tuner_core::pipeline::ProcessingFrame::new();
    for h in 0..hops {
        frame.audio_buffer[..BASS_WINDOW_SIZE]
            .copy_from_slice(&audio[h * HOP_SIZE..h * HOP_SIZE + BASS_WINDOW_SIZE]);
        let out = strobe.process(&frame, noise_floor, false);
        for (i, slot) in best.iter_mut().enumerate().take(count) {
            // The published resolution is 2·f_hop/L, so it *is* the record length.
            let record = if out.line_resolution_hz[i] > 0.0 {
                (2.0 * HOP_RATE_HZ / out.line_resolution_hz[i]).round() as usize
            } else {
                0
            };
            if record > slot.record {
                *slot = Resolved {
                    count: out.line_count[i],
                    resolution_hz: out.line_resolution_hz[i],
                    lines: out.lines[i],
                    record,
                };
                verdict = out.verdict;
            }
        }
    }
    (best, verdict)
}
