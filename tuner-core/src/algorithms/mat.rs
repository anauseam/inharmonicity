//! # Median-Adjustive Trajectories (MAT)
//!
//! Solves for the true fundamental frequency ($f_0$) and inharmonicity
//! coefficient ($B$) using Median-Adjustive Trajectories.
//!
//! Based on: Hodgkinson et al., "Handling Inharmonic Series with Median-Adjustive
//! Trajectories," DAFx-09, Como, Italy, September 2009.
//!
//! ## Algorithm Process
//!
//! 1. **Guided Trajectory (Peak Extraction)**: The paper dictates that the algorithm
//!    should predict the locations of subsequent partials based on the estimated $f_0$ and $B$.
//!    We seed this locally using the matching-template `f0_et` and theoretical `beta`.
//!    We locate up to 12 partial elements within the spectrum by performing sub-bin
//!    refinement (XQIFFT or Quinn's Second Estimator) at the predicted bins.
//!
//! 2. **Combinatorial Solver (MAT Array Logic)**: For each pair of extracted partials
//!    $(f_m, f_n)$ at harmonic indices $(m, n)$, the inharmonicity coefficient is solved algebraically:
//!    $$B = \frac{K_n - K_m}{K_m n^2 - K_n m^2}, \quad K_k = \left(\frac{f_k}{k}\right)^2$$
//!
//! 3. **Back-Calculated F0**: The fundamental is back-calculated from each valid pair constraint:
//!    $$f_0 = \frac{f_k}{k \sqrt{1 + B k^2}}$$
//!
//! 4. **Resilience Filter**: By computing the median of all individual $f_0$ evaluations,
//!    anomalous structural readings (like strings missing physical harmonics) are
//!    completely nullified out of the trajectory.

use crate::algorithms::pitch;

// ─── Public API ───────────────────────────────────────────────────────────────

/// Estimates the fundamental frequency from raw spectrum magnitudes using the MAT guided
/// trajectory procedure.
///
/// # Arguments
/// * `magnitudes` — Linear magnitude spectrum (`spectrum_to_magnitudes` output).
/// * `sample_rate` — Audio sample rate in Hz.
/// * `f0_et` — Base equal temperament seeding frequency matched structurally representing the target.
/// * `beta` — Expected inharmonicity beta metric assigned to the base frequency range.
/// * `is_bass` — Modifies the sub-bin searcher to utilize phase-independent `quinn_second_estimator` for low clarity signals.
/// * `partial_freqs_out` — Storage buffer passed from `Engine` returning partials.
/// * `partial_ns_out` — Storage buffer passed from `Engine` returning index pairs.
///
/// # Returns
/// `Some((f0, partial_count))` on success, `None` if no valid fundamental could be calculated.
pub(crate) fn detect_pitch_mat(
    magnitudes: &[f32],
    sample_rate: u32,
    f0_et: f32,
    beta: f32,
    is_bass: bool,
    partial_freqs_out: &mut [f32; 12],
    partial_ns_out: &mut [u32; 12],
) -> Option<(f32, usize)> {
    // ── 1. Guided Trajectory Extractor ──
    let mut partial_count = 0;

    for n in 1..=12 {
        let n_f = n as f32;
        let seed_hz = n_f * f0_et * (1.0 + beta * n_f * n_f).sqrt();

        // Break when exceeding signal Nyquist limitations.
        if seed_hz >= (sample_rate as f32 / 2.0) {
            break;
        }

        let extracted = if is_bass {
            pitch::quinn_second_estimator(magnitudes, sample_rate, seed_hz)
        } else {
            // High clarity treble spectrums handle phase dependencies gracefully.
            pitch::detect_pitch_xqifft_seeded(magnitudes, sample_rate, seed_hz, 2.0)
        };

        if let Some(frequency) = extracted {
            partial_freqs_out[partial_count] = frequency;
            partial_ns_out[partial_count] = n as u32;
            partial_count += 1;
        }
    }

    if partial_count < 2 {
        if partial_count == 1 && partial_freqs_out[0] > 0.0 {
            // Degraded fallback when only 1 peak presents.
            let n_f = partial_ns_out[0] as f32;
            let root_term = (1.0 + beta * n_f * n_f).sqrt();
            let f0 = partial_freqs_out[0] / (n_f * root_term);
            return Some((f0, partial_count));
        }
        return None;
    }

    // ── 2. MAT Algebraic Trajectory ──
    let mut b_estimates = [0.0_f32; 66]; // n*(n-1)/2 max combinatorics
    let mut f0_estimates = [0.0_f32; 66];
    let mut b_count = 0_usize;
    let mut f0_count = 0_usize;

    for i in 0..partial_count {
        for j in (i + 1)..partial_count {
            if let Some((b_v, f0_v)) = compute_pair(
                partial_freqs_out[i],
                partial_ns_out[i],
                partial_freqs_out[j],
                partial_ns_out[j],
            ) {
                if b_count < 66 {
                    b_estimates[b_count] = b_v;
                    b_count += 1;
                }
                if f0_count < 66 {
                    f0_estimates[f0_count] = f0_v;
                    f0_count += 1;
                }
            }
        }
    }

    if f0_count == 0 {
        return Some((
            partial_freqs_out[0] / partial_ns_out[0] as f32,
            partial_count,
        ));
    }

    // ── 3. Medians Rejection Evaluator ──
    let median_f0 = median_f32(&mut f0_estimates[..f0_count]);

    if median_f0 <= 0.0 || !median_f0.is_finite() {
        return None;
    }

    Some((median_f0, partial_count))
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn median_f32(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    values[values.len() / 2]
}

/// Pairwise B + f0 calculation from DAFx-09 Equations 8 and 6.
fn compute_pair(f_m: f32, n_m: u32, f_n: f32, n_n: u32) -> Option<(f32, f32)> {
    if n_m == n_n || n_m == 0 || n_n == 0 {
        return None;
    }
    let k_m = (f_m / n_m as f32).powi(2);
    let k_n = (f_n / n_n as f32).powi(2);
    let denom = k_m * (n_n as f32).powi(2) - k_n * (n_m as f32).powi(2);

    if denom.abs() < 1e-8 {
        return None;
    }

    let b = (k_n - k_m) / denom;

    // Theoretical bounds-checking. Most physical piano models map logically between 1e-5 to 1e-3.
    // This safely rejects anomalies while accommodating for calculation jitter.
    if b <= -0.001 || b >= 0.01 {
        return None;
    }

    let root_term = 1.0 + b * (n_m as f32).powi(2);
    if root_term <= 0.0 {
        return None;
    }

    let f0 = f_m / (n_m as f32 * root_term.sqrt());

    if f0 <= 0.0 || !f0.is_finite() {
        return None;
    }

    Some((b, f0))
}
