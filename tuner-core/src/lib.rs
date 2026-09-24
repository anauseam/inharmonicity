//! # tuner-core
//!
//! Headless audio analysis for a piano tuner: signal validation, pitch detection
//! and inharmonicity measurement, with no frontend dependency.
//!
//! The entry point is [`pipeline::AudioPipeline`], which runs the DSP and gives a
//! frontend its [`pipeline::PipelineHandle`].

pub mod algorithms;
pub mod audio;
pub mod cola;
pub mod engine;
pub mod gatekeeper;
pub mod models;
pub mod pipeline;
pub mod strobe;
pub mod synth;
pub mod worker;

/// Per-hop output of the DSP thread, published every hop through a lossy
/// `triple_buffer`, so a reader sees only the freshest frame. Fixed-size, so
/// publishing never allocates.
#[derive(Clone)]
pub struct FrameOutput {
    /// Linear magnitude spectrum of the hop, sized for the larger (bass) window.
    pub magnitudes: [f32; audio::BASS_WINDOW_SIZE / 2],
    /// Number of valid bins in `magnitudes` (1024 for treble, 4096 for bass).
    pub magnitude_len: usize,
    /// Smoothed RMS amplitude, an exponential moving average.
    pub rms_ema: f32,
    /// Normalised half-wave rectified spectral flux.
    pub nhwrsf: f32,
    /// Sustain-stability metric (the gate's inverse participation ratio).
    pub sustain_stability: f32,
    /// The Gatekeeper reads the hop as silence.
    pub is_silence: bool,
    /// 88-key piano index (0 = A0, 87 = C8), if a note is currently locked.
    pub note_index: Option<u8>,
    /// Detected fundamental frequency in Hz, if a note is currently locked.
    pub detected_frequency: Option<f32>,
    /// Partial frequencies the engine is tracking this hop.
    /// Valid entries: `[0..tracked_count]`.
    pub tracked_freqs: [f32; 12],
    /// Harmonic index (n) for each partial. Parallel to `tracked_freqs`.
    pub tracked_ns: [u32; 12],
    /// Number of valid entries in `tracked_freqs` / `tracked_ns`.
    pub tracked_count: usize,
    /// Strobe-bank accumulated beat phase per reference (cycles, [0, 1)), at
    /// index partial n − 1 of the strobed key. Accumulated DSP-side, so a dropped
    /// frame cannot corrupt the count. Valid entries: `[0..strobe_count]`.
    pub strobe_angle: [f32; 12],
    /// Per-reference amplitude gate (`true` = below floor, angle held).
    pub strobe_gated: [bool; 12],
    /// Per-reference beat rate `f_live − f_ref` (Hz) against
    /// `StrobeRefUpdate::refs[i]`, a least-squares fit of the accumulated phase
    /// over [`strobe::band_slope::BAND_SLOPE_WINDOW_SECS`]. It aliases past half
    /// the hop rate. `None` while the fit fills, or after a re-strike restarts it.
    pub strobe_beat_hz: [Option<f32>; 12],
    /// Number of valid strobe references (0 = no key being strobed).
    pub strobe_count: usize,
    /// Per-reference Goertzel amplitude in the time signal's units: the quantity
    /// `strobe_gated` thresholds.
    pub strobe_amplitude: [f32; 12],
    /// Each reference partial's individual strings, resolved as separate
    /// spectral lines by [`strobe::unison`]: signed Hz offsets from
    /// `StrobeRefUpdate::refs[i]` with a relative amplitude, strongest first.
    /// Valid entries: `[0..unison_line_count[i]]`. Held while a reference is
    /// gated; emptied on `is_silence`.
    pub unison_lines: [[models::UnisonLine; algorithms::peaks::MAX_UNISON_LINES]; 12],
    /// Valid entries of `unison_lines` per reference.
    pub unison_line_count: [u8; 12],
    /// `2/T` per reference (Hz): the smallest split its current record can
    /// separate, so lines shown without it overstate what was resolved. `0.0`
    /// while nothing is published.
    pub unison_resolution_hz: [f32; 12],
    /// Whether the resolved lines are a unison or one partial splitting against
    /// itself: one verdict for the bank, since the test is that a split is
    /// constant across partials.
    pub unison_verdict: strobe::unison::UnisonVerdict,
    /// Where the capture lifecycle stands. The pipeline owns it; a consumer asks
    /// for a transition with a [`pipeline::CaptureCommand`].
    pub capture_state: pipeline::CaptureState,
    /// Samples written to the capture in progress; `0` when none is recording.
    pub capture_progress_samples: usize,
    /// The measured frequency (Hz) of the strobed key's coarse partial, read off
    /// the magnitude spectrum at the partial the [`strobe::StrobeRefUpdate`]
    /// nominated. It does not alias and needs no note lock. `None` without a
    /// reference, in silence, or when nothing clears the local noise.
    pub coarse_hz: Option<f32>,
}

impl Default for FrameOutput {
    fn default() -> Self {
        Self {
            magnitudes: [0.0; audio::BASS_WINDOW_SIZE / 2],
            magnitude_len: 0,
            rms_ema: 0.0,
            nhwrsf: 0.0,
            sustain_stability: 0.0,
            is_silence: true,
            note_index: None,
            detected_frequency: None,
            tracked_freqs: [0.0; 12],
            tracked_ns: [0; 12],
            tracked_count: 0,
            strobe_angle: [0.0; 12],
            strobe_gated: [true; 12],
            strobe_beat_hz: [None; 12],
            strobe_count: 0,
            strobe_amplitude: [0.0; 12],
            unison_lines: [[models::UnisonLine {
                offset_hz: 0.0,
                relative_amplitude: 0.0,
            }; algorithms::peaks::MAX_UNISON_LINES]; 12],
            unison_line_count: [0; 12],
            unison_resolution_hz: [0.0; 12],
            unison_verdict: strobe::unison::UnisonVerdict::Undetermined,
            capture_state: pipeline::CaptureState::Idle,
            capture_progress_samples: 0,
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
            .field("tracked_count", &self.tracked_count)
            .field("capture_state", &self.capture_state)
            .field("coarse_hz", &self.coarse_hz)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    mod audio_tests;
    mod peaks_tests;
}
