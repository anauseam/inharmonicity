//! # Musical Tuning Module
//! 
//! This module provides comprehensive musical tuning calculations for piano tuning applications.
//! It handles note name conversions, frequency calculations, and cent deviation measurements
//! based on equal temperament tuning with planned support for inharmonicity compensation.
//! 
//! ## Features
//! - 88-key piano note mapping (A0 to C8)
//! - Equal temperament frequency calculations
//! - Cent deviation calculations for tuning accuracy
//! - Note name to frequency conversions
//! - Key index to note name mappings
//! - **Future**: Inharmonicity compensation for professional piano tuning
//! 
//! ## Planned Inharmonicity Features
//! - Piano-specific inharmonicity curve calculation
//! - Stretch tuning compensation for different piano sizes
//! - Partial frequency analysis and adjustment
//! - Professional tuning curve generation

use once_cell::sync::Lazy;
use std::collections::BTreeMap;

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
static NOTES: Lazy<Vec<Note>> = Lazy::new(|| {
    const NOTE_NAMES: [&str; 12] = [
        "A", "A#", "B", "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#",
    ];
    let mut notes = Vec::with_capacity(88);

    for i in 0..88 {
        // A4 is the 49th key, which is index 48 in a 0-indexed loop.
        // The formula for frequency in equal temperament is f = f0 * 2^(n/12)
        // Here, f0 is A4 (440Hz) and n is the number of semitones away from A4.
        let frequency = 440.0 * 2.0_f32.powf((i as f32 - 48.0) / 12.0);

        // A piano starts at A0. The note name cycles every 12 keys.
        let note_index = i % 12;
        // The octave changes at C. We can calculate it based on the key index.
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
static NOTE_MAP: Lazy<BTreeMap<String, u8>> = Lazy::new(|| {
    NOTES.iter()
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

/// Calculates the deviation from a target frequency in cents.
///
/// Cents are a logarithmic unit of pitch measurement where:
/// - 100 cents = 1 semitone
/// - 1200 cents = 1 octave
/// - Positive values indicate sharpness, negative values indicate flatness
///
/// # Arguments
/// * `freq` - Measured frequency in Hz
/// * `target_freq` - Target frequency in Hz
///
/// # Returns
/// * Cent deviation (positive = sharp, negative = flat)
pub fn calculate_cents_deviation(freq: f32, target_freq: f32) -> f32 {
    1200.0 * (freq / target_freq).log2()
}

/// Calculates inharmonicity-compensated target frequency for professional piano tuning.
/// 
/// **Note**: This function is planned for future implementation and currently returns
/// the equal temperament frequency. Inharmonicity compensation will account for:
/// - Piano string stiffness and inharmonicity
/// - Stretch tuning for different piano sizes
/// - Partial frequency adjustments
/// - Professional tuning curve generation
/// 
/// # Arguments
/// * `key_index` - Piano key index (0-87)
/// * `piano_type` - Type of piano (grand, upright, etc.) - future parameter
/// 
/// # Returns
/// * Target frequency with inharmonicity compensation (currently equal temperament)
/// 
/// # Future Implementation
/// This function will implement the inharmonicity calculations described in:
/// - Young's inharmonicity model
/// - Piano-specific stretch tuning curves
/// - Partial frequency analysis and compensation
pub fn calculate_inharmonicity_compensated_frequency(
    key_index: u8,
    _piano_type: &str, // Reserved for future piano type parameter
) -> f32 {
    // TODO: Implement inharmonicity compensation
    // For now, return equal temperament frequency
    let (_, freq) = find_nearest_note_by_index(key_index);
    freq
}
