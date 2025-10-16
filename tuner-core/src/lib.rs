// tuner-core/src/lib.rs

//! The core logic for the professional piano tuner.
//! This crate is responsible for audio processing, pitch detection,
//! and inharmonicity calculations. It is completely headless
//! and contains no GUI code.

pub mod audio;
pub mod fft;
pub mod pitch;
pub mod tuning;

/// Represents the result of a single audio analysis frame.
// This derive is necessary for the struct to be used in the `CustomEvent` enum.
#[derive(Debug, Clone)]
pub struct AnalysisResult {
    /// The primary detected frequency in Hz.
    pub detected_frequency: Option<f32>,
    /// The confidence of the detected frequency (0.0 to 1.0).
    pub confidence: Option<f32>,
    /// The deviation from the target note in cents.
    pub cents_deviation: Option<f32>,
    /// The name of the nearest note.
    pub note_name: Option<String>,
    /// Data for the spectrogram visualization.
    pub spectrogram_data: Vec<f32>,
    /// Frequencies of the detected partials.
    pub partials: Vec<f32>,
}