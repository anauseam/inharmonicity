//! # Decimated pYIN (DPYIN) — Bass Register Pitch Detection
//!
//! A highly optimized, four-phase pitch detection algorithm for the bass register
//! (< 150 Hz). By decimating the signal before running the YIN core, DPYIN achieves
//! the long lag windows needed to resolve low fundamentals at a fraction of the
//! computational cost of full-rate YIN.
//!
//! ## Four-Phase Pipeline
//!
//! 1. **Signal Preparation & Decimation**: 4th-order Butterworth LPF (~500 Hz cutoff)
//!    followed by 8× downsampling (44,100 → 5,512.5 Hz, 8192 → 1024 samples).
//! 2. **CMNDF**: Cumulative Mean Normalized Difference Function on the decimated buffer,
//!    reusing [`pitch::yin_difference`].
//! 3. **Probabilistic Candidate Generation**: 100 Beta-distributed thresholds sweep the
//!    CMNDF. Each local minimum that crosses a threshold gains emission probability.
//!    Candidates are refined with [`pitch::parabolic_interpolation_offset`].
//! 4. **Viterbi Decoding**: Single-frame HMM selects the most probable pitch candidate,
//!    penalizing large pitch jumps via a log-frequency transition cost.

use crate::algorithms::pitch;

// ─── Constants ───────────────────────────────────────────────────────────────

/// Decimation factor. Reduces 44,100 Hz → 5,512.5 Hz.
const DECIMATION_FACTOR: usize = 8;

/// Effective sample rate after decimation.
const DECIMATED_SAMPLE_RATE: f32 = 44_100.0 / DECIMATION_FACTOR as f32; // 5512.5 Hz

/// Number of Beta-distributed thresholds for probabilistic candidate generation.
const NUM_THRESHOLDS: usize = 100;

/// Maximum number of pitch candidates to track per frame.
const MAX_CANDIDATES: usize = 16;

// ─── Phase 1: Anti-Aliasing & Decimation ─────────────────────────────────────

/// Coefficients for a single second-order section (biquad) of the Butterworth filter.
///
/// Transfer function: H(z) = (b0 + b1·z⁻¹ + b2·z⁻²) / (1 + a1·z⁻¹ + a2·z⁻²)
struct BiquadCoeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

/// Transient state for a single biquad section (Direct Form II Transposed).
struct BiquadState {
    z1: f32,
    z2: f32,
}

impl BiquadState {
    fn new() -> Self {
        Self { z1: 0.0, z2: 0.0 }
    }

    /// Process a single sample through this biquad section.
    #[inline]
    fn process(&mut self, input: f32, c: &BiquadCoeffs) -> f32 {
        let output = c.b0 * input + self.z1;
        self.z1 = c.b1 * input - c.a1 * output + self.z2;
        self.z2 = c.b2 * input - c.a2 * output;
        output
    }
}

/// Compute biquad coefficients for one second-order section of a Butterworth
/// low-pass filter using the bilinear transform.
///
/// # Arguments
/// * `cutoff_hz` — Desired cutoff frequency.
/// * `sample_rate` — Sample rate of the incoming audio.
/// * `q` — Quality factor for this section (derived from Butterworth pole angles).
fn butterworth_biquad_coeffs(cutoff_hz: f32, sample_rate: f32, q: f32) -> BiquadCoeffs {
    let omega = 2.0 * std::f32::consts::PI * cutoff_hz / sample_rate;
    let sin_omega = omega.sin();
    let cos_omega = omega.cos();
    let alpha = sin_omega / (2.0 * q);

    let b0 = (1.0 - cos_omega) / 2.0;
    let b1 = 1.0 - cos_omega;
    let b2 = (1.0 - cos_omega) / 2.0;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos_omega;
    let a2 = 1.0 - alpha;

    // Normalize by a0
    BiquadCoeffs {
        b0: b0 / a0,
        b1: b1 / a0,
        b2: b2 / a0,
        a1: a1 / a0,
        a2: a2 / a0,
    }
}

/// Applies a 4th-order Butterworth low-pass filter (two cascaded biquad sections)
/// in-place on the provided audio buffer.
///
/// The cutoff is set to 500 Hz to strip all energy above the Nyquist of the
/// decimated rate (5512.5 / 2 = 2756 Hz), with generous margin to prevent aliasing.
fn apply_butterworth_lowpass(buffer: &mut [f32], sample_rate: f32) {
    let cutoff = 500.0_f32;

    // 4th-order Butterworth has 2 conjugate pole pairs.
    // The Q factors for each section are derived from the pole angles:
    //   Section 1: Q = 1 / (2 · cos(π/8))  ≈ 0.5412
    //   Section 2: Q = 1 / (2 · cos(3π/8)) ≈ 1.3066
    let q1 = 1.0 / (2.0 * (std::f32::consts::PI / 8.0).cos());
    let q2 = 1.0 / (2.0 * (3.0 * std::f32::consts::PI / 8.0).cos());

    let c1 = butterworth_biquad_coeffs(cutoff, sample_rate, q1);
    let c2 = butterworth_biquad_coeffs(cutoff, sample_rate, q2);

    let mut s1 = BiquadState::new();
    let mut s2 = BiquadState::new();

    for sample in buffer.iter_mut() {
        let out1 = s1.process(*sample, &c1);
        *sample = s2.process(out1, &c2);
    }
}

/// Decimates an audio buffer by `DECIMATION_FACTOR`, writing every Mth sample
/// into the output slice.
///
/// # Returns
/// The number of samples written to `output`.
fn decimate(input: &[f32], output: &mut [f32]) -> usize {
    let count = input.len() / DECIMATION_FACTOR;
    for i in 0..count {
        output[i] = input[i * DECIMATION_FACTOR];
    }
    count
}

// ─── Phase 3: Probabilistic Candidate Generation ─────────────────────────────

/// A single pitch candidate extracted from the CMNDF.
#[derive(Clone, Copy)]
struct PitchCandidate {
    /// Sub-sample-refined lag period in the decimated domain.
    lag: f32,
    /// Emission probability: fraction of Beta thresholds this trough crossed.
    probability: f32,
}

/// Pre-computed Beta(2, 18) CDF thresholds, evaluated at 100 evenly spaced quantiles.
///
/// The Beta(2,18) distribution concentrates probability mass near 0, producing
/// mostly low thresholds. This aligns with the expectation that strong pitch
/// candidates have very low CMNDF values.
fn beta_thresholds() -> [f32; NUM_THRESHOLDS] {
    let mut thresholds = [0.0_f32; NUM_THRESHOLDS];
    let alpha = 2.0_f64;
    let beta = 10.0_f64;

    for i in 0..NUM_THRESHOLDS {
        // Evenly spaced quantiles from the CDF of Beta(2, 10).
        // Relaxed from Beta(2,18) to spread thresholds more evenly,
        // giving inharmonic bass string troughs a fairer confidence score.
        let target = (i as f64 + 0.5) / NUM_THRESHOLDS as f64;
        thresholds[i] = invert_beta_cdf(target, alpha, beta) as f32;
    }
    thresholds
}

/// Evaluates the regularized incomplete beta function I_x(a, b) for Beta(2, 18).
///
/// For integer parameters a=2, b=18 this has the closed form:
/// I_x(2,18) = 1 - (1-x)^18 - 18·x·(1-x)^17  (Wait, let me derive properly)
///
/// Actually for Beta(a,b) with integer params, we use the expansion.
/// For a=2, b=18: I_x(2,18) = 1 - (1-x)^19 * [1 + 19*x / ... ]
/// Simpler: use numerical bisection on the Beta CDF which is straightforward.
fn beta_cdf(x: f64, _alpha: f64, beta: f64) -> f64 {
    // For Beta(2, b), the CDF has a closed form:
    // I_x(2, b) = 1 - (1-x)^b * (1 + b*x)
    // This comes from the incomplete beta function with integer parameters.
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let one_minus_x = 1.0 - x;
    let b = beta as i32;
    1.0 - one_minus_x.powi(b) * (1.0 + beta * x)
}

/// Inverts the Beta CDF via bisection to find the threshold value for a given quantile.
fn invert_beta_cdf(target: f64, alpha: f64, beta_param: f64) -> f64 {
    let mut lo = 0.0_f64;
    let mut hi = 1.0_f64;
    for _ in 0..50 {
        let mid = (lo + hi) / 2.0;
        if beta_cdf(mid, alpha, beta_param) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

/// Extracts pitch candidates from the CMNDF using multi-threshold probabilistic evaluation.
///
/// Each local minimum in the CMNDF is tested against `NUM_THRESHOLDS` Beta-distributed
/// thresholds. The fraction of thresholds crossed determines the candidate's emission
/// probability. Candidates are refined with parabolic interpolation for sub-sample precision.
fn generate_candidates(
    yin_buffer: &[f32],
    buffer_len: usize,
    candidates: &mut [PitchCandidate; MAX_CANDIDATES],
) -> usize {
    let thresholds = beta_thresholds();
    let half = buffer_len / 2;
    let mut count = 0;

    // Scan for local minima in the CMNDF
    for tau in 2..(half - 1) {
        let prev = yin_buffer[tau - 1];
        let current = yin_buffer[tau];
        let next = yin_buffer[tau + 1];

        // Is this a local minimum?
        if current < prev && current < next {
            // Count how many thresholds this minimum crosses
            let crossings = thresholds.iter().filter(|&&t| current < t).count();

            if crossings == 0 {
                continue;
            }

            let probability = crossings as f32 / NUM_THRESHOLDS as f32;

            // Refine the lag with parabolic interpolation
            let offset = pitch::parabolic_interpolation_offset(prev, current, next)
                .unwrap_or(0.0);
            let refined_lag = tau as f32 + offset;

            if refined_lag > 0.0 && count < MAX_CANDIDATES {
                candidates[count] = PitchCandidate {
                    lag: refined_lag,
                    probability,
                };
                count += 1;
            }
        }
    }

    count
}

// ─── Phase 4: Viterbi Decoding ───────────────────────────────────────────────

/// Selects the best pitch candidate using a single-frame Viterbi-style evaluation.
///
/// Without a previous frame's state, this reduces to selecting the candidate with
/// the highest emission probability. When a `prev_lag` is available (from the
/// previous frame), a transition cost penalizes large pitch jumps in log-frequency
/// space, implementing the HMM's transition matrix.
///
/// # Arguments
/// * `candidates` — Array of pitch candidates for this frame.
/// * `count` — Number of valid candidates.
/// * `prev_lag` — The winning lag from the previous frame, if any.
///
/// # Returns
/// The index of the winning candidate, or `None` if no candidates exist.
fn viterbi_select(
    candidates: &[PitchCandidate; MAX_CANDIDATES],
    count: usize,
    prev_lag: Option<f32>,
) -> Option<usize> {
    if count == 0 {
        return None;
    }

    let mut best_idx = 0;
    let mut best_score = f32::NEG_INFINITY;

    for i in 0..count {
        let c = &candidates[i];

        // Emission score: log probability (higher is better)
        let emission = c.probability.ln();

        // Transition score: penalize large jumps from previous pitch
        let transition = if let Some(prev) = prev_lag {
            // Cost proportional to the absolute log-frequency distance.
            // A semitone is ~5.95% change, so jumps > ~1 semitone are penalized heavily.
            let log_ratio = (c.lag / prev).ln().abs();
            // Transition weight: each log-ratio unit costs -12 (tuned for piano stability)
            -12.0 * log_ratio
        } else {
            0.0 // No previous state — no transition penalty
        };

        let score = emission + transition;
        if score > best_score {
            best_score = score;
            best_idx = i;
        }
    }

    Some(best_idx)
}

// ─── Top-Level API ───────────────────────────────────────────────────────────

/// Detects the fundamental frequency of a bass-register piano note using the
/// Decimated pYIN algorithm.
///
/// This is the primary entry point for the Bass Engine. It executes the full
/// four-phase pipeline: Butterworth anti-aliasing → decimation → CMNDF →
/// multi-threshold candidate generation → Viterbi selection.
///
/// # Arguments
/// * `audio_8192` — 8192-sample audio buffer from the ring buffer.
/// * `sample_rate` — Original sample rate (e.g., 44100 Hz).
/// * `scratch` — Mutable scratch buffer (at least 8192 floats). The first 8192
///   elements are used for the anti-aliased + decimated signal and YIN working space.
///
/// Silence gating is handled upstream by the [`Gatekeeper`](crate::gatekeeper).
/// This function trusts that the caller has already verified the signal is above
/// the calibrated silence threshold.
///
/// # Returns
/// * `Some((frequency, confidence))` — Detected F0 in Hz and optional confidence.
/// * `None` — No pitch detected (silence, noise, or invalid signal).
pub fn detect_pitch_dpyin(
    audio_8192: &[f32],
    sample_rate: u32,
    scratch: &mut [f32],
    prev_lag: Option<f32>,
) -> Option<(f32, Option<f32>)> {
    let input_len = 8192;
    assert!(
        audio_8192.len() >= input_len,
        "DPYIN requires at least 8192 audio samples"
    );
    assert!(
        scratch.len() >= input_len,
        "DPYIN scratch buffer must be at least 8192 floats"
    );

    // --- Phase 1: Anti-aliasing + Decimation ---
    // Copy audio into scratch for in-place filtering
    scratch[..input_len].copy_from_slice(&audio_8192[..input_len]);

    // Apply 4th-order Butterworth LPF (500 Hz cutoff)
    apply_butterworth_lowpass(&mut scratch[..input_len], sample_rate as f32);

    // Decimate: write 1024 decimated samples into a local buffer
    let decimated_len = input_len / DECIMATION_FACTOR; // 1024
    let mut decimated = [0.0_f32; 1024];
    decimate(&scratch[..input_len], &mut decimated[..]);

    // --- Phase 2: CMNDF ---
    // Use scratch as the YIN buffer (needs at least decimated_len / 2 = 512)
    let yin_half = decimated_len / 2; // 512
    scratch[..yin_half].fill(0.0);
    pitch::yin_difference(&decimated[..], decimated_len, &mut scratch[..yin_half]);

    // --- Phase 3: Probabilistic Candidate Generation ---
    let mut candidates = [PitchCandidate {
        lag: 0.0,
        probability: 0.0,
    }; MAX_CANDIDATES];
    let candidate_count = generate_candidates(&scratch[..yin_half], decimated_len, &mut candidates);

    if candidate_count == 0 {
        return None;
    }

    // --- Phase 4: Viterbi Selection ---
    // Use the previous frame's winning lag (if available) so the Viterbi
    // transition penalty can stabilize tracking across frames.
    let winner_idx = viterbi_select(&candidates, candidate_count, prev_lag)?;
    let winner = &candidates[winner_idx];

    // Convert winning lag → frequency using the decimated sample rate
    let frequency = DECIMATED_SAMPLE_RATE / winner.lag;

    // Accept up to 300 Hz — the scout already decided this is a bass note,
    // so DPYIN shouldn't second-guess it with a tight cap. This prevents
    // borderline notes (e.g., D#3 at 155 Hz) from being silently discarded.
    if frequency.is_finite() && frequency > 20.0 && frequency < 300.0 {
        // Confidence derived from emission probability
        let confidence = Some(winner.probability);
        Some((frequency, confidence))
    } else {
        None
    }
}
