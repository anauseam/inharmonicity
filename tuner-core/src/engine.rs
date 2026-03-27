//! # Engine (Thread 2) — Fundamental Frequency Detection
//!
//! The "Brains" of the pipeline. The Engine is orchestrated by the `AudioPipeline`
//! after the signal has been validated by the Gatekeeper. Its sole responsibility
//! is to process the signal and extract the exact fundamental frequency.
//!
//! ## Sequence of processing:
//!
//! 1. Run the Scout Engine to determine rough frequency neighborhood.
//! 2. Route to Bass Engine (pYIN) or Treble Engine (QIFFT / DPLL) to extract the exact F0.
//! 3. Output the resulting F0 and confidence back to the pipeline.

use crate::algorithms::{dpyin, metrics, pitch, spectral};
use crate::pipeline::ProcessingFrame;

/// Selects the Treble (>= 150 Hz) pitch refinement algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrebleAlgorithm {
    /// Quadratic Interpolated FFT — frequency-domain sub-bin peak detection.
    QIFFT,
    /// Digital Phase-Locked Loop — time-domain phase tracking (future).
    DPLL,
    /// Phase Vocoder - frequency-domain phase angle measurement
    PVOCODER,
}

/// Selects the Bass (< 150 Hz) pitch detection algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BassAlgorithm {
    /// Decimated Probabilistic YIN — robust against octave errors on stiff bass strings.
    DPYIN,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RoutingState {
    Unclassified,
    LockedBass,
    LockedTreble,
}

/// The Fundamental Frequency ($f_0$) Engine.
///
/// Executes the Scout → Router → Bass/Treble detection chain.
pub struct Engine {
    pub treble_algorithm: TrebleAlgorithm,
    pub bass_algorithm: BassAlgorithm,
    pub sample_rate: u32,
    /// Previous winning lag from the bass DPYIN algorithm (in the decimated domain).
    /// Used to give the Viterbi decoder temporal continuity across frames.
    prev_bass_lag: Option<f32>,
    pub routing_state: RoutingState,
    pub consecutive_bass_votes: usize,
    pub consecutive_treble_votes: usize,
}

impl Engine {
    /// Creates a new Engine with default algorithms.
    pub fn new(sample_rate: u32) -> Self {
        Self {
            treble_algorithm: TrebleAlgorithm::QIFFT,
            bass_algorithm: BassAlgorithm::DPYIN,
            sample_rate,
            prev_bass_lag: None,
            routing_state: RoutingState::Unclassified,
            consecutive_bass_votes: 0,
            consecutive_treble_votes: 0,
        }
    }

    /// Evaluates the Gatekeeper state and the Band Energy Ratio to determine if the
    /// signal should be routed to the Bass or Treble detection pipeline.
    ///
    /// Returns `true` if the signal is actively locked into a pipeline, or `false`
    /// if it is still unclassified (waiting for consensus).
    fn update_routing_state(
        &mut self,
        frame: &ProcessingFrame,
        is_silence: bool,
        is_new_onset: bool,
    ) -> bool {
        // Reset routing state if the signal has dropped to silence or a new hammer strike (CSD) occurs.
        if is_silence || is_new_onset {
            self.routing_state = RoutingState::Unclassified;
            self.consecutive_bass_votes = 0;
            self.consecutive_treble_votes = 0;
            self.prev_bass_lag = None;
        }

        // If unclassified, use the Band Energy Classifier to lock a routing path.
        if self.routing_state == RoutingState::Unclassified {
            let expected_bins = 2048 / 2 + 1; // 1025 for a 2048-sample FFT
            let ratio = metrics::evaluate_band_energy_ratio(&frame.frequency_buffer[..expected_bins]);
            
            // Asymmetric thresholding (Schmitt trigger hysteresis logic)
            if ratio > 0.25 {
                self.consecutive_bass_votes += 1;
                self.consecutive_treble_votes = 0;
            } else if ratio < 0.15 {
                self.consecutive_treble_votes += 1;
                self.consecutive_bass_votes = 0;
            } else {
                // Borderline frame (15-25%), resets confidence
                self.consecutive_bass_votes = 0;
                self.consecutive_treble_votes = 0;
            }

            // Lock routing if consensus reached
            if self.consecutive_bass_votes >= 3 {
                self.routing_state = RoutingState::LockedBass;
                println!("Scout Locked: Bass Engine");
            } else if self.consecutive_treble_votes >= 3 {
                self.routing_state = RoutingState::LockedTreble;
                println!("Scout Locked: Treble Engine");
            }

            // Delay extracting fundamental frequencies until the lock is established
            return false;
        }

        true
    }

    /// Executes the primary DSP detection loop for a single frame.
    ///
    /// The `ProcessingFrame` must already have its `frequency_buffer` populated
    /// by the Gatekeeper's RFFT. Returns the detected frequency (Hz) and its confidence (0.0 - 1.0).
    /// Note that confidence is optional, as some algorithms may not produce a confidence metric.
    pub fn process(
        &mut self,
        frame: &mut ProcessingFrame,
        is_silence: bool,
        is_new_onset: bool,
    ) -> Option<(f32, Option<f32>)> {
        // Evaluate routing locks before proceeding to pitch detection
        if !self.update_routing_state(frame, is_silence, is_new_onset) {
            return None;
        }

        // We only use the first half of the frequency buffer (up to Nyquist)
        let expected_bins = 2048 / 2 + 1; // 1025 for a 2048-sample FFT
        let spectrogram_data =
            spectral::spectrum_to_magnitudes(&frame.frequency_buffer[..expected_bins]);

        let frame_size = 2048;
        let _audio_frame = &frame.audio_buffer[..frame_size];

        match self.routing_state {
            RoutingState::LockedBass => {
                // Route to Bass Engine — uses full 8192-sample audio buffer for decimation
                match self.bass_algorithm {
                    BassAlgorithm::DPYIN => {
                        let result = dpyin::detect_pitch_dpyin(
                            &frame.audio_buffer[..],
                            self.sample_rate,
                            &mut frame.time_buffer[..],
                            self.prev_bass_lag,
                        );
                        // Store the winning lag for next frame's Viterbi transition penalty.
                        // Convert frequency back to lag in the decimated domain:
                        //   lag = decimated_sample_rate / frequency
                        const DECIMATED_SR: f32 = 44_100.0 / 8.0; // 5512.5 Hz
                        if let Some((freq, _)) = result {
                            self.prev_bass_lag = Some(DECIMATED_SR / freq);
                        }
                        result
                    }
                }
            }
            RoutingState::LockedTreble => {
                // Route to Treble Engine
                match self.treble_algorithm {
                    TrebleAlgorithm::QIFFT => {
                        pitch::detect_pitch_qifft(&spectrogram_data, self.sample_rate)
                    }
                    TrebleAlgorithm::DPLL => {
                        pitch::detect_pitch_dpll(_audio_frame, self.sample_rate, 0.0)
                    }
                    TrebleAlgorithm::PVOCODER => {
                        // PVOCODER is stubbed out for future implementation
                        None
                    }
                }
            }
            RoutingState::Unclassified => unreachable!(),
        }
    }
}
