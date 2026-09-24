//! # Engine — fundamental-frequency detection
//!
//! Names the key a validated signal belongs to, locks onto it, and tracks the
//! frequencies of its partials.

use crate::algorithms::{
    discovery,
    peaks::{self, extract_peaks},
    spectral, twm,
};
use crate::audio::{BASS_WINDOW_SIZE, HOP_SIZE};
use crate::models::{KeyProfile, MAX_PARTIALS, SpectralPeak};
use crate::pipeline::ProcessingFrame;

// Binary integration (Schwartz 1956, IRE Trans. IT 2(4); Shnidman 1998, IEEE
// Trans. AES 34(3)): lock the first key to win ≥ M of the last N Stable-frame
// scans. M > N/2, so at most one key can hold M votes. (7, 8) costs ≈ 93 ms of
// latency against (5, 6)'s ≈ 46 ms, for its accuracy.
// report 0010
const LOCK_VOTES_M: usize = 7;
const LOCK_WINDOW_N: usize = 8;

/// Rate at which each partial's Goertzel centre follows its measured
/// instantaneous frequency (Dolson 1986, Computer Music Journal), so the tracker
/// stays on a detuned string.
// Ours and unswept, and bounded rather than confirmed: α = 1 at N = 4096 makes the
// loop unstable (|z| = √2), and the τ ≈ 0.46 s this gives is outrun by a turning
// peg (152–387 ¢ of aliasing at 200–400 ¢/s), which the coarse readout covers.
// report 0011
const TRACKER_SEED_ALPHA: f32 = 0.05;

/// Result of a successful pitch detection frame.
#[derive(Debug, Clone)]
pub struct PitchResult {
    /// 0–87 key index of the identified note.
    pub key_index: u8,
    /// Partial 1's frequency in Hz; `None` when partial 1 is dead.
    pub measured_f0: Option<f32>,
    /// Per-partial instantaneous frequency (Hz). Valid entries: `[0..partial_count]`.
    pub partial_freqs: [f32; MAX_PARTIALS],
    /// Per-partial cents from the key's discovery template.
    pub partial_cents: [f32; MAX_PARTIALS],
    /// Harmonic index (n) for each live partial.
    pub partial_ns: [u32; MAX_PARTIALS],
    /// Per-partial Goertzel amplitude.
    pub partial_amplitudes: [f32; MAX_PARTIALS],
    /// Live partials this frame.
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

/// The fundamental-frequency engine: discovery, the M-of-N lock, and partial
/// tracking.
pub struct Engine {
    pub sample_rate: u32,
    pub identified_key: Option<u8>,
    /// Ring buffer of the last `LOCK_WINDOW_N` Stable-frame discovery winners,
    /// with its fill length and write cursor.
    lock_window: [u8; LOCK_WINDOW_N],
    lock_window_len: usize,
    lock_window_head: usize,
    tracking_targets: [f32; MAX_PARTIALS],
    partial_trackers: [PartialTracker; MAX_PARTIALS],
    warmup_hops: u8,
    /// Winning Stage B scale (s_win) of the current lock; 1.0 when unlocked.
    locked_scale: f32,
    /// The locked key's partial spacing (≈ f₀, proxied by the f₁ seed) is inside
    /// the 1024-sample Hann main lobe (half-width 2·fs/1024 ≈ 86 Hz), so every
    /// partial uses the 4096-sample Goertzel window. The hop, and with it the
    /// unwrap range, is unchanged. The strobe bank applies the same rule.
    long_window: bool,
    /// The silence threshold, standing in for the noise σ of the peak floor and
    /// the partial gate.
    pub noise_floor: f32,
    peak_scratch: Box<[SpectralPeak]>,
}

fn hz_to_cents(freq: f32, reference: f32) -> f32 {
    1200.0 * (freq / reference).log2()
}

impl Engine {
    /// An unlocked engine.
    pub fn new(sample_rate: u32) -> Self {
        Engine {
            sample_rate,
            identified_key: None,
            lock_window: [0; LOCK_WINDOW_N],
            lock_window_len: 0,
            lock_window_head: 0,
            tracking_targets: [0.0; MAX_PARTIALS],
            partial_trackers: [PartialTracker::default(); MAX_PARTIALS],
            warmup_hops: 0,
            locked_scale: 1.0,
            long_window: false,
            noise_floor: 0.0,
            peak_scratch: vec![SpectralPeak::default(); 64].into_boxed_slice(),
        }
    }

    /// Records a Stable-frame discovery winner and returns whether `key` now holds
    /// `LOCK_VOTES_M` of the last `LOCK_WINDOW_N` votes. A partly filled window
    /// locks after M straight votes, and M > N/2 makes testing the recorded key
    /// alone exact.
    fn record_stable_winner(&mut self, key: u8) -> bool {
        self.lock_window[self.lock_window_head] = key;
        self.lock_window_head = (self.lock_window_head + 1) % LOCK_WINDOW_N;
        if self.lock_window_len < LOCK_WINDOW_N {
            self.lock_window_len += 1;
        }
        // Until the buffer is full the head moves with the fill length, so the
        // valid votes are the first `len` slots.
        let votes = self.lock_window[..self.lock_window_len]
            .iter()
            .filter(|&&k| k == key)
            .count();
        votes >= LOCK_VOTES_M
    }

    /// Clears the M-of-N lock window. Only the fill length and cursor are reset;
    /// slots past `len` are never read.
    #[inline]
    fn reset_lock_window(&mut self) {
        self.lock_window_len = 0;
        self.lock_window_head = 0;
    }

    /// Runs one frame: discovery until a key locks, then partial tracking.
    /// `profiles` is the per-key template table discovery scores against.
    // Gate state arrives as booleans rather than a `GateResult`, so the engine
    // does not depend on the gatekeeper.
    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        frame: &ProcessingFrame,
        profiles: &[KeyProfile; 88],
        is_silence: bool,
        is_stable: bool,
        is_new_onset: bool,
        is_transient_bypass: bool,
        target_note: Option<u8>,
    ) -> Option<PitchResult> {
        if is_silence {
            self.reset_lock_window();
            self.identified_key = None;
            self.partial_trackers = [PartialTracker::default(); MAX_PARTIALS];
            self.locked_scale = 1.0;
            return None;
        }

        if is_transient_bypass {
            self.reset_lock_window();
            self.identified_key = None;
            return None;
        }

        if is_new_onset {
            self.reset_lock_window();
            self.identified_key = None;
        }

        // A new manual target drops the current lock.
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
            // Auto mode votes on Stable frames only, and a non-Stable frame leaves
            // the window intact. A manual target locks at once.
            if target_note.is_none() && !is_stable {
                return None;
            }

            // Neyman–Pearson peak floor for white Gaussian noise (Kay 1998,
            // Fundamentals of Statistical Signal Processing, eq. 7.26):
            // P_fa = exp(−T²/p_bin), so T = √(−p_bin·ln P_fa), with p_bin = σ²·Σw² for
            // the unnormalised Hann window (Σw² = 0.375·N), at P_fa = 0.001.
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

            // 2. Discovery
            let cfg = twm::TwmConfig::default();
            // `min_error` feeds only the debug_assertions diagnostic below.
            #[cfg_attr(not(debug_assertions), allow(unused_variables))]
            let (winning_key, lock_acquired, s_win, min_error) =
                if let Some(target_idx) = target_note {
                    // Manual: no 88-key scan, but the scale is still refined, or a
                    // mistuned string would be seeded at pure ET.
                    let (s, err) =
                        discovery::refine_scale(active_peaks, &profiles[target_idx as usize], &cfg);
                    (target_idx, true, s, err)
                } else {
                    let res = discovery::discover(active_peaks, profiles, &cfg, true);

                    // A key that locks is this frame's winner (M > N/2), so the
                    // scale and error are this scan's.
                    let locked = self.record_stable_winner(res.key_index);

                    (res.key_index, locked, res.scale, res.error)
                };

            let profile = &profiles[winning_key as usize];

            #[cfg(debug_assertions)]
            eprintln!(
                "[ENGINE] Discovery Gate: peaks={}, key_idx={}, f0={:.1}, min_error={:.2}",
                valid_count, winning_key, profile.f0_et, min_error
            );

            if lock_acquired {
                #[cfg(debug_assertions)]
                eprintln!(
                    "[ENGINE] *** LOCK ACQUIRED *** -> key_idx={}, s_win={:+.1}c",
                    winning_key,
                    1200.0 * s_win.log2()
                );
                self.identified_key = Some(winning_key);
                self.warmup_hops = 0;
                self.locked_scale = s_win;
                // Inside the main lobe, the neighbouring partial beats against
                // the tracked one.
                self.long_window =
                    profile.predicted_partials[0] * s_win * 1024.0 < 2.0 * self.sample_rate as f32;

                let limit = profile.valid_partial_count.min(MAX_PARTIALS);
                for i in 0..limit {
                    // Seeded from the refined series, not ET: an ET seed for
                    // partial n of a mistuned note is off by δ·n·f₀, past the
                    // unwrap range for high partials.
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
        // The evaluator reads the freshest window of the buffer; the unwrap
        // range follows the hop, not the window.
        let audio_slice = &frame.audio_buffer[..BASS_WINDOW_SIZE];
        let (eval, np_k): (spectral::GoertzelFn, f32) = if self.long_window {
            (spectral::goertzel_bass, spectral::neyman_pearson_k(4096))
        } else {
            (spectral::goertzel, spectral::neyman_pearson_k(1024))
        };
        let t_hop = HOP_SIZE as f32 / self.sample_rate as f32;

        let mut live_partials = 0;
        let mut result = PitchResult {
            key_index: key,
            ..Default::default()
        };

        for i in 0..profile.valid_partial_count.min(MAX_PARTIALS) {
            let f_target = self.tracking_targets[i];
            let (amplitude, phase_current) = eval(audio_slice, self.sample_rate, f_target);
            let tracker = &mut self.partial_trackers[i];

            if self.warmup_hops == 0 {
                tracker.prev_phase = phase_current;
                continue;
            }

            // Instantaneous frequency from the unwrapped phase advance (McAulay &
            // Quatieri 1986, IEEE Trans. ASSP 34(4)):
            // Δφ = (φₙ − φₙ₋₁ − 2π·f_target·t_hop) mod 2π, f_inst = f_target + Δφ/(2π·t_hop).
            let expected_advance = 2.0 * core::f32::consts::PI * f_target * t_hop;
            let phase_diff = phase_current - tracker.prev_phase - expected_advance;
            let delta_phi = (phase_diff + core::f32::consts::PI)
                .rem_euclid(2.0 * core::f32::consts::PI)
                - core::f32::consts::PI;

            let f_inst = f_target + delta_phi / (2.0 * core::f32::consts::PI * t_hop);
            tracker.prev_phase = phase_current;

            // Kay 1998 Neyman–Pearson amplitude gate at the active window length.
            let t_amp = self.noise_floor * np_k;

            // A partial has a positive, finite frequency. On a spurious deep-bass
            // lock the adaptive target can walk toward DC until f_inst goes
            // negative, which `hz_to_cents` would publish as NaN; weight 0 drops it
            // and stops the walk.
            let weight = if amplitude < t_amp || !(f_inst.is_finite() && f_inst > 0.0) {
                0.0
            } else {
                amplitude * amplitude
            };

            if weight > 0.0 {
                // Adapt only on partials that survived the SNR gate, so the seed
                // follows the physical string and not phase-unwrapping noise.
                self.tracking_targets[i] = (1.0 - TRACKER_SEED_ALPHA) * self.tracking_targets[i]
                    + TRACKER_SEED_ALPHA * f_inst;

                result.partial_freqs[live_partials] = f_inst;
                result.partial_amplitudes[live_partials] = amplitude;

                let template_hz = profile.predicted_partials[i];
                let cents_i = hz_to_cents(f_inst, template_hz);
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

        // The pitch is partial 1's alone: a template B that misses the string's
        // biases higher partials in proportion to n², while n = 1 carries no B.
        for i in 0..live_partials {
            if result.partial_ns[i] == 1 {
                result.measured_f0 = Some(result.partial_freqs[i]);
                break;
            }
        }

        result.partial_count = live_partials;

        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    // The reference M-of-N rule: a port of `eval_rule` in `replay_lock_rules.py`,
    // the harness report 0010 was measured with. Returns `(key, index)` of the
    // first lock, or `None`.
    fn ref_eval(seq: &[u8], m: usize, n: usize) -> Option<(u8, usize)> {
        let mut win: VecDeque<u8> = VecDeque::with_capacity(n);
        for (t, &w) in seq.iter().enumerate() {
            if win.len() == n {
                win.pop_front();
            }
            win.push_back(w);
            if win.iter().filter(|&&k| k == w).count() >= m {
                return Some((w, t));
            }
        }
        None
    }

    /// Feed a winner sequence through the engine's ring-buffer voter as if every
    /// frame were Stable, returning where it first locks.
    fn engine_lock(seq: &[u8]) -> Option<(u8, usize)> {
        let mut e = Engine::new(44100);
        for (t, &w) in seq.iter().enumerate() {
            if e.record_stable_winner(w) {
                return Some((w, t));
            }
        }
        None
    }

    #[test]
    fn m_of_n_matches_reference_rule() {
        // Battery covering: clean lock, one-dissenter tolerance, alternating
        // no-lock, the eviction boundary (a vote that ages out of the window),
        // a mid-sequence key change, and short/empty inputs.
        let seqs: &[&[u8]] = &[
            &[],
            &[5],
            &[5, 5, 5, 5, 5, 5, 5],                // 7 straight → locks
            &[5, 5, 5, 5, 5, 5],                   // only 6 → never locks
            &[1, 2, 1, 2, 1, 5, 5, 5, 5, 5, 5, 5], // clean run after churn
            &[0, 1, 0, 1, 0, 1, 0, 1, 0, 1],       // alternating → never (needs 7 of 8)
            &[5, 5, 5, 5, 5, 5, 9, 5],             // 7 of the last 8 → one dissenter tolerated
            &[9, 5, 5, 5, 5, 5, 5, 5],             // leading dissenter then 7 straight
            &[5, 5, 5, 9, 5, 5, 5, 5, 5],          // gap inside the run
            &[3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4],    // winner switches, then locks on 4
        ];
        for seq in seqs {
            assert_eq!(
                engine_lock(seq),
                ref_eval(seq, LOCK_VOTES_M, LOCK_WINDOW_N),
                "engine and reference rule disagree on {seq:?}"
            );
        }
    }

    #[test]
    fn vote_ages_out_of_full_window() {
        // Votes that fall outside the last N frames must not count. Six fives,
        // then N alternating fillers (which never themselves reach M) evict
        // every five from the window, so a fresh run of M fives is needed to
        // lock — the six pre-eviction fives no longer contribute.
        let mut e = Engine::new(44100);
        for _ in 0..(LOCK_VOTES_M - 1) {
            assert!(!e.record_stable_winner(5));
        }
        for i in 0..LOCK_WINDOW_N {
            let filler = if i % 2 == 0 { 9 } else { 8 };
            assert!(!e.record_stable_winner(filler));
        }
        for _ in 0..(LOCK_VOTES_M - 1) {
            assert!(!e.record_stable_winner(5));
        }
        assert!(e.record_stable_winner(5)); // Mth five of the fresh run → lock
    }

    #[test]
    fn reset_clears_accumulated_votes() {
        let mut e = Engine::new(44100);
        for _ in 0..(LOCK_VOTES_M - 1) {
            assert!(!e.record_stable_winner(5));
        }
        e.reset_lock_window();
        // Post-reset, the full M straight frames are required again.
        for _ in 0..(LOCK_VOTES_M - 1) {
            assert!(!e.record_stable_winner(5));
        }
        assert!(e.record_stable_winner(5));
    }
}
