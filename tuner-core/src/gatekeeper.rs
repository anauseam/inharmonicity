//! # Gatekeeper — 5-State Signal Validator (Pure DSP)
//!
//! The Gatekeeper is the "traffic cop" of the audio pipeline. It evaluates every
//! incoming [`ProcessingFrame`] against a sequence of stability heuristics and
//! outputs a discrete [`SignalState`] (`Silence`, `Unstable`, or `Stable`) that
//! downstream consumers (GUI, capture logic) observe.
//!
//! ## Pure DSP — No Shared State
//!
//! The Gatekeeper has **zero knowledge** of `Arc`, `Mutex`, or the GUI. It exposes
//! its observations through `pub` fields (e.g., `current_rms_ema`, `current_state`).
//! The [`AudioPipeline`](crate::pipeline::AudioPipeline) reads these fields after
//! each frame and syncs them to shared state on behalf of the frontend.
//!
//! ## 5-State Capture Logic
//!
//! | State | Name | Metric | Purpose |
//! |---|---|---|---|
//! | 0 | IDLE | RMS + EMA | Silence gating — ignore background noise |
//! | 1 | ATTACK | NHWRSF | Detect the hammer strike transient |
//! | 2 | TRANSIENT | Counter | Hard delay for broadband noise to decay |
//! | 3 | HARMONIC DECAY | NINOS2 | Identify the "Golden Window" of stable harmonics |
//! | 4 | RELEASE | Counter | Cap capture at 1.5s, dispatch to Worker, reset |
//!
//! ## Noise Floor
//!
//! The silence threshold is provided externally via `config.silence_threshold`,
//! set by the standalone [`calibration`](crate::calibration) module or the GUI slider.
//! The Gatekeeper has no knowledge of how the threshold was computed.

use crate::algorithms::metrics::{
    calculate_ema, calculate_nhwrsf, calculate_ninos2, calculate_rms,
};
use crate::audio::WINDOW_SIZE;
use crate::pipeline::{AudioPool, ProcessingFrame};
use std::sync::Arc;

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
    /// How many frames to hard-wait after a NHWRSF transient is detected (e.g., 10 frames ≈ 464ms)
    pub transient_delay_frames: usize,
    /// The NINOS2 sparsity threshold above which the signal is considered harmonically stable
    pub ninos2_stability_threshold: f32,
    /// How many consecutive frames the NINOS2 threshold must be met to declare the signal `Stable` (e.g., 4 frames ≈ 185ms)
    pub required_stable_frames: usize,
    /// Hard limit on the number of frames to capture (e.g., 32 frames ≈ 1.5 seconds)
    pub capture_max_frames: usize,
}

impl Default for GatekeeperConfig {
    fn default() -> Self {
        // At 44.1kHz with a 2048 sample buffer:
        // 1 Frame = 2048 / 44100 ≈ 0.0464 seconds (46.4 milliseconds)
        Self {
            silence_threshold: 0.005, // Default until overwritten by calibration or GUI
            rms_ema_alpha: 0.1, // Strong smoothing to ride through momentary unison beating dips
            nhwrsf_threshold: 0.5, // Arbitrary starting threshold
            transient_delay_frames: 10, // Hard bypass delay after transient (~464ms)
            ninos2_stability_threshold: 10.0, // Scale 1 (white noise) to N (pure tone)
            required_stable_frames: 4, // (~185ms)
            capture_max_frames: 32, // (~1.48 seconds)
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
/// Returned by value from [`Gatekeeper::process_frame`]. Replaces direct
/// field reads of internal state, eliminating temporal coupling.
#[derive(Debug, Clone, Copy)]
pub struct GateResult {
    pub rms_ema: f32,
    pub nhwrsf: f32,
    pub state: SignalState,
    pub is_new_onset: bool,
}

/// The 5-state signal validator. Pure DSP — no shared state awareness.
///
/// See the [module-level docs](crate::gatekeeper) for the full state machine
/// description and the role of each metric.
pub struct Gatekeeper {
    pub capture_mode_enabled: bool,
    #[allow(dead_code)] // To be utilized upon full implementation
    audio_pool: Arc<AudioPool>,
    pub config: GatekeeperConfig,

    // Output state
    pub(crate) current_state: SignalState,

    // Internal DSP State memory
    // Pre-allocated array matching the size of ProcessingFrame.frequency_buffer (2048)
    // to prevent dynamic heap allocation on the audio hot-path.
    prev_spectrum: Box<[f32]>,

    pub(crate) current_nhwrsf: f32,

    // State machine counters (internal bookkeeping — not exposed)
    transient_delay_counter: usize,
    stable_counter: usize,
    capture_counter: usize,
    is_capturing: bool,

    // EMA State
    pub(crate) current_rms_ema: f32,

    // Expose transient detection state for routing engine
    pub(crate) is_new_onset: bool,
}

impl Gatekeeper {
    /// Creates a new Gatekeeper bound to the provided [`AudioPool`].
    ///
    /// All counters and EMA state are zeroed. The Gatekeeper starts in
    /// [`SignalState::Silence`].
    ///
    /// # Arguments
    /// * `audio_pool` — Shared reference to the lock-free object pool
    ///   (used for buffer dispatch in State 4).
    pub fn new(audio_pool: Arc<AudioPool>) -> Self {
        Self {
            audio_pool,
            capture_mode_enabled: false,
            config: GatekeeperConfig::default(),
            current_state: SignalState::Silence,
            prev_spectrum: vec![0.0; 2048].into_boxed_slice(),
            current_nhwrsf: 0.0,
            transient_delay_counter: 0,
            stable_counter: 0,
            capture_counter: 0,
            is_capturing: false,
            current_rms_ema: 0.0,
            is_new_onset: false,
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
    /// 4. **NINOS2 stability + capture** — States 3 & 4
    pub fn process_frame(&mut self, frame: &ProcessingFrame) -> GateResult {
        // State 0: Calculate RMS amplitude for Silence fallback
        // Slice only the newest WINDOW_SIZE samples from the historical buffer to keep transient detection snappy
        let rms = calculate_rms(&frame.audio_buffer[frame.audio_buffer.len() - WINDOW_SIZE..]);

        // Apply Exponential Moving Average (EMA) to smooth out momentary wave nodes / unison dips
        self.current_rms_ema = calculate_ema(rms, self.current_rms_ema, self.config.rms_ema_alpha);
        let smoothed_rms = self.current_rms_ema;

        if smoothed_rms < self.config.silence_threshold {
            self.current_state = SignalState::Silence;
            self.is_new_onset = false;
            self.reset_capture_state();
            return self.build_result();
        }

        let current_spectrum = &frame.frequency_buffer[..];

        // State 1 & 2: Calculate NHWRSF to detect transients
        if self.process_transient_detection(current_spectrum) {
            return self.build_result();
        }

        // State 3 & 4: NINOS2 Stability Gating & Capture Dispatch
        self.process_stability_and_capture(current_spectrum);

        self.build_result()
    }

    #[inline]
    fn build_result(&self) -> GateResult {
        GateResult {
            rms_ema: self.current_rms_ema,
            nhwrsf: self.current_nhwrsf,
            state: self.current_state,
            is_new_onset: self.is_new_onset,
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
    fn process_transient_detection(
        &mut self,
        current_spectrum: &[rustfft::num_complex::Complex<f32>],
    ) -> bool {
        let nhwrsf = calculate_nhwrsf(current_spectrum, &mut self.prev_spectrum[..]);
        self.current_nhwrsf = nhwrsf;

        // Reset the onset flag by default; it only fires true on the exact frame the transient triggers
        self.is_new_onset = false;

        // State 1: ATTACK
        if nhwrsf > self.config.nhwrsf_threshold {
            // Hammer strike detected
            self.transient_delay_counter = self.config.transient_delay_frames;
            self.stable_counter = 0;
            self.current_state = SignalState::Unstable;
            self.is_new_onset = true;
            return true;
        }

        // State 2: TRANSIENT
        if self.transient_delay_counter > 0 {
            // Waiting for transient tail to die down
            self.transient_delay_counter -= 1;
            self.current_state = SignalState::Unstable;
            return true;
        }

        false
    }

    /// Evaluates spectral stability (State 3) and manages audio capture (State 4).
    ///
    /// **State 3 (HARMONIC DECAY):** The NINOS2 sparsity metric must exceed
    /// `ninos2_stability_threshold` for `required_stable_frames` consecutive
    /// frames before the signal is declared [`Stable`](SignalState::Stable).
    ///
    /// **State 4 (RELEASE):** Once stable and `capture_mode_enabled`, the
    /// Gatekeeper counts frames up to `capture_max_frames` (~1.5s), then
    /// dispatches the buffer to the Worker and resets.
    fn process_stability_and_capture(
        &mut self,
        current_spectrum: &[rustfft::num_complex::Complex<f32>],
    ) {
        // State 3: HARMONIC DECAY (NINOS2 Stability Gating)
        let ninos2 = calculate_ninos2(current_spectrum);

        if ninos2 > self.config.ninos2_stability_threshold {
            self.stable_counter += 1;
        } else {
            self.stable_counter = 0;
            self.current_state = SignalState::Unstable;
        }

        if self.stable_counter >= self.config.required_stable_frames {
            self.current_state = SignalState::Stable;

            // Capture Mode Execution logic for State 3
            if self.capture_mode_enabled && !self.is_capturing {
                self.is_capturing = true;
            }
        }

        // Handle ongoing capture timeout
        if self.is_capturing {
            self.capture_counter += 1;
            if self.capture_counter >= self.config.capture_max_frames {
                // The AudioPipeline mediator monitors this state transition.
                // It handles popping the filled capture buffer out of the audio_pool
                // and dispatching it to the Worker thread (Thread 3).
                self.reset_capture_state();
            }
        }
    }

    /// Resets all capture-related state machine counters.
    ///
    /// Called when transitioning back to Silence, after a capture completes,
    /// or during noise floor calibration.
    fn reset_capture_state(&mut self) {
        self.transient_delay_counter = 0;
        self.stable_counter = 0;
        self.capture_counter = 0;
        self.is_capturing = false;
    }
}
