//! # Domain Data Models
//!
//! **NOTE**: This file and the data structures within will be highly likely to change
//! after the pipeline structure is finalized and a new communication structure with
//! the GUI is formulated.
//!
//! This module contains pure data structures used throughout the tuner domain,
//! divorced from the mathematical algorithms that operate on them.

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
/// The dual-exponential parametric model is utilized to smoothly interpolate
/// the physical transition from the bass bridge to the treble bridge.
///
/// Implements Equation for B(n):
///   B(n) = exp(-0.066n - 9.211) + exp(0.0926n - 11.788)
///
/// # Reference
/// 1. Rigaud, F., David, B., & Daudet, L. (2013). "A parametric model and estimation techniques
///    for the inharmonicity and tuning of the piano". JASA 133(5), pp. 3107-3118.
///    DOI: 10.1121/1.4802644
pub fn get_expected_beta(key_index: u8) -> f32 {
    // Rigaud model uses a 1-indexed key number (A0 = 1).
    // key_index is 0-indexed (A0 = 0), so we offset by 1.
    let n = key_index as f32 + 1.0;
    (-0.066 * n - 9.211).exp() + (0.0926 * n - 11.788).exp()
}
