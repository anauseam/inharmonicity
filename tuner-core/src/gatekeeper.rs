//! # Gatekeeper — 5-State Signal Validator (Pure DSP)
//!
//! The Gatekeeper is the "traffic cop" of the audio pipeline. It evaluates every
//! incoming [`ProcessingFrame`] against a sequence of stability heuristics and
//! outputs a discrete [`SignalState`] (`Silence`, `Unstable`, or `Stable`) that
//! downstream consumers (GUI, capture logic) observe.
//!
//! ## Pure DSP — No Shared State
//!
//! The Gatekeeper has **zero knowledge** of `Arc`, `Mutex`, or the GUI. It returns
//! its observations as a [`GateResult`] from every [`Gatekeeper::process_frame`]
//! call; the [`AudioPipeline`](crate::pipeline::AudioPipeline) reads that result
//! and syncs it to shared state on behalf of the frontend.
//!
//! ## 5-State Capture Logic
//!
//! | State | Name | Metric | Purpose |
//! |---|---|---|---|
//! | 0 | IDLE | RMS + EMA | Silence gating — ignore background noise |
//! | 1 | ATTACK | NHWRSF | Detect the hammer strike transient |
//! | 2 | TRANSIENT | NHWRSF drop | One-frame buffer: resolves transient_active flag once NHWRSF falls |
//! | 3 | HARMONIC DECAY | NINOS2 (EMA) | Identify the "Golden Window" of stable harmonics |
//! | 4 | RELEASE | RMS + EMA | The decay back to `Silence` calls the note over, which is what ends a record |
//!
//! ## Noise Floor
//!
//! The silence threshold is provided externally via `config.silence_threshold`,
//! set by the standalone [`calibration`](crate::calibration) module or the GUI slider.
//! The Gatekeeper has no knowledge of how the threshold was computed.

use crate::algorithms::metrics::{ema, nhwrsf, ninos2, rms};
use crate::audio::{SAMPLE_RATE, WINDOW_SIZE};
use crate::pipeline::ProcessingFrame;

/// Configuration thresholds for the Gatekeeper's internal DSP algorithms.
/// These can be tuned to optimize stability detection for different piano registers.
#[derive(Debug, Clone)]
pub struct GatekeeperConfig {
    /// Minimum RMS amplitude required to exit the `Silence` state.
    /// Set externally by the calibration module or the GUI slider.
    pub silence_threshold: f32,
    /// The smoothing factor for the Root Mean Square (RMS) Exponential Moving Average (EMA). 0.0 is infinite smoothing, 1.0 is instantaneous.
    pub rms_ema_alpha: f32,
    /// The threshold for Normalized Half-Wave Rectified Spectral Flux (NHWRSF) above which we declare a transient
    pub nhwrsf_threshold: f32,
    /// The NINOS2 sparsity threshold above which the signal is considered harmonically stable
    pub ninos2_stability_threshold: f32,
    /// Smoothing factor for the NINOS2 EMA. Chosen at 0.5 to ride through
    /// single-frame phase-cancellation dropouts during unison beating while
    /// still clearing the stability threshold within 1 frame for treble decay.
    pub ninos2_ema_alpha: f32,
    /// How many consecutive frames the NINOS2 threshold must be met to declare the signal `Stable` (e.g., 4 frames ≈ 93 ms)
    pub required_stable_frames: usize,
}

impl Default for GatekeeperConfig {
    fn default() -> Self {
        // One frame per COLA hop: 1024 samples @ 44.1 kHz ≈ 23.2 ms. Each frame
        // analyses a 2048-sample window, but the windows overlap 50 %, so a
        // frame count converts to elapsed time at the hop rate, never at the
        // window length.
        Self {
            silence_threshold: 0.005, // Default until overwritten by calibration or GUI
            rms_ema_alpha: 0.1, // Strong smoothing to ride through momentary unison beating dips
            nhwrsf_threshold: 0.5, // Arbitrary starting threshold
            ninos2_stability_threshold: 10.0, // Scale 1 (white noise) to N (pure tone)
            ninos2_ema_alpha: 0.5, // Smooths over phase cancellation dips during unison beating
            required_stable_frames: 4, // (~93 ms)
        }
    }
}

/// The Gatekeeper's output — a discrete evaluation of the audio stream.
///
/// The GUI observes this value (via the pipeline's shared state) to drive
/// visual feedback (e.g., "listening…", "note detected", silence indicator).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalState {
    /// The stream contains a clear, steady fundamental frequency.
    /// The NINOS2 metric has exceeded the stability threshold for
    /// `required_stable_frames` consecutive frames.
    Stable,
    /// The stream contains audio energy but lacks a clear fundamental frequency
    /// (e.g., hammer strike transient, broadband noise, inharmonic sounds).
    /// This covers States 1 (ATTACK) and 2 (TRANSIENT) of the state machine.
    Unstable,
    /// The EMA-smoothed RMS falls below `silence_threshold`.
    /// No DSP beyond RMS is performed in this state (bypasses NHWRSF, NINOS2).
    Silence,
}

/// Observed outputs of a single Gatekeeper frame evaluation.
///
/// Returned by value from [`Gatekeeper::process_frame`].
#[derive(Debug, Clone, Copy)]
pub struct GateResult {
    pub rms_ema: f32,
    pub nhwrsf: f32,
    pub ninos2_ema: f32,
    pub ninos2_raw: f32,
    pub state: SignalState,
    pub is_new_onset: bool,
    pub is_transient_bypass: bool,
}

/// The 5-state signal validator. Pure DSP — no shared state awareness.
///
/// See the [module-level docs](crate::gatekeeper) for the full state machine
/// description and the role of each metric.
pub struct Gatekeeper {
    pub config: GatekeeperConfig,

    // Output state
    pub(crate) current_state: SignalState,

    // Internal DSP State memory
    // Pre-allocated array matching the size of ProcessingFrame.frequency_buffer (2048)
    // to prevent dynamic heap allocation on the audio hot-path.
    prev_spectrum: Box<[f32]>,

    pub(crate) current_nhwrsf: f32,
    pub(crate) current_ninos2_ema: f32,
    pub(crate) current_ninos2_raw: f32,

    // State machine counter (internal bookkeeping — not exposed)
    stable_counter: usize,

    // Dynamic transient gating state
    transient_active: bool,

    // EMA State
    pub(crate) current_rms_ema: f32,

    // Expose transient detection state for routing engine
    pub(crate) is_new_onset: bool,
    pub(crate) is_transient_bypass: bool,
}

impl Default for Gatekeeper {
    fn default() -> Self {
        Self::new()
    }
}

impl Gatekeeper {
    /// Creates a new Gatekeeper.
    ///
    /// All counters and EMA state are zeroed. The Gatekeeper starts in
    /// [`SignalState::Silence`].
    pub fn new() -> Self {
        Self {
            config: GatekeeperConfig::default(),
            current_state: SignalState::Silence,
            prev_spectrum: vec![0.0; 2048].into_boxed_slice(),
            current_nhwrsf: 0.0,
            current_ninos2_ema: 0.0,
            current_ninos2_raw: 0.0,
            stable_counter: 0,
            current_rms_ema: 0.0,
            is_new_onset: false,
            is_transient_bypass: false,
            transient_active: false,
        }
    }

    /// Evaluates a single [`ProcessingFrame`] through the 5-state machine.
    ///
    /// This is the main entry point called by [`AudioPipeline::process_cola_hop()`].
    /// Returns a [`GateResult`] snapshot of the evaluated signal state.
    ///
    /// ## State Machine Flow
    ///
    /// 1. **RMS + EMA** — compute smoothed amplitude
    /// 2. **Silence gate** — if below threshold, emit `Silence` and reset
    /// 3. **NHWRSF transient detection** — States 1 & 2
    /// 4. **NINOS2 stability** — State 3 (State 4, the capture itself, is the pipeline's)
    pub fn process_frame(&mut self, frame: &ProcessingFrame) -> GateResult {
        // State 0: Calculate RMS amplitude for Silence fallback
        // Slice only the newest WINDOW_SIZE samples from the historical buffer to keep transient detection snappy
        let rms = rms(&frame.audio_buffer[frame.audio_buffer.len() - WINDOW_SIZE..]);

        // Evaluate signal deterioration using EMA alpha.
        // We rely purely on the slow EMA release or the next NHWRSF onset
        // to detect the note boundary.
        let alpha = if rms > self.current_rms_ema {
            1.0 // Instant attack: track volume surges immediately
        } else {
            self.config.rms_ema_alpha // Slow release: Ride smoothly over unison beating dips
        };

        // Apply dynamic Exponential Moving Average
        self.current_rms_ema = ema(rms, self.current_rms_ema, alpha);
        let smoothed_rms = self.current_rms_ema;

        // State 0: Transient Guard — do not abort to Silence mid-transient.
        // A weak note's RMS can dip below the silence threshold during the mechanical
        // strike itself. If transient_active is true, we know a transient is actively
        // resolving and we ride it out.
        if smoothed_rms < self.config.silence_threshold && !self.transient_active {
            self.current_state = SignalState::Silence;
            self.is_new_onset = false;
            self.is_transient_bypass = false;
            self.current_ninos2_ema = 0.0;
            self.current_ninos2_raw = 0.0;
            self.current_nhwrsf = 0.0;
            self.reset_note_state();
            return self.build_result();
        }

        let current_spectrum = &frame.frequency_buffer[..];

        // Calculate all active-state spectral metrics
        let raw_ninos2 = ninos2(current_spectrum);
        self.current_ninos2_raw = raw_ninos2;
        self.current_ninos2_ema = ema(
            raw_ninos2,
            self.current_ninos2_ema,
            self.config.ninos2_ema_alpha,
        );
        self.current_nhwrsf = nhwrsf(
            current_spectrum,
            &mut self.prev_spectrum[..],
            WINDOW_SIZE,
            SAMPLE_RATE,
        );

        // State 1 & 2: Transient detection routing
        self.is_transient_bypass = self.process_transient_detection();
        if self.is_transient_bypass {
            return self.build_result();
        }

        // State 3: Stability routing
        self.process_stability();

        self.build_result()
    }

    #[inline]
    fn build_result(&self) -> GateResult {
        GateResult {
            rms_ema: self.current_rms_ema,
            nhwrsf: self.current_nhwrsf,
            ninos2_ema: self.current_ninos2_ema,
            ninos2_raw: self.current_ninos2_raw,
            state: self.current_state,
            is_new_onset: self.is_new_onset,
            is_transient_bypass: self.is_transient_bypass,
        }
    }

    /// Detects transient events (States 1 & 2) using NHWRSF.
    ///
    /// **State 1 (ATTACK):** If NHWRSF exceeds `nhwrsf_threshold`, a hammer
    /// strike is declared. The transient delay counter is armed.
    ///
    /// **State 2 (TRANSIENT):** While the delay counter is nonzero, the Gatekeeper
    /// waits for the broadband strike noise to physically decay.
    ///
    /// Also updates `prev_spectrum` for the next frame's NHWRSF calculation.
    ///
    /// # Returns
    /// `true` if a transient was detected or we are still in the bypass delay.
    fn process_transient_detection(&mut self) -> bool {
        self.is_new_onset = false;

        // Failsafe: ALWAYS reset capture on a new hammer strike.
        // Even if EMA hasn't hit silence_threshold yet, a new strike means a new note.
        if self.current_nhwrsf > self.config.nhwrsf_threshold {
            self.stable_counter = 0;
            self.current_state = SignalState::Unstable;
            self.is_new_onset = true;
            self.reset_note_state();
            self.transient_active = true; // arm AFTER reset so it isn't cleared by reset_note_state
            return true;
        }

        // State 2: TRANSIENT — Wait for onset energy to physically subside.
        if self.transient_active {
            // Once NHWRSF drops below threshold, the physical transient is over.
            self.transient_active = false;
        }

        false
    }

    /// Evaluates spectral stability (State 3).
    ///
    /// The NINOS2 sparsity metric must exceed `ninos2_stability_threshold` for
    /// `required_stable_frames` consecutive frames before the signal is declared
    /// [`Stable`](SignalState::Stable). That verdict is what a record starts on,
    /// as the return to [`Silence`](SignalState::Silence) is what ends one: the
    /// Gatekeeper decides *when*, and the pipeline — which owns `CaptureState`
    /// and the buffer — acts on it.
    fn process_stability(&mut self) {
        // current_ninos2_ema is calculated unconditionally in process_frame
        if self.current_ninos2_ema > self.config.ninos2_stability_threshold {
            self.stable_counter += 1;
        } else {
            self.stable_counter = 0;
            self.current_state = SignalState::Unstable;
        }

        if self.stable_counter >= self.config.required_stable_frames {
            self.current_state = SignalState::Stable;
        }
    }

    /// Resets the per-note counters — the transient guard and the stability run.
    ///
    /// Called on silence and on a new onset.
    fn reset_note_state(&mut self) {
        self.transient_active = false;
        self.stable_counter = 0;
    }
}
