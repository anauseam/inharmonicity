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

use crate::algorithms::{dpyin, pitch, scout, spectral};
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

/// The Fundamental Frequency ($f_0$) Engine.
///
/// Executes the Scout → Router → Bass/Treble detection chain.
pub struct Engine {
    pub treble_algorithm: TrebleAlgorithm,
    pub bass_algorithm: BassAlgorithm,
    pub sample_rate: u32,
}

impl Engine {
    /// Creates a new Engine with default algorithms.
    pub fn new(sample_rate: u32) -> Self {
        Self {
            treble_algorithm: TrebleAlgorithm::QIFFT,
            bass_algorithm: BassAlgorithm::DPYIN,
            sample_rate,
        }
    }

    /// Executes the primary DSP detection loop for a single frame.
    ///
    /// The `ProcessingFrame` must already have its `frequency_buffer` populated
    /// by the Gatekeeper's RFFT. Returns the detected frequency (Hz) and its confidence (0.0 - 1.0).
    /// Note that confidence is optional, as some algorithms may not produce a confidence metric.
    pub fn process(
        &self,
        frame: &mut ProcessingFrame,
        amplitude_threshold: f32,
    ) -> Option<(f32, Option<f32>)> {
        // We only use the first half of the frequency buffer (up to Nyquist)
        let expected_bins = 2048 / 2 + 1; // 1025 for a 2048-sample FFT

        let spectrogram_data =
            spectral::spectrum_to_magnitudes(&frame.frequency_buffer[..expected_bins]);

        // 1. Run the Scout Engine to determine rough frequency neighborhood
        let f_scout = scout::process_scout(&frame.frequency_buffer[..expected_bins]);

        // 2. Route to Bass Engine or Treble Engine based on Scout peak
        // Note: For now, both rely on the 2048-sample frame until decimation is added for Bass
        let frame_size = 2048;
        let audio_frame = &frame.audio_buffer[..frame_size];

        if f_scout < 150.0 {
            // Route to Bass Engine — uses full 8192-sample audio buffer for decimation
            match self.bass_algorithm {
                BassAlgorithm::DPYIN => dpyin::detect_pitch_dpyin(
                    &frame.audio_buffer[..],
                    self.sample_rate,
                    amplitude_threshold,
                    &mut frame.time_buffer[..],
                ),
            }
        } else {
            // Route to Treble Engine
            match self.treble_algorithm {
                TrebleAlgorithm::QIFFT => {
                    if let Some((freq, conf)) = pitch::detect_pitch_pyin(
                        audio_frame,
                        self.sample_rate,
                        amplitude_threshold,
                        &mut frame.time_buffer[..],
                    ) {
                        let refined_freq =
                            pitch::refine_from_spectrum(&spectrogram_data, freq, self.sample_rate);
                        // refine_from_spectrum can return None or original. If it returns Some(freq), use it
                        refined_freq.map(|f| (f, Some(conf)))
                    } else {
                        None
                    }
                }
                TrebleAlgorithm::DPLL => {
                    // DPLL is stubbed out for future implementation
                    None
                }
                TrebleAlgorithm::PVOCODER => {
                    // PVOCODER is stubbed out for future implementation
                    None
                }
            }
        }
    }
}
