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
//! | [`audio`] | CPAL audio capture, stream management, standalone host extension |
//! | [`pipeline`] | AudioPipeline mediator, shared state types, memory pools |
//! | [`engine`] | F0 Engine — 3-Stage Matched Filter pitch detection |
//! | [`gatekeeper`] | 5-state signal validator (pure DSP, no shared state) |
//! | [`synth`] | Offline additive resynthesis of a tuning curve to audio (cold-path) |
//! | [`worker`] | Background worker for heavy offline DSP |

/// Stateless DSP building blocks: spectral transforms, pitch detection, signal metrics, and tuning math.
pub mod algorithms;
/// CPAL audio capture, device selection, real-time streaming, and standalone host extension.
pub mod audio;
/// Circular FIFO overlapping analysis sliding window.
pub mod cola;
/// F0 Engine — 3-Stage Matched Filter pitch detection.
pub mod engine;
/// 5-state signal validator (pure DSP). Evaluates RMS, CSD, and NINOS2 for stability gating.
pub mod gatekeeper;
/// Domain data types, lookup tables, and serializable structures.
pub mod models;
/// AudioPipeline mediator: orchestrates DSP components, owns shared state, memory pools.
pub mod pipeline;
/// Offline additive resynthesis of a [`models::TuningCurve`] to audio samples
/// (cold-path, thread-free — no audio stream; the caller plays or saves).
pub mod synth;
/// Background worker manager for heavy offline DSP (MAT, Beta calculation).
pub mod worker;

/// Continuous per-hop visualization payload sent from the DSP thread to the
/// GUI thread via a `triple_buffer` (lossy — GUI reads only the freshest frame).
///
/// Written every DSP hop regardless of note detection state, ensuring the
/// spectrogram always renders live data.
///
/// Fixed-size — zero heap allocations. The `magnitudes` array is sized for
/// the bass engine (4096 bins) to future-proof for bass spectrogram display;
/// `magnitude_len` indicates how many bins are actually valid.
#[derive(Clone)]
pub struct FrameOutput {
    /// Linear magnitude spectrum for spectrogram visualization.
    /// Sized to `BASS_WINDOW_SIZE / 2` (4096) to accommodate both treble (1024)
    /// and bass (4096) paths.
    pub magnitudes: [f32; audio::BASS_WINDOW_SIZE / 2],
    /// Number of valid bins in `magnitudes` (1024 for treble, 4096 for bass).
    pub magnitude_len: usize,
    /// Current smoothed RMS amplitude (Exponential Moving Average).
    pub rms_ema: f32,
    /// Current Normalised Half-Wave Rectified Spectral Flux.
    pub nhwrsf: f32,
    /// Current NINOS2 stability metric.
    pub ninos2: f32,
    /// Indicates whether the Gatekeeper evaluates the current signal as absolute silence.
    pub is_silence: bool,
    /// 88-key piano index (0 = A0, 87 = C8), if a note is currently locked.
    pub note_index: Option<u8>,
    /// Detected fundamental frequency in Hz, if a note is currently locked.
    pub detected_frequency: Option<f32>,
    /// Detection confidence (0.0–1.0). Currently unused by MAT, returning None.
    pub confidence: Option<f32>,
    /// Deviation from nearest equal-temperament note in cents (positive = sharp).
    pub cents_deviation: Option<f32>,
    /// Real-time partial frequencies for multi-ring strobe visualization.
    /// Valid entries: `[0..partial_count]`.
    pub partial_freqs: [f32; 12],
    /// Harmonic index (n) for each partial. Parallel to `partial_freqs`.
    pub partial_ns: [u32; 12],
    /// Number of valid entries in `partial_freqs` / `partial_ns`.
    pub partial_count: usize,
}

impl Default for FrameOutput {
    fn default() -> Self {
        Self {
            magnitudes: [0.0; audio::BASS_WINDOW_SIZE / 2],
            magnitude_len: 0,
            rms_ema: 0.0,
            nhwrsf: 0.0,
            ninos2: 0.0,
            is_silence: true,
            note_index: None,
            detected_frequency: None,
            confidence: None,
            cents_deviation: None,
            partial_freqs: [0.0; 12],
            partial_ns: [0; 12],
            partial_count: 0,
        }
    }
}

impl std::fmt::Debug for FrameOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameOutput")
            .field("magnitude_len", &self.magnitude_len)
            .field("rms_ema", &self.rms_ema)
            .field("is_silence", &self.is_silence)
            .field("note_index", &self.note_index)
            .field("detected_frequency", &self.detected_frequency)
            .field("confidence", &self.confidence)
            .field("cents_deviation", &self.cents_deviation)
            .field("partial_count", &self.partial_count)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    mod audio_tests;
    mod peaks_tests;
}
