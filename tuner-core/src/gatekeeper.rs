//! # Gatekeeper — the signal validator
//!
//! Reads each [`ProcessingFrame`] and reports one of three [`SignalState`]s,
//! from three metrics: the RMS envelope (EMA-smoothed), the spectral flux
//! (NHWRSF) and the sustain stability (an EMA of the inverse participation
//! ratio).
//!
//! | State | Entered when | Left when |
//! |---|---|---|
//! | [`Silence`](SignalState::Silence) | The RMS EMA is under `silence_threshold`, except on the hop after an onset. The flux, the stability and the stable-frame run reset. | The first hop at or over the threshold. |
//! | [`Unstable`](SignalState::Unstable) | An onset hop, or one whose sustain stability is at or under its threshold. | `required_stable_frames` consecutive hops over the stability threshold, none of them an onset. |
//! | [`Stable`](SignalState::Stable) | The last hop of that run. | An onset, a hop at or under the stability threshold, or silence. |
//!
//! An onset is a per-hop event, not a state: every hop whose flux clears
//! `nhwrsf_threshold` reports one, so a strike usually reports two in a row. It
//! makes its hop `Unstable`, restarts the run, and holds the next hop out of
//! `Silence` so a weak strike's dip cannot end the note.

use crate::algorithms::metrics::{ema, inverse_participation_ratio, nhwrsf, rms};
use crate::audio::{SAMPLE_RATE, WINDOW_SIZE};
use crate::pipeline::ProcessingFrame;

/// The Gatekeeper's thresholds and smoothing factors.
#[derive(Debug, Clone)]
pub struct GatekeeperConfig {
    /// Minimum RMS amplitude required to leave `Silence`. Supplied by the host;
    /// the Gatekeeper does not measure it.
    pub silence_threshold: f32,
    /// Release factor of the RMS EMA (attack is instant): 0 holds, 1 follows at once.
    pub rms_ema_alpha: f32,
    /// NHWRSF above which a frame is an onset.
    pub nhwrsf_threshold: f32,
    /// Sustain stability above which the signal counts as harmonically stable.
    pub sustain_stability_threshold: f32,
    /// Smoothing factor for the sustain-stability EMA.
    pub sustain_ema_alpha: f32,
    /// Consecutive frames over the stability threshold before the signal is `Stable`.
    pub required_stable_frames: usize,
}

impl Default for GatekeeperConfig {
    fn default() -> Self {
        // A frame is one hop, 1024 samples ≈ 23.2 ms: the 2048-sample windows
        // overlap, so frames convert to time at the hop rate.
        Self {
            silence_threshold: 0.005, // Until the host supplies its calibrated threshold
            rms_ema_alpha: 0.1, // Strong smoothing to ride through momentary unison beating dips
            nhwrsf_threshold: 0.5, // A starting point; the profile carries the calibrated value
            sustain_stability_threshold: 10.0, // On a scale from 1 (white noise) to N (a pure tone)
            // Rides a one-frame phase-cancellation dropout during unison beating,
            // yet still clears the threshold within a frame on treble decay.
            sustain_ema_alpha: 0.5,
            required_stable_frames: 4, // ≈ 93 ms
        }
    }
}

/// The Gatekeeper's evaluation of one frame of the stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalState {
    /// Sustain stability has held over its threshold for
    /// `required_stable_frames` consecutive frames.
    Stable,
    /// Audio above the silence threshold that has not held stable: a strike,
    /// noise, or a sound without steady partials.
    Unstable,
    /// The smoothed RMS is below `silence_threshold`. Nothing beyond the RMS is
    /// computed.
    Silence,
}

/// One frame's evaluation, with the metrics it was made from.
#[derive(Debug, Clone, Copy)]
pub struct GateResult {
    pub rms_ema: f32,
    pub nhwrsf: f32,
    pub sustain_stability_ema: f32,
    pub sustain_stability_raw: f32,
    pub state: SignalState,
    pub is_new_onset: bool,
    pub is_transient_bypass: bool,
}

/// The signal validator the [module docs](crate::gatekeeper) describe.
pub struct Gatekeeper {
    pub config: GatekeeperConfig,
    pub(crate) current_state: SignalState,
    /// The previous frame's spectrum, sized to `ProcessingFrame::frequency_buffer`
    /// so the hot path never allocates.
    prev_spectrum: Box<[f32]>,
    pub(crate) current_nhwrsf: f32,
    pub(crate) current_sustain_ema: f32,
    pub(crate) current_sustain_raw: f32,
    stable_counter: usize,
    transient_active: bool,
    pub(crate) current_rms_ema: f32,
    pub(crate) is_new_onset: bool,
    pub(crate) is_transient_bypass: bool,
}

impl Default for Gatekeeper {
    fn default() -> Self {
        Self::new()
    }
}

impl Gatekeeper {
    /// A Gatekeeper in [`SignalState::Silence`], its state zeroed.
    pub fn new() -> Self {
        Self {
            config: GatekeeperConfig::default(),
            current_state: SignalState::Silence,
            prev_spectrum: vec![0.0; 2048].into_boxed_slice(),
            current_nhwrsf: 0.0,
            current_sustain_ema: 0.0,
            current_sustain_raw: 0.0,
            stable_counter: 0,
            current_rms_ema: 0.0,
            is_new_onset: false,
            is_transient_bypass: false,
            transient_active: false,
        }
    }

    /// Evaluates one frame through the states.
    pub fn process_frame(&mut self, frame: &ProcessingFrame) -> GateResult {
        // The RMS envelope, on the newest WINDOW_SIZE samples only, so it follows the
        // attack promptly.
        let rms = rms(&frame.audio_buffer[frame.audio_buffer.len() - WINDOW_SIZE..]);

        let alpha = if rms > self.current_rms_ema {
            1.0 // Instant attack: track volume surges immediately
        } else {
            self.config.rms_ema_alpha // Slow release: ride over unison beating dips
        };
        self.current_rms_ema = ema(rms, self.current_rms_ema, alpha);
        let smoothed_rms = self.current_rms_ema;

        // Never silence mid-transient: a weak note's RMS can dip below the
        // threshold during the strike itself.
        if smoothed_rms < self.config.silence_threshold && !self.transient_active {
            self.current_state = SignalState::Silence;
            self.is_new_onset = false;
            self.is_transient_bypass = false;
            self.current_sustain_ema = 0.0;
            self.current_sustain_raw = 0.0;
            self.current_nhwrsf = 0.0;
            self.reset_note_state();
            return self.build_result();
        }

        let current_spectrum = &frame.frequency_buffer[..];

        let raw_sustain_stability = inverse_participation_ratio(current_spectrum);
        self.current_sustain_raw = raw_sustain_stability;
        self.current_sustain_ema = ema(
            raw_sustain_stability,
            self.current_sustain_ema,
            self.config.sustain_ema_alpha,
        );
        self.current_nhwrsf = nhwrsf(
            current_spectrum,
            &mut self.prev_spectrum[..],
            WINDOW_SIZE,
            SAMPLE_RATE,
        );

        // Onset
        self.is_transient_bypass = self.process_transient_detection();
        if self.is_transient_bypass {
            return self.build_result();
        }

        // Stability
        self.process_stability();

        self.build_result()
    }

    #[inline]
    fn build_result(&self) -> GateResult {
        GateResult {
            rms_ema: self.current_rms_ema,
            nhwrsf: self.current_nhwrsf,
            sustain_stability_ema: self.current_sustain_ema,
            sustain_stability_raw: self.current_sustain_raw,
            state: self.current_state,
            is_new_onset: self.is_new_onset,
            is_transient_bypass: self.is_transient_bypass,
        }
    }

    /// A frame over `nhwrsf_threshold` is an onset and arms
    /// `transient_active`; the next frame at or below it disarms it. Returns
    /// whether this frame is an onset.
    fn process_transient_detection(&mut self) -> bool {
        self.is_new_onset = false;

        // A new strike is a new note, even before the RMS has fallen to silence.
        if self.current_nhwrsf > self.config.nhwrsf_threshold {
            self.stable_counter = 0;
            self.current_state = SignalState::Unstable;
            self.is_new_onset = true;
            self.reset_note_state();
            self.transient_active = true; // armed after the reset, which clears it
            return true;
        }

        if self.transient_active {
            self.transient_active = false;
        }

        false
    }

    /// [`Stable`](SignalState::Stable) once sustain stability has held
    /// over its threshold for `required_stable_frames` consecutive frames.
    fn process_stability(&mut self) {
        if self.current_sustain_ema > self.config.sustain_stability_threshold {
            self.stable_counter += 1;
        } else {
            self.stable_counter = 0;
            self.current_state = SignalState::Unstable;
        }

        if self.stable_counter >= self.config.required_stable_frames {
            self.current_state = SignalState::Stable;
        }
    }

    /// Resets the per-note state: the transient guard and the stability run.
    fn reset_note_state(&mut self) {
        self.transient_active = false;
        self.stable_counter = 0;
    }
}
