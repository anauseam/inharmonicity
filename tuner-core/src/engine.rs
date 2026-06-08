//! # Engine (Thread 2) — Fundamental Frequency Detection
//!
//! The "Brains" of the pipeline. The Engine is orchestrated by the `AudioPipeline`
//! after the signal has been validated by the Gatekeeper. Its sole responsibility
//! is to process the signal and extract the exact fundamental frequency.

use crate::algorithms::{
    peaks::{self, SpectralPeak, extract_peaks},
    spectral::goertzel,
    twm,
};
use crate::audio::BASS_WINDOW_SIZE;
use crate::models::{NOTES, get_expected_beta};
use crate::pipeline::ProcessingFrame;

pub const MAX_PARTIALS: usize = 128;

/// Precomputed per-key data.
#[derive(Debug, Clone)]
pub struct KeyProfile {
    pub f0_et: f32,
    pub beta: f32,
    pub predicted_partials: [f32; MAX_PARTIALS],
    pub valid_partial_count: usize,
}

impl KeyProfile {
    pub fn new(f0_et: f32, beta: f32) -> Self {
        let mut predicted_partials = [0.0; MAX_PARTIALS];
        let mut valid_partial_count = 0;

        for n in 1..=MAX_PARTIALS {
            let n_f32 = n as f32;
            let f_n = n_f32 * f0_et * (1.0 + beta * n_f32 * n_f32).sqrt();
            if f_n < 22050.0 {
                predicted_partials[n - 1] = f_n;
                valid_partial_count += 1;
            } else {
                break;
            }
        }

        Self {
            f0_et,
            beta,
            predicted_partials,
            valid_partial_count,
        }
    }
}

/// Result of a successful pitch detection frame.
#[derive(Debug, Clone)]
pub struct PitchResult {
    /// 0–87 key index of the identified note.
    pub key_index: u8,
    /// MVUE-combined cents deviation (weighted average across all live partials).
    pub cents_deviation: f32,
    /// Absolute physical fundamental frequency (Hz), derived from cents_deviation.
    pub measured_f0: f32,
    /// Per-partial instantaneous frequency (Hz). Valid entries: [0..partial_count].
    pub partial_freqs: [f32; MAX_PARTIALS],
    /// Per-partial cents deviation relative to tuning curve target.
    pub partial_cents: [f32; MAX_PARTIALS],
    /// Harmonic index (n) for each live partial.
    pub partial_ns: [u32; MAX_PARTIALS],
    /// Per-partial amplitude from Goertzel (used as weight by consumer if desired).
    pub partial_amplitudes: [f32; MAX_PARTIALS],
    /// Number of live (non-ghost) partials contributing to this frame.
    pub partial_count: usize,
}

impl Default for PitchResult {
    fn default() -> Self {
        Self {
            key_index: 0,
            cents_deviation: 0.0,
            measured_f0: 0.0,
            partial_freqs: [0.0; MAX_PARTIALS],
            partial_cents: [0.0; MAX_PARTIALS],
            partial_ns: [0; MAX_PARTIALS],
            partial_amplitudes: [0.0; MAX_PARTIALS],
            partial_count: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct PartialTracker {
    prev_phase: f32,
    prev_f_inst: f32,
    phase_var_ema: f32,
}

/// The Fundamental Frequency ($f_0$) Engine.
pub struct Engine {
    pub sample_rate: u32,

    // Discovery State
    pub identified_key: Option<u8>,
    pub consistency_key: u8,
    pub stable_frames: u8,

    // Tracking state
    tracking_targets: [f32; MAX_PARTIALS],
    partial_trackers: [PartialTracker; MAX_PARTIALS],
    warmup_hops: u8,

    // Shared
    pub noise_floor: f32,
    profiles: Box<[KeyProfile; 88]>,
    peak_scratch: Box<[SpectralPeak]>,
}

fn hz_to_cents(freq: f32, reference: f32) -> f32 {
    1200.0 * (freq / reference).log2()
}

impl Engine {
    /// Creates a new Engine with default algorithms.
    pub fn new(sample_rate: u32) -> Self {
        let mut profiles_vec = Vec::with_capacity(88);
        for i in 0..88 {
            let note = &NOTES[i];
            let beta = get_expected_beta(i as u8);
            profiles_vec.push(KeyProfile::new(note.frequency, beta));
        }

        let profiles_array: [KeyProfile; 88] = profiles_vec.try_into().unwrap();

        Engine {
            sample_rate,
            identified_key: None,
            consistency_key: 0,
            stable_frames: 0,
            tracking_targets: [0.0; MAX_PARTIALS],
            partial_trackers: [PartialTracker::default(); MAX_PARTIALS],
            warmup_hops: 0,
            noise_floor: 0.0, // updated by pipeline
            profiles: Box::new(profiles_array),
            peak_scratch: vec![SpectralPeak::default(); 64].into_boxed_slice(),
        }
    }

    /// Executes the primary DSP detection loop for a single frame.
    pub fn process(
        &mut self,
        frame: &mut ProcessingFrame,
        is_silence: bool,
        is_new_onset: bool,
        is_transient_bypass: bool,
        target_note: Option<u8>,
    ) -> Option<PitchResult> {
        if is_silence {
            self.stable_frames = 0;
            self.identified_key = None;
            self.partial_trackers = [PartialTracker::default(); MAX_PARTIALS];
            return None;
        }

        if is_transient_bypass {
            self.stable_frames = 0;
            self.identified_key = None;
            return None;
        }

        if is_new_onset {
            self.stable_frames = 0;
            self.identified_key = None;
        }

        // Force re-evaluation for instant UI response
        if let Some(target_idx) = target_note
            && self.identified_key.is_some()
            && self.identified_key != Some(target_idx)
        {
            self.identified_key = None;
        }

        let mag_count_bass = BASS_WINDOW_SIZE / 2;
        let bass_magnitudes = &frame.bass_magnitude_buffer[..mag_count_bass];

        // ── Discovery State ──
        if self.identified_key.is_none() {
            // ── Gaussian Noise Filter (Detection Theory) ─────────────────────
            // Computes a Neyman-Pearson threshold for Additive White Gaussian Noise.
            // Foundation: Kay, S. M. (1998) Fundamentals of Statistical Signal Processing.
            //
            // Kay defines the false-alarm rate for a power threshold (gamma') as:
            // P_fa = exp(-gamma' / sigma^2) (eq. 7.26, ISBN: 0-13-504135-X)
            //
            // For efficiency, we target a magnitude threshold (T) where gamma' = T^2.
            // The total bin power variance (sigma^2) is pre-calculated here as `p_bin`
            // using the unnormalized FFT Hann window energy (sum_w2 = 0.375 * N).
            //
            // Substituting these yields: P_fa = exp(-T^2 / p_bin)
            // Solving for T gives:       T = sqrt(-p_bin * ln(P_fa))
            //
            // Here we target a 0.1% false-alarm rate (P_fa = 0.001).
            let sum_w2 = 0.375 * BASS_WINDOW_SIZE as f32;
            let p_bin = self.noise_floor * self.noise_floor * sum_w2;
            let min_magnitude = if p_bin > 0.0 {
                (-p_bin * 0.001_f32.ln()).sqrt()
            } else {
                0.0
            };

            let count = extract_peaks(
                bass_magnitudes,
                &frame.bass_frequency_buffer[..],
                self.sample_rate,
                BASS_WINDOW_SIZE,
                min_magnitude,
                &mut self.peak_scratch,
            );

            let k = count.min(64);
            let active_peaks = &mut self.peak_scratch[..k];

            // 1. Peak Masking (Gómez 2006 / Cano 1998)
            let valid_count = peaks::mask_peaks(active_peaks);
            let active_peaks = &mut active_peaks[..valid_count];

            // 2. Safe Bypass Gate
            let (winning_key, temporal_gate, min_error) = if let Some(target_idx) = target_note {
                (target_idx, true, 0.0) // Bypass 88-key TWM array
            } else {
                // Auto Mode: TWM Error Scoring
                let mut min_error = f32::MAX;
                let mut winning_key = 0;
                for k in 0..88 {
                    let err = twm::score_candidate(active_peaks, &self.profiles[k]);
                    if err < min_error {
                        min_error = err;
                        winning_key = k as u8;
                    }
                }

                // 3-Frame Consistency Gate
                if winning_key == self.consistency_key {
                    self.stable_frames += 1;
                } else {
                    self.consistency_key = winning_key;
                    self.stable_frames = 1;
                }

                (winning_key, self.stable_frames >= 3, min_error)
            };

            let profile = &self.profiles[winning_key as usize];

            #[cfg(debug_assertions)]
            eprintln!(
                "[ENGINE] Consistency Gate: peaks={}, key_idx={}, f0={:.1}, min_error={:.2}",
                valid_count, winning_key, profile.f0_et, min_error
            );

            if temporal_gate {
                // Lock
                #[cfg(debug_assertions)]
                eprintln!("[ENGINE] *** LOCK ACQUIRED *** -> key_idx={}", winning_key);
                self.identified_key = Some(winning_key);
                self.warmup_hops = 0;

                let limit = profile.valid_partial_count.min(MAX_PARTIALS);
                for i in 0..limit {
                    let predicted = profile.predicted_partials[i];
                    let tol = predicted * 0.03;

                    let nearest_obs = self.peak_scratch[..count].iter().min_by(|a, b| {
                        (a.frequency - predicted)
                            .abs()
                            .partial_cmp(&(b.frequency - predicted).abs())
                            .unwrap()
                    });

                    self.tracking_targets[i] = match nearest_obs {
                        Some(p) if (p.frequency - predicted).abs() < tol => p.frequency,
                        _ => predicted,
                    };

                    self.partial_trackers[i].prev_phase = 0.0;
                    self.partial_trackers[i].prev_f_inst = self.tracking_targets[i];
                    self.partial_trackers[i].phase_var_ema = 0.0;
                }
            } else {
                return None;
            }
        }

        // ── Tracking State ──
        let key = self.identified_key?;
        let profile = &self.profiles[key as usize];
        let audio_slice = &frame.audio_buffer[7168..8192];
        let t_hop = 1024.0 / self.sample_rate as f32;

        let mut weight_sum = 0.0;
        let mut cents_sum = 0.0;
        let mut live_partials = 0;
        let mut result = PitchResult {
            key_index: key,
            ..Default::default()
        };

        const ALPHA: f32 = 0.1;
        let fs = self.sample_rate as f32;
        let c_crlb_geometric = 6.0 * fs * fs
            / (core::f32::consts::PI * core::f32::consts::PI * 1024.0 * 1024.0 * 1024.0);

        for i in 0..profile.valid_partial_count.min(MAX_PARTIALS) {
            let f_target = self.tracking_targets[i];
            let (amplitude, phase_current) = goertzel(audio_slice, self.sample_rate, f_target);
            let tracker = &mut self.partial_trackers[i];

            if self.warmup_hops == 0 {
                tracker.prev_phase = phase_current;
                continue;
            }

            // ── Phase Vocoder / Sinusoidal Tracking ──
            // Foundation: McAulay, R. J., & Quatieri, T. F. (1986). Speech analysis/synthesis based
            // on a sinusoidal representation. IEEE Transactions on Acoustics, Speech, and Signal Processing.
            //
            // Computes instantaneous frequency by unwrapping the phase derivative relative to the static f_target:
            // Δφ = (φ_n - φ_{n-1} - 2π f_target t_hop) mod 2π
            // f_inst = f_target + Δφ / (2π t_hop)
            let expected_advance = 2.0 * core::f32::consts::PI * f_target * t_hop;
            let phase_diff = phase_current - tracker.prev_phase - expected_advance;
            let delta_phi = (phase_diff + core::f32::consts::PI)
                .rem_euclid(2.0 * core::f32::consts::PI)
                - core::f32::consts::PI;

            let f_inst = f_target + delta_phi / (2.0 * core::f32::consts::PI * t_hop);
            tracker.prev_phase = phase_current;

            if self.warmup_hops == 1 {
                tracker.prev_f_inst = f_inst;
                continue;
            }

            let delta_f_inst = f_inst - tracker.prev_f_inst;
            tracker.phase_var_ema =
                ALPHA * delta_f_inst * delta_f_inst + (1.0 - ALPHA) * tracker.phase_var_ema;
            tracker.prev_f_inst = f_inst;

            // ── MVUE Frequency Estimation ──
            // Calculates the Minimum Variance Unbiased Estimator (MVUE) for the true fundamental.
            // Foundation: Kay, S. M. (1993). Fundamentals of Statistical Signal Processing: Estimation Theory.
            //
            // The Cramer-Rao Lower Bound (CRLB) for the variance of a sinusoidal frequency estimate in AWGN is:
            // var(f) >= 12 / ((2*pi)^2 * SNR * N(N^2-1))
            // Because SNR is proportional to amplitude^2, optimal MVUE weighting combines partials
            // using W_i = A_i^2. The missing fundamental tracks noise, exceeds the CRLB variance
            // threshold, and is assigned a weight of 0.0, cleanly erasing it from the final pitch.
            let expected_var = c_crlb_geometric * (self.noise_floor * self.noise_floor)
                / (amplitude * amplitude + 1e-12);

            let weight = if tracker.phase_var_ema > 3.0 * expected_var {
                0.0
            } else {
                amplitude * amplitude
            };

            if weight > 0.0 {
                result.partial_freqs[live_partials] = f_inst;
                result.partial_amplitudes[live_partials] = amplitude;

                let tuning_curve_target = profile.predicted_partials[i];
                let cents_i = hz_to_cents(f_inst, tuning_curve_target);
                result.partial_cents[live_partials] = cents_i;
                result.partial_ns[live_partials] = (i + 1) as u32;

                weight_sum += weight;
                cents_sum += weight * cents_i;
                live_partials += 1;
            }
        }

        if self.warmup_hops == 0 {
            self.warmup_hops = 1;
            return None;
        } else if self.warmup_hops == 1 {
            self.warmup_hops = 2;
            return None; // No f_inst diff variance available yet
        }

        if live_partials < 2 {
            self.identified_key = None;
            return None;
        }

        result.cents_deviation = if weight_sum > 0.0 {
            cents_sum / weight_sum
        } else {
            0.0
        };

        // ── Mathematical Proof of f0 Reconstruction ──
        // Let f_n be the physical frequency of partial n.
        // The expected tuning curve is: f_{n, curve} = n * f_{0, et} * sqrt(1 + B * n^2).
        // The cents deviation is: c_n = 1200 * log2(f_n / f_{n, curve}).
        // Substituting the uniform global deviation C back into the ET fundamental gives:
        // f_{0, measured} = f_{0, et} * 2^(C / 1200)
        //                 = f_{0, et} * (f_n / f_{n, curve})
        //                 = f_{0, et} * (f_n / (n * f_{0, et} * sqrt(1 + B * n^2)))
        //                 = f_n / (n * sqrt(1 + B * n^2))
        // This is the EXACT algebraic inverse mapping for extracting the fundamental from an
        // inharmonic partial! Doing it logarithmically via cents explicitly factors out f_{0, et}
        // and yields the pure physical fundamental without any division or square roots in the hot path.
        result.measured_f0 = profile.f0_et * 2.0_f32.powf(result.cents_deviation / 1200.0);
        result.partial_count = live_partials;

        Some(result)
    }
}
