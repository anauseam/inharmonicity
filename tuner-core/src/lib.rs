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
//! | [`engine`] | F0 Engine — Scout / Bass / Treble DSP (wireframe) |
//! | [`gatekeeper`] | 5-state signal validator (pure DSP, no shared state) |
//! | [`worker`] | Background worker for heavy offline DSP (wireframe) |
//! | [`capture_processing`] | Legacy frame processing (deprecated) |

/// Stateless DSP building blocks: spectral transforms, pitch detection, signal metrics, and tuning math.
pub mod algorithms;
/// CPAL audio capture, device selection, real-time streaming, and standalone host extension.
pub mod audio;
/// Noise-floor and transient calibration routines (uses [`audio::AudioSource`]).
pub mod calibration;
/// Legacy capture frame processing (deprecated — to be replaced by `pipeline` + `worker`).
pub mod capture_processing;
/// Circular FIFO overlapping analysis sliding window.
pub mod cola;
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
    /// 88-key piano index (0 = A0, 87 = C8), if a note is currently locked.
    pub note_index: Option<u8>,
    /// Detected fundamental frequency in Hz, if a note is currently locked.
    pub detected_frequency: Option<f32>,
    /// Detection confidence (0.0–1.0), if a note is currently locked.
    pub confidence: Option<f32>,
    /// Deviation from nearest equal-temperament note in cents (positive = sharp), if a note is currently locked.
    pub cents_deviation: Option<f32>,
}

impl Default for FrameOutput {
    fn default() -> Self {
        Self {
            magnitudes: [0.0; audio::BASS_WINDOW_SIZE / 2],
            magnitude_len: 0,
            rms_ema: 0.0,
            nhwrsf: 0.0,
            note_index: None,
            detected_frequency: None,
            confidence: None,
            cents_deviation: None,
        }
    }
}

impl std::fmt::Debug for FrameOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameOutput")
            .field("magnitude_len", &self.magnitude_len)
            .field("rms_ema", &self.rms_ema)
            .field("nhwrsf", &self.nhwrsf)
            .field("note_index", &self.note_index)
            .field("detected_frequency", &self.detected_frequency)
            .field("confidence", &self.confidence)
            .field("cents_deviation", &self.cents_deviation)
            .finish()
    }
}

// ─── Deprecated Compatibility Shim ───────────────────────────────────────────

/// Temporary compatibility type used only by the deprecated [`capture_processing`] module.
///
/// This struct exists solely to keep `capture_processing.rs` compiling until it
/// is replaced by the Worker pipeline. The GUI (`app.rs`) constructs instances
/// by repackaging [`NoteEvent`] data on the UI thread.
///
/// **Do not use this type for new code.** Use [`FrameOutput`] instead.
// TODO: Remove when capture_processing.rs is replaced by Worker pipeline.
#[deprecated(note = "Use FrameOutput instead. Remove with capture_processing.rs.")]
#[derive(Debug, Clone)]
pub struct AnalysisResult {
    /// Detected frequency in Hz, or `None` if silence/noise.
    pub detected_frequency: Option<f32>,
    /// Detection confidence (0.0–1.0).
    pub confidence: Option<f32>,
    /// Cents deviation from nearest ET note.
    pub cents_deviation: Option<f32>,
    /// Note name string (e.g., "A4"). Reconstructed on the UI thread from note index.
    pub note_name: Option<String>,
}

#[cfg(test)]
mod tests {
    mod audio_tests;
}
