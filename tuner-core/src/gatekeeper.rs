//! # Gatekeeper Module
//!
//! Provides the primary stability evaluation logic for the continuous incoming
//! audio analysis stream. The Gatekeeper operates continuously alongside the
//! fundamental frequency (`f0`) engine, serving two primary functions:
//!
//! 1. **Signal Validation**: Emits a continuous, filtered status output (e.g.,
//!    `Stable`, `Unstable`, or `Silence`) to downstream consumers (such
//!    as the graphical interface).
//! 2. **Pool Dispatch Authorization**: Conditionally regulates the allocation and
//!    dispatch of high-capacity memory buffers from the `AudioPool` to the Thread 3
//!    `WorkerPool` when `capture_mode` is enabled and a target frequency attains
//!    defined stability metrics.

use crate::algorithms::power::{calculate_csd, calculate_ema, calculate_ninos2, calculate_rms};
use crate::pipeline::{AudioPool, ProcessingFrame};
use std::sync::Arc;

/// Configuration thresholds for the Gatekeeper's internal DSP algorithms.
/// These can be tuned to optimize stability detection for different piano registers.
#[derive(Debug, Clone)]
pub struct GatekeeperConfig {
    /// Minimum RMS amplitude required to exit the `Silence` state
    pub silence_threshold: f32,
    /// Number of frames to sample at startup/reset to establish the room's baseline ambient noise (e.g., 43 frames ≈ 2.0 seconds)
    pub noise_calibration_frames: usize,
    /// Multiplier applied to the calculated ambient noise to set the silence threshold (e.g., 2.0x)
    pub noise_safety_multiplier: f32,
    /// The smoothing factor for the Root Mean Square (RMS) Exponential Moving Average (EMA). 0.0 is infinite smoothing, 1.0 is instantaneous.
    pub rms_ema_alpha: f32,
    /// The threshold for Complex Spectral Difference (CSD) above which we declare a transient
    pub csd_transient_threshold: f32,
    /// How many frames to hard-wait after a CSD transient is detected (e.g., 10 frames ≈ 464ms)
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
            silence_threshold: 0.005, // Arbitrary hard-coded default (will be overwritten if calibrated)
            noise_calibration_frames: 43, // (~2.0 seconds)
            noise_safety_multiplier: 2.0, // Require signal to be 2x louder than ambient noise
            rms_ema_alpha: 0.1, // Strong smoothing to ride through momentary unison beating dips
            csd_transient_threshold: 15.0, // Arbitrary starting threshold
            transient_delay_frames: 10, // Hard bypass delay after transient (~464ms)
            ninos2_stability_threshold: 10.0, // Scale 1 (white noise) to N (pure tone)
            required_stable_frames: 4, // (~185ms)
            capture_max_frames: 32, // (~1.48 seconds)
        }
    }
}

/// Represents the discrete evaluation state of the realtime audio stream.
#[derive(Debug, Clone, PartialEq)]
pub enum SignalState {
    /// The stream contains a clear, steady fundamental frequency.
    Stable,
    /// The stream contains audio energy but lacks a clear fundamental frequency
    /// (e.g., attack transient, noise, inharmonic sounds).
    Unstable,
    /// The stream falls below the configured noise floor amplitude threshold.
    Silence,
}

/// Regulates stream state emissions and authorizes computational captures using a 5-state state machine.
pub struct Gatekeeper {
    pub capture_mode_enabled: bool,
    #[allow(dead_code)] // To be utilized upon full implementation
    audio_pool: Arc<AudioPool>,
    pub config: GatekeeperConfig,

    // Output state
    pub current_state: SignalState,

    // Internal DSP State memory
    // Pre-allocated array matching the size of ProcessingFrame.frequency_buffer (2048)
    // to prevent dynamic heap allocation on the audio hot-path.
    prev_spectrum: [rustfft::num_complex::Complex<f32>; 2048],

    // State machine counters
    pub transient_delay_counter: usize,
    pub stable_counter: usize,
    pub capture_counter: usize,
    pub is_capturing: bool,

    // Noise Baseline Calibration State
    pub noise_calibration_counter: usize,
    noise_calibration_sum: f32,

    // EMA State
    pub current_rms_ema: f32,
}

impl Gatekeeper {
    /// Instantiates a new Gatekeeper bound to the provided AudioPool.
    pub fn new(audio_pool: Arc<AudioPool>) -> Self {
        Self {
            audio_pool,
            capture_mode_enabled: false,
            config: GatekeeperConfig::default(),
            current_state: SignalState::Silence,
            prev_spectrum: [rustfft::num_complex::Complex { re: 0.0, im: 0.0 }; 2048],
            transient_delay_counter: 0,
            stable_counter: 0,
            capture_counter: 0,
            is_capturing: false,
            noise_calibration_counter: 0,
            noise_calibration_sum: 0.0,
            current_rms_ema: 0.0,
        }
    }

    /// Resets the baseline noise floor calibration, forcing the Gatekeeper to listen to
    /// the room's ambient noise for `config.noise_calibration_frames` before processing frames normally.
    pub fn reset_noise_floor(&mut self) {
        self.noise_calibration_counter = 0;
        self.noise_calibration_sum = 0.0;
        self.current_rms_ema = 0.0;
        self.current_state = SignalState::Silence;
    }

    /// Evaluates a single `ProcessingFrame` against stability heuristics.
    ///
    /// The 5-State Capture Logic:
    /// State 0: IDLE (Silence / Noise Floor Gating).
    /// State 1: ATTACK (Transient spike).
    /// State 2: TRANSIENT (Waiting for broadband noise to decay).
    /// State 3: HARMONIC DECAY (NINOS2 Stability gating / Golden Window).
    /// State 4: RELEASE (Hard timeout capture & dispatch).
    pub fn process_frame(&mut self, frame: &ProcessingFrame) {
        // State 0: Calculate RMS amplitude for Silence fallback
        let rms = calculate_rms(&frame.audio_buffer[..]);

        // Apply Exponential Moving Average (EMA) to smooth out momentary wave nodes / unison dips
        self.current_rms_ema = calculate_ema(rms, self.current_rms_ema, self.config.rms_ema_alpha);
        let smoothed_rms = self.current_rms_ema;

        // Dynamic Noise Floor Calibration Phase
        if self.process_noise_calibration(smoothed_rms) {
            return;
        }

        if smoothed_rms < self.config.silence_threshold {
            self.current_state = SignalState::Silence;
            self.reset_capture_state();
            return;
        }

        let current_spectrum = &frame.frequency_buffer[..];

        // State 1 & 2: Calculate CSD to detect transients
        if self.process_transient_detection(current_spectrum) {
            return;
        }

        // State 3 & 4: NINOS2 Stability Gating & Capture Dispatch
        self.process_stability_and_capture(current_spectrum);
    }

    /// Handles the dynamic noise floor calibration phase.
    /// Returns `true` if the Gatekeeper is actively calibrating and should skip further processing.
    fn process_noise_calibration(&mut self, smoothed_rms: f32) -> bool {
        if self.noise_calibration_counter >= self.config.noise_calibration_frames {
            return false;
        }

        self.noise_calibration_sum += smoothed_rms;
        self.noise_calibration_counter += 1;

        // If we just finished collecting frames, compute and set the threshold
        if self.noise_calibration_counter == self.config.noise_calibration_frames {
            let avg_noise =
                self.noise_calibration_sum / (self.config.noise_calibration_frames as f32);
            self.config.silence_threshold = avg_noise * self.config.noise_safety_multiplier;
            eprintln!(
                "[GATEKEEPER] Noise floor calibrated. Ambient RMS EMA: {:.5}, Threshold set to: {:.5}",
                avg_noise, self.config.silence_threshold
            );
        }

        self.current_state = SignalState::Silence;
        self.reset_capture_state();
        true
    }

    /// Calculates CSD to detect transients (hammer strikes) and manage the bypass delay.
    /// Returns `true` if a transient is detected or if we are waiting for a transient tail to die down.
    fn process_transient_detection(
        &mut self,
        current_spectrum: &[rustfft::num_complex::Complex<f32>],
    ) -> bool {
        // We only calculate CSD up to the size of the current spectrum to avoid out-of-bounds
        // if for some reason the buffer is smaller than our 2048 array.
        let csd = calculate_csd(
            &self.prev_spectrum[..current_spectrum.len()],
            current_spectrum,
        );

        // Update history (zero-allocation copy)
        self.prev_spectrum[..current_spectrum.len()].copy_from_slice(current_spectrum);

        // State 1: ATTACK
        if csd > self.config.csd_transient_threshold {
            // Hammer strike detected
            self.transient_delay_counter = self.config.transient_delay_frames;
            self.stable_counter = 0;
            self.current_state = SignalState::Unstable;
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

    /// Evaluates signal stability (NINOS2) and manages the capture phase.
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
                // Time to close the gate and dispatch the buffer!
                // TODO: Pop the 1.5s array out of audio_pool, fill with logic, send to Thread 3
                self.reset_capture_state();
            }
        }
    }

    fn reset_capture_state(&mut self) {
        self.transient_delay_counter = 0;
        self.stable_counter = 0;
        self.capture_counter = 0;
        self.is_capturing = false;
    }
}
