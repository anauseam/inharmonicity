//! # Two-Way Mismatch (TWM) Module
//!
//! This module implements the Two-Way Mismatch (TWM) algorithm, serving as the 
//! coarse fundamental frequency ($f_0$) estimator in the pipeline. By generating 
//! a theoretical template of harmonic partials and comparing it against measured 
//! spectral peaks, TWM evaluates both "Predicted-to-Observed" and 
//! "Observed-to-Predicted" errors to robustly identify the true $f_0$ without 
//! octave errors.
//!
//! ## Features
//! - Sub-bin accurate spectral peak extraction via fast parabolic interpolation.
//! - Targeted harmonic template matching with inharmonicity ($B$) coefficient compensation.
//! - Octave-error immunity across the full piano range (A0 to C8).

use crate::engine::RoutingState;

// Threshold above which a candidate is rejected
const ERROR_CEILING: f32 = 0.25;

#[derive(Debug, Clone, Copy, Default)]
pub struct SpectralPeak {
    pub freq: f32,
    pub magnitude: f32,
}

#[derive(Debug, Clone, Copy)]
struct TwmCandidate {
    pub f0: f32,
    pub error: f32,
}

/// Populates the provided scratch buffer with local maxima from the magnitude spectrum.
/// Uses parabolic interpolation for sub-bin frequency precision.
///
/// Returns the number of peaks placed into `out`. No heap allocations occur.
pub fn extract_spectral_peaks(
    magnitudes: &[f32],
    sample_rate: u32,
    window_size: usize,
    out: &mut [SpectralPeak],
) -> usize {
    let max_magnitude = magnitudes.iter().cloned().fold(0.0_f32, f32::max);
    if max_magnitude <= 1e-6 {
        return 0; // Silence: skip entirely
    }
    let dynamic_threshold = (max_magnitude * 0.01_f32).max(1e-5);

    let mut count = 0;
    let max_peaks = out.len();
    
    // Ignore DC and Nyquist
    for i in 1..(magnitudes.len() - 1) {
        if count >= max_peaks {
            break; // Stop if scratch buffer is full
        }

        let mag_prev = magnitudes[i - 1];
        let mag_curr = magnitudes[i];
        let mag_next = magnitudes[i + 1];

        if mag_curr > mag_prev && mag_curr > mag_next && mag_curr > dynamic_threshold {
            // Found a peak, apply parabolic interpolation on log-magnitudes
            let y1 = mag_prev.max(1e-6).ln();
            let y2 = mag_curr.max(1e-6).ln();
            let y3 = mag_next.max(1e-6).ln();
            
            let denom = y1 - 2.0 * y2 + y3;
            let offset = if denom.abs() > 1e-6 {
                (y1 - y3) / (2.0 * denom)
            } else {
                0.0
            };

            let interpolated_bin = i as f32 + offset;
            let freq = (interpolated_bin * sample_rate as f32) / window_size as f32;
            
            if freq.is_finite() && freq > 0.0 {
                out[count] = SpectralPeak {
                    freq,
                    magnitude: mag_curr,
                };
                count += 1;
            }
        }
    }
    
    // Sort peaks by magnitude descending and keep only the strongest if we have many
    out[..count].sort_unstable_by(|a, b| b.magnitude.partial_cmp(&a.magnitude).unwrap_or(std::cmp::Ordering::Equal));
    count
}

/// Computes the Two-Way Mismatch error for a predicted harmonic template vs observed peaks.
fn compute_twm_error(candidate_f0: f32, peaks: &[SpectralPeak], _sample_rate: u32, inharmonicity_b: f32) -> f32 {
    let max_peak_freq = peaks.iter().map(|p| p.freq).fold(0.0_f32, f32::max);
    
    // Dynamically calculate n_partials by generating expected frequencies with inharmonicity
    // until we exceed the highest measured peak (plus a half-step margin).
    let mut n_partials = 0;
    while n_partials < 20 {
        let n_f32 = (n_partials + 1) as f32;
        let p_n = candidate_f0 * n_f32 * (1.0 + inharmonicity_b * n_f32 * n_f32).sqrt();
        if p_n > max_peak_freq + candidate_f0 * 0.5 {
            break;
        }
        n_partials += 1;
    }
    let n_partials = n_partials.max(1);
    
    // P -> O
    let mut p_to_o = 0.0;
    for n in 1..=n_partials {
        let n_f32 = n as f32;
        let p_n = candidate_f0 * n_f32 * (1.0 + inharmonicity_b * n_f32 * n_f32).sqrt();
        
        let min_dist = peaks.iter()
            .map(|peak| (peak.freq - p_n).abs())
            .fold(f32::INFINITY, f32::min);
        
        // Normalize distance by expected freq (cents-like scaling)
        p_to_o += min_dist / p_n.powf(0.8); 
    }
    p_to_o /= n_partials as f32;
    
    // O -> P (split into Set α and Set β)
    // Set α: peaks >= candidate_f0 (standard normalized penalty)
    // Set β: peaks < candidate_f0  (Subharmonic Veto — absolute, undiluted penalty)

    let a_max = peaks.iter().map(|p| p.magnitude).fold(0.0_f32, f32::max);

    let mut o_to_p_alpha = 0.0_f32;
    let mut k_alpha = 0_usize;
    let mut subharmonic_veto = 0.0_f32; // absolute, un-normalized Set β penalty

    let freq_tolerance = candidate_f0 * 0.25; // quarter-tone margin below F0

    for peak in peaks {
        if peak.freq >= candidate_f0 - freq_tolerance {
            // Set α: standard M&B O->P matching
            let mut min_dist = f32::INFINITY;
            let mut best_p_n = candidate_f0;
            for n in 1..=n_partials {
                let n_f32 = n as f32;
                let p_n = candidate_f0 * n_f32 * (1.0 + inharmonicity_b * n_f32 * n_f32).sqrt();
                let dist = (peak.freq - p_n).abs();
                if dist < min_dist {
                    min_dist = dist;
                    best_p_n = p_n;
                }
            }
            o_to_p_alpha += min_dist / best_p_n;
            k_alpha += 1;
        } else {
            // Set β: unexplained sub-fundamental peak — Subharmonic Veto
            // The penalty is quadratic, smoothly ignoring 4% noise floors (<0.05 error)
            // but instantly and strictly vetoing >=10% weak piano fundamentals (>0.30 error).
            let relative_amp = (peak.magnitude / a_max.max(1e-6)).clamp(0.0, 1.0);
            subharmonic_veto += 30.0 * relative_amp.powi(2);
        }
    }

    let o_to_p = if k_alpha > 0 {
        o_to_p_alpha / k_alpha as f32
    } else {
        0.0
    };
    
    // Dynamic ρ: increases when low-frequency energy is sparse (fundamental fading)
    // This prevents the O->P penalty from being globally suppressed at end-of-sustain.
    let low_freq_energy: f32 = peaks.iter()
        .filter(|p| p.freq < 1000.0)
        .map(|p| p.magnitude)
        .sum();
    let total_energy: f32 = peaks.iter().map(|p| p.magnitude).sum::<f32>().max(1e-6);
    let low_freq_ratio = (low_freq_energy / total_energy).clamp(0.01, 1.0);

    // ρ base = 0.5 (higher than M&B 0.33 to suit piano)
    // As fundamentals fade (low_freq_ratio → 0), ρ rises toward 0.9
    let rho = 0.5_f32 + 0.4 * (1.0 / low_freq_ratio - 1.0).clamp(0.0, 1.0);

    // Total error = normalized P->O  +  dynamic-ρ weighted O->P  +  absolute subharmonic veto
    p_to_o + rho * o_to_p + subharmonic_veto
}

/// # Arguments
/// * `peaks` — Pre-extracted peaks (≥3 required). Use `peak_scratch` from `ProcessingFrame`.
/// * `sample_rate` - Example: 44100
/// * `routing_state` — Scout lock; constrains candidate range when `key_hint` is None.
/// * `key_hint` — If Some, targeted mode (±50 cents, ~32 candidates).
/// * `inharmonicity_b` — If Some, stretch harmonic template to physical string stiffness.
pub fn detect_pitch_twm(
    peaks: &[SpectralPeak],
    sample_rate: u32,
    routing_state: RoutingState,
    key_hint: Option<f32>,
    inharmonicity_b: Option<f32>,
) -> Option<(f32, Option<f32>)> {
    if peaks.len() < 1 {
        return None;
    }
    
    let b_val = inharmonicity_b.unwrap_or(0.0);
    
    // 1. Candidate Generation
    let mut candidates = [TwmCandidate { f0: 0.0, error: 0.0 }; 128];
    let mut num_candidates = 0;
    
    if let Some(target_f0) = key_hint {
        // Targeted Mode: ±50 cents around the hint
        // ~32 candidates spaced logarithmically
        let cents_range = 50.0;
        let num_steps = 32;
        
        for i in 0..num_steps {
            // map i=0..31 to -50..+50 cents
            let cents_offset = -cents_range + (i as f32 / (num_steps - 1) as f32) * (2.0 * cents_range);
            let f0 = target_f0 * 2.0_f32.powf(cents_offset / 1200.0);
            
            let error = compute_twm_error(f0, peaks, sample_rate, b_val);
            candidates[num_candidates] = TwmCandidate { f0, error };
            num_candidates += 1;
        }
    } else {
        // Discovery Mode
        let (f_min, f_max): (f32, f32) = match routing_state {
            RoutingState::LockedBass   => (27.5,   400.0),
            RoutingState::LockedTreble => (130.0, 4186.0),
            RoutingState::Unclassified => (27.5,  4186.0), // fallback only
        };
        
        let true_step = 2.0_f32.powf(1.0 / 12.0); // Exactly 1 semitone per step
        
        let mut current_f0 = f_min;
        while current_f0 <= f_max && num_candidates < candidates.len() {
            let error = compute_twm_error(current_f0, peaks, sample_rate, b_val);
            candidates[num_candidates] = TwmCandidate { f0: current_f0, error };
            num_candidates += 1;
            
            current_f0 *= true_step;
        }
    }
    
    if num_candidates == 0 {
        return None;
    }
    
    // Find best candidate
    let best = candidates[..num_candidates].iter().min_by(|a, b| a.error.partial_cmp(&b.error).unwrap())?;
    
    if best.error > ERROR_CEILING {
        return None;
    }
    
    let confidence = 1.0 - (best.error / ERROR_CEILING).clamp(0.0, 1.0);
    Some((best.f0, Some(confidence)))
}
