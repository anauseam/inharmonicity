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
/// Fixed-reference strobe phase-comparator bank (Path A of the manual-mode
/// strobe): DSP-side beat-phase accumulation against curve targets.
pub mod strobe;
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
    /// Real-time partial frequencies tracked by the engine for visual telemetry.
    /// Valid entries: `[0..tracked_count]`.
    pub tracked_freqs: [f32; 12],
    /// Harmonic index (n) for each partial. Parallel to `tracked_freqs`.
    pub tracked_ns: [u32; 12],
    /// Number of valid entries in `tracked_freqs` / `tracked_ns`.
    pub tracked_count: usize,
    /// Strobe-bank accumulated beat phase per reference (cycles, [0, 1)) —
    /// index = partial n − 1 of the strobed key. Accumulated DSP-side
    /// (strobe design R2) so this lossy buffer cannot corrupt the count.
    /// Valid entries: `[0..strobe_count]`.
    pub strobe_angle: [f32; 12],
    /// Per-reference D3 amplitude gate (`true` = below floor, angle held).
    /// Parallel to `strobe_angle`.
    pub strobe_gated: [bool; 12],
    /// Per-reference beat rate `f_live − f_ref` (Hz) — the band's rotation
    /// *rate*, least-squares-fit DSP-side over
    /// [`strobe::BAND_SLOPE_WINDOW_SECS`]. Parallel to `strobe_angle`, and the
    /// fine half of the readout pair: phase-integrated, so it is far steadier
    /// than a per-hop frequency estimate, but it aliases past ±21.5 Hz where
    /// `coarse_hz` takes over. `None` while the fit is filling or has been
    /// restarted by a re-strike. Like `coarse_hz` it ships in Hz — the
    /// reference it is measured against is `StrobeRefUpdate::refs[i]`.
    pub strobe_beat_hz: [Option<f32>; 12],
    /// Number of valid strobe references (0 = no key being strobed).
    pub strobe_count: usize,
    /// Coarse readout: the measured frequency (Hz) of the strobed key's coarse
    /// partial, read straight off the magnitude spectrum
    /// (`algorithms::peaks::coarse_read`) at the partial the
    /// [`strobe::StrobeRefUpdate`] nominated. `None` when the reference is not
    /// set, the signal is `Silence`, or nothing there clears the local noise.
    ///
    /// The wide-range half of the readout pair: unlike `strobe_angle` it does
    /// not alias past ±21.5 Hz, and unlike `detected_frequency` it needs no note
    /// lock — so it is what remains readable during a pitch raise. Absolute Hz,
    /// not cents: the frontend owns the reference the number is shown against.
    pub coarse_hz: Option<f32>,
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
            tracked_freqs: [0.0; 12],
            tracked_ns: [0; 12],
            tracked_count: 0,
            strobe_angle: [0.0; 12],
            strobe_gated: [true; 12],
            strobe_beat_hz: [None; 12],
            strobe_count: 0,
            coarse_hz: None,
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
            .field("tracked_count", &self.tracked_count)
            .field("coarse_hz", &self.coarse_hz)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    mod audio_tests;
    mod peaks_tests;
}
