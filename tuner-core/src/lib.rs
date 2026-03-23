// tuner-core/src/lib.rs

//! # tuner-core — Headless Audio Processing & Analysis
//!
//! This crate contains all audio processing, pitch detection, signal validation,
//! and inharmonicity calculations for the piano tuner. It is **completely headless**
//! and contains no GUI code, making it consumable by any frontend (Iced, egui, WASM, etc.).
//!
//! ## Entry Point
//!
//! The primary public API is [`pipeline::AudioPipeline`], which orchestrates all DSP
//! components and provides a [`pipeline::PipelineHandle`] for the frontend to poll
//! shared state. See the `pipeline` module for the Split / Handle pattern.
//!
//! ## Modules
//!
//! | Module | Purpose |
//! |---|---|
//! | [`algorithms`] | Stateless DSP building blocks (spectral, pitch, metrics, tuning) |
//! | [`models`] | Domain data types, lookup tables, and serializable structures |
//! | [`audio`] | CPAL audio capture and stream management |
//! | [`pipeline`] | AudioPipeline mediator, shared state types, memory pools |
//! | [`engine`] | F0 Engine — Scout / Bass / Treble DSP (wireframe) |
//! | [`gatekeeper`] | 5-state signal validator (pure DSP, no shared state) |
//! | [`worker`] | Background worker for heavy offline DSP (wireframe) |
//! | [`capture_processing`] | Legacy frame processing (deprecated) |

/// Stateless DSP building blocks: spectral transforms, pitch detection, signal metrics, and tuning math.
pub mod algorithms;
/// CPAL audio capture, device selection, and real-time streaming.
pub mod audio;
/// Standalone noise-floor calibration (opens its own temporary CPAL stream).
pub mod calibration;
/// Legacy capture frame processing (deprecated — to be replaced by `pipeline` + `worker`).
pub mod capture_processing;
/// F0 Engine — Scout, Bass, and Treble frequency detection (wireframe).
pub mod engine;
/// 5-state signal validator (pure DSP). Evaluates RMS, CSD, and NINOS2 for stability gating.
pub mod gatekeeper;
/// Domain data types, lookup tables, and serializable structures.
pub mod models;
/// AudioPipeline mediator: orchestrates DSP components, owns shared state, memory pools.
pub mod pipeline;
/// Background worker manager for heavy offline DSP (MAT / ICF).
pub mod worker;

/// Represents the result of a single audio analysis frame.
///
/// This struct is the primary output of the real-time analysis loop.
/// It is sent from the audio processing thread to the GUI thread
/// via a crossbeam channel for display and further processing.
#[derive(Debug, Clone)]
pub struct AnalysisResult {
    /// The primary detected frequency in Hz, or `None` if silence / noise.
    pub detected_frequency: Option<f32>,
    /// Confidence of the detection (0.0–1.0), derived from the YIN/pYIN clarity metric.
    pub confidence: Option<f32>,
    /// Deviation from the nearest equal-temperament note in cents (positive = sharp).
    pub cents_deviation: Option<f32>,
    /// Name of the nearest note (e.g., `"A4"`, `"C#3"`).
    pub note_name: Option<String>,
    /// Magnitude spectrum for the spectrogram visualization (first `BUFFER_SIZE / 2` bins).
    pub spectrogram_data: Vec<f32>,
    /// Frequencies of detected harmonic partials (2nd, 3rd, … overtones).
    pub partials: Vec<f32>,
}

#[cfg(test)]
mod tests {
    mod audio_tests;
}
