//! # Engine (Thread 2) — Fundamental Frequency Detection
//!
//! The "Brains" of the pipeline. The Engine is orchestrated by the `AudioPipeline`
//! after the signal has been validated by the Gatekeeper. Its sole responsibility
//! is to process the signal and extract the exact fundamental frequency.

use crate::algorithms::{
    discovery,
    peaks::{self, extract_peaks},
    spectral, twm,
};
use crate::audio::{BASS_WINDOW_SIZE, HOP_SIZE};
use crate::models::{KeyProfile, MAX_PARTIALS, SpectralPeak};
use crate::pipeline::ProcessingFrame;

// ── M-of-N Acquisition Lock (Binary Integration) ──
// Discovery locks the first key to win ≥ M of the last N Stable-frame scans —
// the binary-integration / coincidence procedure (Schwartz 1956, IRE Trans. IT
// 2(4); Shnidman 1998, IEEE Trans. AES 34(3)). M > N/2 guarantees at most one
// key can hold ≥ M votes, so only the frame's own winner is ever tested.
// (M, N) = (7, 8) is the refined-path production pick; provenance and the full
// (M, N) latency/accuracy tradeoff (e.g. the cheaper (5, 6) ≈ 46 ms vs ≈ 93 ms)
// are in docs/adr/0010-m-of-n-lock-rule-replay.md.
const LOCK_VOTES_M: usize = 7;
const LOCK_WINDOW_N: usize = 8;

/// Result of a successful pitch detection frame.
#[derive(Debug, Clone)]
pub struct PitchResult {
    /// 0–87 key index of the identified note.
    pub key_index: u8,
    /// Physical fundamental frequency (Partial 1), tracked via Goertzel phase vocoder. Returns None if Partial 1 is dead.
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
    /// Ring buffer of the last `LOCK_WINDOW_N` Stable-frame discovery winners
    /// (key indices), with its fill length and write cursor — the M-of-N lock
    /// window (see [`Engine::record_stable_winner`]).
    lock_window: [u8; LOCK_WINDOW_N],
    lock_window_len: usize,
    lock_window_head: usize,

    // Tracking state
    tracking_targets: [f32; MAX_PARTIALS],
    partial_trackers: [PartialTracker; MAX_PARTIALS],
    warmup_hops: u8,
    /// Winning Stage B scale (s_win) of the current lock; 1.0 when unlocked.
    locked_scale: f32,
    /// R3 for the tracker: `true` while the locked key's partial spacing
    /// (≈ f₀, proxied by the f₁ seed) sits inside the 1024-sample Hann main
    /// lobe (half-width 2·fs/1024 ≈ 86 Hz), selecting the 4096-sample
    /// Goertzel window for every partial of the key. Same derivation and
    /// boundary as [`crate::strobe::Strobe`]'s long-window rule; window
    /// length ≠ hop, so the ±21.5 Hz phase-unwrap range is unchanged.
    long_window: bool,

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
            lock_window: [0; LOCK_WINDOW_N],
            lock_window_len: 0,
            lock_window_head: 0,
            tracking_targets: [0.0; MAX_PARTIALS],
            partial_trackers: [PartialTracker::default(); MAX_PARTIALS],
            warmup_hops: 0,
            locked_scale: 1.0,
            long_window: false,
            noise_floor: 0.0, // updated by pipeline
            peak_scratch: vec![SpectralPeak::default(); 64].into_boxed_slice(),
        }
    }

    /// Records a Stable-frame discovery winner into the M-of-N window and
    /// returns whether `key` has reached the lock threshold.
    ///
    /// Binary integration (Schwartz 1956; Shnidman 1998; ADR 0010): lock the
    /// first key to win ≥ `LOCK_VOTES_M` of the last `LOCK_WINDOW_N` Stable
    /// frames. The window is a fixed ring buffer — the oldest vote is
    /// overwritten once it is full — so a partially-filled window lets clean
    /// evidence lock after `M` straight frames. Only Stable frames call this: a
    /// non-Stable interruption casts no vote and leaves the window intact
    /// ([`reset_lock_window`](Self::reset_lock_window) clears it on a new
    /// onset). `M > N/2` guarantees at most one key can hold ≥ `M` votes, so
    /// testing the just-recorded key alone is exact.
    fn record_stable_winner(&mut self, key: u8) -> bool {
        self.lock_window[self.lock_window_head] = key;
        self.lock_window_head = (self.lock_window_head + 1) % LOCK_WINDOW_N;
        if self.lock_window_len < LOCK_WINDOW_N {
            self.lock_window_len += 1;
        }
        // Until the buffer is full the head advances in lockstep with the fill
        // length, so the valid votes are always the first `len` slots. N ≤ 8 ⇒
        // this linear count is trivially cheap and allocation-free.
        let votes = self.lock_window[..self.lock_window_len]
            .iter()
            .filter(|&&k| k == key)
            .count();
        votes >= LOCK_VOTES_M
    }

    /// Clears the M-of-N lock window (new onset / silence / transient bypass).
    /// Only the fill length and cursor are reset; slots past `len` are never
    /// read.
    #[inline]
    fn reset_lock_window(&mut self) {
        self.lock_window_len = 0;
        self.lock_window_head = 0;
    }

    /// Executes the primary DSP detection loop for a single frame.
    ///
    /// `profiles` is the read-only per-key template table to score against.
    // Gate state is passed as decomposed booleans (not the gatekeeper's
    // `GateResult`) to keep the engine decoupled from `crate::gatekeeper`.
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
            // Auto-mode M-of-N votes accrue on Stable frames only (ADR 0010
            // window semantics): a non-Stable frame casts no vote and — crucially
            // — must not clear the accumulated window, so skip the Stage-A scan
            // entirely until the gatekeeper is Stable again. Manual mode (an
            // explicit target) is acquisition-immediate and unaffected.
            if target_note.is_none() && !is_stable {
                return None;
            }

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
            let (winning_key, lock_acquired, s_win, min_error) =
                if let Some(target_idx) = target_note {
                    // Manual Mode: bypass the 88-key scan, but still run Stage B scale
                    // refinement on the single target profile — otherwise this is the
                    // worst-seeded path (pure ET), and it is the critical one for
                    // Pitch Raise on heavily mistuned strings.
                    let (s, err) =
                        discovery::refine_scale(active_peaks, &profiles[target_idx as usize], &cfg);
                    (target_idx, true, s, err)
                } else {
                    // Auto Mode: split discovery (ADR 0005) — Stage A discrete 88-key
                    // scan, Stage B basin-clamped scale refinement of the top-3.
                    let res = discovery::discover(active_peaks, profiles, &cfg, true);

                    // M-of-N binary-integration lock (ADR 0010): count this
                    // Stable-frame winner into the N-window; lock the first key
                    // to reach M votes. `s_win`/`error` come from the winning
                    // frame's own scan — because M > N/2, the key that crosses
                    // the threshold is always this frame's `res.key_index`.
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
                // Long window iff the 1024-sample main-lobe half-width
                // (2·fs/1024) exceeds the partial spacing, proxied by the
                // refined f₁ seed (spacing ≈ f₀ ≤ f₁ for every partial pair
                // of the key) — the strobe bank's R3 rule applied to the
                // tracker (its absence was Prompt N Defect 1: guitar E2's
                // 2nd partial inside the lobe read ±47 ¢ of jitter).
                self.long_window =
                    profile.predicted_partials[0] * s_win * 1024.0 < 2.0 * self.sample_rate as f32;

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
        // The evaluator reads the freshest `window` samples of the full COLA
        // buffer; the hop cadence (and hence the ±21.5 Hz unwrap range) is
        // set by the caller, not the window length.
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

            // Kay 1998 Neyman–Pearson amplitude gate at the active window
            // length (see `spectral::neyman_pearson_k` for the derivation).
            let t_amp = self.noise_floor * np_k;

            // A physical partial has a positive, finite frequency. On a
            // spurious deep-bass lock the tracker free-runs on noise and the
            // adaptive target can walk toward DC until f_target − 21.5 Hz
            // (the unwrap half-range) crosses zero; such an f_inst is not a
            // partial reading. Gating it to weight 0 both keeps it out of
            // the result (a negative f_inst reached hz_to_cents as NaN and
            // panicked the GUI canvas, observed 2026-07-10) and stops the
            // target adaptation that lets the walk continue.
            let weight = if amplitude < t_amp || !(f_inst.is_finite() && f_inst > 0.0) {
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

    /// Reference M-of-N rule — a direct port of `eval_rule` in
    /// `scripts/replay_lock_rules.py`, the harness the ADR 0010 numbers were
    /// measured with. Returns `(key, index)` of the first lock, or `None`.
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
