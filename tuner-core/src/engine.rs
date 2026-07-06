//! # Engine (Thread 2) — Fundamental Frequency Detection
//!
//! The "Brains" of the pipeline. The Engine is orchestrated by the `AudioPipeline`
//! after the signal has been validated by the Gatekeeper. Its sole responsibility
//! is to process the signal and extract the exact fundamental frequency.

use crate::algorithms::{
    discovery,
    peaks::{self, SpectralPeak, extract_peaks},
    spectral::goertzel,
    twm,
};
use crate::audio::{BASS_WINDOW_SIZE, HOP_SIZE};
use crate::models::{KeyProfile, MAX_PARTIALS};
use crate::pipeline::ProcessingFrame;

// ── Neyman-Pearson Amplitude SNR Gate ──
// Foundation: Kay, S. M. (1998). Fundamentals of Statistical Signal Processing: Detection Theory (Vol 2), Chapter 9.
// For a target false-alarm probability P_fa = 0.001 (0.1%), the threshold is derived from the Rayleigh tail.
// Scaled for physical amplitude (Hann window energy = 0.375 * HOP_SIZE):
// T_amp = σ * (4/HOP_SIZE) * sqrt(-(0.375 * HOP_SIZE) * ln(0.001))
const NEYMAN_PEARSON_K: f32 = 0.201184;

/// Result of a successful pitch detection frame.
#[derive(Debug, Clone)]
pub struct PitchResult {
    /// 0–87 key index of the identified note.
    pub key_index: u8,
    /// Physical fundamental frequency (Partial 1), tracked via Goertzel phase vocoder. Returns None if Partial 1 is dead.
    pub cents_deviation: Option<f32>,
    /// Absolute physical fundamental frequency (Partial 1) in Hz. Returns None if Partial 1 is dead.
    pub measured_f0: Option<f32>,
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
    #[cfg(feature = "telemetry")]
    pub telemetry_count: usize,
    #[cfg(feature = "telemetry")]
    pub partial_targets: [f32; MAX_PARTIALS],
    #[cfg(feature = "telemetry")]
    pub partial_t_amps: [f32; MAX_PARTIALS],
    #[cfg(feature = "telemetry")]
    pub partial_is_alive: [bool; MAX_PARTIALS],
    /// Stage B winning scale of the current lock, in cents (0.0 = locked at ET).
    #[cfg(feature = "telemetry")]
    pub s_win_cents: f32,
}

impl Default for PitchResult {
    fn default() -> Self {
        Self {
            key_index: 0,
            cents_deviation: None,
            measured_f0: None,
            partial_freqs: [0.0; MAX_PARTIALS],
            partial_cents: [0.0; MAX_PARTIALS],
            partial_ns: [0; MAX_PARTIALS],
            partial_amplitudes: [0.0; MAX_PARTIALS],
            partial_count: 0,
            #[cfg(feature = "telemetry")]
            telemetry_count: 0,
            #[cfg(feature = "telemetry")]
            partial_targets: [0.0; MAX_PARTIALS],
            #[cfg(feature = "telemetry")]
            partial_t_amps: [0.0; MAX_PARTIALS],
            #[cfg(feature = "telemetry")]
            partial_is_alive: [false; MAX_PARTIALS],
            #[cfg(feature = "telemetry")]
            s_win_cents: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct PartialTracker {
    prev_phase: f32,
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
    /// Winning Stage B scale (s_win) of the current lock; 1.0 when unlocked.
    locked_scale: f32,

    // Shared
    pub noise_floor: f32,
    peak_scratch: Box<[SpectralPeak]>,
}

fn hz_to_cents(freq: f32, reference: f32) -> f32 {
    1200.0 * (freq / reference).log2()
}

impl Engine {
    /// Creates a new Engine with default algorithms.
    pub fn new(sample_rate: u32) -> Self {
        Engine {
            sample_rate,
            identified_key: None,
            consistency_key: 0,
            stable_frames: 0,
            tracking_targets: [0.0; MAX_PARTIALS],
            partial_trackers: [PartialTracker::default(); MAX_PARTIALS],
            warmup_hops: 0,
            locked_scale: 1.0,
            noise_floor: 0.0, // updated by pipeline
            peak_scratch: vec![SpectralPeak::default(); 64].into_boxed_slice(),
        }
    }

    /// Executes the primary DSP detection loop for a single frame.
    ///
    /// `profiles` is the read-only per-key template table to score against.
    pub fn process(
        &mut self,
        frame: &mut ProcessingFrame,
        profiles: &[KeyProfile; 88],
        is_silence: bool,
        is_new_onset: bool,
        is_transient_bypass: bool,
        target_note: Option<u8>,
    ) -> Option<PitchResult> {
        if is_silence {
            self.stable_frames = 0;
            self.identified_key = None;
            self.partial_trackers = [PartialTracker::default(); MAX_PARTIALS];
            self.locked_scale = 1.0;
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
            let cfg = twm::TwmConfig::default();
            // `min_error` feeds only the debug_assertions diagnostic below.
            #[cfg_attr(not(debug_assertions), allow(unused_variables))]
            let (winning_key, temporal_gate, s_win, min_error) =
                if let Some(target_idx) = target_note {
                    // Manual Mode: bypass the 88-key scan, but still run Stage B scale
                    // refinement on the single target profile — otherwise this is the
                    // worst-seeded path (pure ET), and it is the critical one for
                    // Pitch Raise on heavily mistuned strings.
                    let (s, err) = discovery::refine_scale(
                        active_peaks,
                        &profiles[target_idx as usize],
                        &cfg,
                    );
                    (target_idx, true, s, err)
                } else {
                    // Auto Mode: split discovery (ADR 0005) — Stage A discrete 88-key
                    // scan, Stage B basin-clamped scale refinement of the top-3.
                    let res = discovery::discover(active_peaks, profiles, &cfg, true);

                    // 3-Frame Consistency Gate (key-based, unchanged)
                    if res.key_index == self.consistency_key {
                        self.stable_frames += 1;
                    } else {
                        self.consistency_key = res.key_index;
                        self.stable_frames = 1;
                    }

                    (res.key_index, self.stable_frames >= 3, res.scale, res.error)
                };

            let profile = &profiles[winning_key as usize];

            #[cfg(debug_assertions)]
            eprintln!(
                "[ENGINE] Consistency Gate: peaks={}, key_idx={}, f0={:.1}, min_error={:.2}",
                valid_count, winning_key, profile.f0_et, min_error
            );

            if temporal_gate {
                // Lock
                #[cfg(debug_assertions)]
                eprintln!(
                    "[ENGINE] *** LOCK ACQUIRED *** -> key_idx={}, s_win={:+.1}c",
                    winning_key,
                    1200.0 * s_win.log2()
                );
                self.identified_key = Some(winning_key);
                self.warmup_hops = 0;
                self.locked_scale = s_win;

                let limit = profile.valid_partial_count.min(MAX_PARTIALS);
                for i in 0..limit {
                    // Seed the Goertzel trackers from the REFINED series, not ET:
                    // an ET seed for partial n of a mistuned note is off by
                    // δ·n·f0, which exceeds the ±21.5 Hz phase-unwrap range at
                    // the 1024-sample hop for high partials.
                    self.tracking_targets[i] = profile.predicted_partials[i] * s_win;

                    self.partial_trackers[i].prev_phase = 0.0;
                }
            } else {
                return None;
            }
        }

        // ── Tracking State ──
        let key = self.identified_key?;
        let profile = &profiles[key as usize];
        let audio_slice = &frame.audio_buffer[(BASS_WINDOW_SIZE - HOP_SIZE)..BASS_WINDOW_SIZE];
        let t_hop = HOP_SIZE as f32 / self.sample_rate as f32;

        let mut live_partials = 0;
        let mut result = PitchResult {
            key_index: key,
            ..Default::default()
        };

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

            let t_amp = self.noise_floor * NEYMAN_PEARSON_K;

            let weight = if amplitude < t_amp {
                0.0
            } else {
                amplitude * amplitude
            };

            if weight > 0.0 {
                // ── Adaptive Tracking Seed (Phase Vocoder Feedback) ──
                // Foundation: Dolson, M. (1986). The Phase Vocoder: A Tutorial. Computer Music Journal.
                // Slowly adapts the Goertzel evaluation center toward the measured physical frequency.
                // We only adapt when the signal survives the SNR gate, ensuring we track the physical
                // string and not phase-unwrapping noise.
                self.tracking_targets[i] = 0.95 * self.tracking_targets[i] + 0.05 * f_inst;

                result.partial_freqs[live_partials] = f_inst;
                result.partial_amplitudes[live_partials] = amplitude;

                let tuning_curve_target = profile.predicted_partials[i];
                let cents_i = hz_to_cents(f_inst, tuning_curve_target);
                result.partial_cents[live_partials] = cents_i;
                result.partial_ns[live_partials] = (i + 1) as u32;

                live_partials += 1;
            }

            #[cfg(feature = "telemetry")]
            {
                result.partial_targets[i] = self.tracking_targets[i];
                result.partial_amplitudes[i] = amplitude;
                result.partial_t_amps[i] = t_amp;
                result.partial_is_alive[i] = weight > 0.0;
            }
        }

        #[cfg(feature = "telemetry")]
        {
            result.telemetry_count = profile.valid_partial_count;
            result.s_win_cents = 1200.0 * self.locked_scale.log2();
        }

        if self.warmup_hops == 0 {
            self.warmup_hops = 1;
            return None; // Need one hop to calculate the first phase derivative
        }

        if live_partials < 1 {
            return None;
        }

        // ── Global Pitch Reconstruction (Partial 1 Only) ──
        // We deliberately use Partial 1 exclusively to drive the Cent Meter.
        // If the stored inharmonicity profile (B_profile) doesn't perfectly match the physical string
        // (B_true) due to uncalibrated models or string aging, higher partials will carry an n^2
        // systematic cents error. Partial 1 carries no B correction (n=1) and is immune to this inaccuracy.
        // For extreme bass notes where Partial 1 has no acoustic energy, this correctly returns None,
        // and the tuner naturally falls back to the strobe display.
        for i in 0..live_partials {
            if result.partial_ns[i] == 1 {
                result.cents_deviation = Some(result.partial_cents[i]);
                result.measured_f0 = Some(result.partial_freqs[i]);
                break;
            }
        }

        result.partial_count = live_partials;

        Some(result)
    }
}
