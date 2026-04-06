//! # Engine (Thread 2) — Fundamental Frequency Detection
//!
//! The "Brains" of the pipeline. The Engine is orchestrated by the `AudioPipeline`
//! after the signal has been validated by the Gatekeeper. Its sole responsibility
//! is to process the signal and extract the exact fundamental frequency.
//!
//! ## Sequence of processing:
//!
//! 1. Run the Scout Engine to determine rough frequency neighborhood.
//! 2. Route candidate frequencies through the Two-Way Mismatch (TWM) algorithm to find coarse F0.
//! 3. Extract sub-cent accurate F0 by passing the TWM seed bin into XQIFFT (or legacy QIFFT).
//! 4. Output the resulting F0 and confidence back to the pipeline.

use crate::algorithms::{metrics, pitch, spectral};
use crate::audio::{BASS_WINDOW_SIZE, WINDOW_SIZE};
use crate::pipeline::ProcessingFrame;
use std::sync::Arc;

/// Selects the refinement pitch detection algorithm (after TWM stage 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefinementAlgorithm {
    /// Quadratic Interpolated FFT — frequency-domain sub-bin peak detection.
    XQIFFT,
    /// Digital Phase-Locked Loop — time-domain phase tracking (future).
    DPLL,
    /// Phase Vocoder - frequency-domain phase angle measurement
    PVOCODER,
    /// Classical Quadratic Interpolated FFT
    QIFFT,
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
    pub refinement_algorithm: RefinementAlgorithm,
    pub sample_rate: u32,
    pub routing_state: RoutingState,
    pub consecutive_bass_votes: usize,
    pub consecutive_treble_votes: usize,
    pub key_hint: Option<f32>,
    pub inharmonicity_b: Option<f32>,
    pub xqifft_p: f32,
    /// Scratch space for storing detected local maxima when creating candidates for TWM.
    pub peak_scratch: Box<[crate::algorithms::twm::SpectralPeak]>,
    /// Dedicated FFT instance for generating BASS_WINDOW_SIZE spectrums on bass notes.
    pub fft_bass_instance: Arc<dyn realfft::RealToComplex<f32>>,
}

impl Engine {
    /// Creates a new Engine with default algorithms.
    pub fn new(sample_rate: u32) -> Self {
        let mut planner = realfft::RealFftPlanner::<f32>::new();
        let fft_bass_instance = planner.plan_fft_forward(BASS_WINDOW_SIZE);

        Self {
            refinement_algorithm: RefinementAlgorithm::QIFFT,
            sample_rate,
            routing_state: RoutingState::Unclassified,
            consecutive_bass_votes: 0,
            consecutive_treble_votes: 0,
            key_hint: None,
            inharmonicity_b: None,
            xqifft_p: 0.5,
            peak_scratch: vec![crate::algorithms::twm::SpectralPeak::default(); 30]
                .into_boxed_slice(),
            fft_bass_instance,
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
        }

        // If unclassified, use the Band Energy Classifier to lock a routing path.
        if self.routing_state == RoutingState::Unclassified {
            let expected_bins = WINDOW_SIZE / 2 + 1; // 1025 for a 2048-sample FFT
            let ratio =
                metrics::evaluate_band_energy_ratio(&frame.frequency_buffer[..expected_bins]);

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

        // ── Step 1: Bass FFT (8192-point) when Scout locks Bass ──────────
        if self.routing_state == RoutingState::LockedBass {
            crate::algorithms::spectral::perform_fft(
                &frame.audio_buffer[..BASS_WINDOW_SIZE],
                &mut frame.time_buffer[..BASS_WINDOW_SIZE],
                &mut frame.bass_frequency_buffer[..],
                &self.fft_bass_instance,
                BASS_WINDOW_SIZE,
            );
        }

        // ── Step 2: Conditional magnitude extraction (zero-alloc) ────────
        let (active_bins, active_window_size) = match self.routing_state {
            RoutingState::LockedBass => {
                let bins = BASS_WINDOW_SIZE / 2;
                let expected_complex = BASS_WINDOW_SIZE / 2 + 1;
                spectral::spectrum_to_magnitudes(
                    &frame.bass_frequency_buffer[..expected_complex],
                    BASS_WINDOW_SIZE,
                    &mut frame.magnitude_buffer[..bins],
                );
                (bins, BASS_WINDOW_SIZE)
            }
            _ => {
                // LockedTreble or Unclassified — standard WINDOW_SIZE-point rapid FFT
                let bins = WINDOW_SIZE / 2;
                let expected_complex = WINDOW_SIZE / 2 + 1;
                spectral::spectrum_to_magnitudes(
                    &frame.frequency_buffer[..expected_complex],
                    WINDOW_SIZE,
                    &mut frame.magnitude_buffer[..bins],
                );
                (bins, WINDOW_SIZE)
            }
        };
        let spectrogram_data = &frame.magnitude_buffer[..active_bins];

        // TODO(DPLL): When DPLL refinement is activated, this must use active_window_size.
        let _audio_frame = &frame.audio_buffer[..WINDOW_SIZE];

        // ── Step 3: Peak extraction + TWM with dynamic window ────────────
        let peak_count = crate::algorithms::twm::extract_spectral_peaks(
            spectrogram_data,
            self.sample_rate,
            active_window_size,
            &mut self.peak_scratch,
        );

        let search_bounds = match self.routing_state {
            RoutingState::LockedBass => Some((27.5, 400.0)),
            RoutingState::LockedTreble => Some((130.0, 4186.0)),
            RoutingState::Unclassified => Some((27.5, 4186.0)),
        };

        let twm_result = crate::algorithms::twm::detect_pitch_twm(
            &self.peak_scratch[..peak_count],
            self.sample_rate,
            search_bounds,
            self.key_hint,
            self.inharmonicity_b,
        );

        // ── Step 4: Sub-cent refinement on the active magnitude buffer ───
        if let Some((coarse_f0, _)) = twm_result {
            match self.refinement_algorithm {
                RefinementAlgorithm::XQIFFT => pitch::detect_pitch_xqifft_seeded(
                    spectrogram_data,
                    self.sample_rate,
                    coarse_f0,
                    self.xqifft_p,
                )
                .map(|freq| (freq, None)),
                RefinementAlgorithm::DPLL => {
                    pitch::detect_pitch_dpll(_audio_frame, self.sample_rate, coarse_f0)
                }
                RefinementAlgorithm::PVOCODER => None,
                RefinementAlgorithm::QIFFT => {
                    pitch::detect_pitch_qifft_seeded(spectrogram_data, self.sample_rate, coarse_f0)
                        .map(|freq| (freq, None))
                }
            }
        } else {
            None
        }
    }
}
