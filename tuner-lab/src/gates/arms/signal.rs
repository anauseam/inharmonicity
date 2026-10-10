//! The signal-side gate: frequency consistency. A live partial advances the
//! Goertzel's phase at one rate hop after hop; noise advances it at random, so
//! the wrapped increments against the target are uniform on (−π, π]. The test
//! is Rayleigh's for circular uniformity on the last `K` increments — the mean
//! resultant length `R`, with `Z = K R²` — level-free: no noise estimate enters.
//!
//! The increments are not independent — consecutive ones share a phase, and at
//! the 4096-sample window the hops overlap three quarters — so the null is
//! numerical: `Z`'s upper quantile under white noise at each window, simulated
//! once at construction. Greenwood & Durand's series for independent uniform
//! phases is kept as the reference the 1024-sample window is checked against.

use std::collections::HashMap;

use super::{Arm, Consumer, GateProbe, GateVerdict, HopCtx, SetCtx};
use crate::gates::truth::Gaussian;
use crate::truth::{SAMPLE_RATE, TAU, hann_vec};
use tuner_core::algorithms::spectral::goertzel_windowed;
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE};

/// Increments in the window: eight hops, the 8192-sample frame's own span.
const K: usize = 8;

/// Trials of the numerical null per window.
const NULL_TRIALS: usize = 40_000;

/// Upper-tail probability of `Z = K R²` for `K` independent uniform phases,
/// Greenwood & Durand (1955)'s series in `1/K`.
pub(super) fn rayleigh_tail(z: f64, k: usize) -> f64 {
    let n = k as f64;
    let corr = 1.0 + (2.0 * z - z * z) / (4.0 * n)
        - (24.0 * z - 132.0 * z * z + 76.0 * z.powi(3) - 9.0 * z.powi(4)) / (288.0 * n * n);
    ((-z).exp() * corr).clamp(0.0, 1.0)
}

/// The `Z` whose tail is `p` under independent uniform phases, by bisection.
pub(super) fn rayleigh_threshold(k: usize, p: f64) -> f64 {
    let (mut lo, mut hi) = (0.0f64, k as f64);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if rayleigh_tail(mid, k) > p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// `Z` of `K` consecutive wrapped increments of a Goertzel's phase at `window`
/// samples on white noise, hopped as the pipeline hops.
fn z_statistic(increments: &[f32]) -> f64 {
    let (c, s) = increments.iter().fold((0.0f64, 0.0f64), |(c, s), &d| {
        (c + (d as f64).cos(), s + (d as f64).sin())
    });
    let r = (c * c + s * s).sqrt() / increments.len() as f64;
    increments.len() as f64 * r * r
}

/// The `Z` whose upper tail is `p` under white noise at `window`, simulated:
/// `NULL_TRIALS` runs of `K + 1` hops, one Goertzel at a mid-band frequency each.
pub(super) fn numerical_threshold(window: usize, p: f64, seed: u64) -> f64 {
    let hann = hann_vec(window);
    let mut rng = Gaussian::new(seed);
    let f = 1000.0f32;
    let t_hop = HOP_SIZE as f32 / SAMPLE_RATE as f32;
    let expected = TAU * f * t_hop;
    let span = BASS_WINDOW_SIZE + K * HOP_SIZE;
    let mut noise = vec![0.0f32; span];
    let mut zs = Vec::with_capacity(NULL_TRIALS);
    let mut inc = vec![0.0f32; K];
    for _ in 0..NULL_TRIALS {
        for x in noise.iter_mut() {
            *x = rng.next();
        }
        let mut prev: Option<f32> = None;
        let mut j = 0;
        for h in 0..=K {
            let end = BASS_WINDOW_SIZE + h * HOP_SIZE;
            let (_, phase) = goertzel_windowed(&noise[end - window..end], SAMPLE_RATE, f, &hann);
            if let Some(pp) = prev {
                inc[j] = (phase - pp - expected + std::f32::consts::PI).rem_euclid(TAU)
                    - std::f32::consts::PI;
                j += 1;
            }
            prev = Some(phase);
        }
        zs.push(z_statistic(&inc));
    }
    zs.sort_by(f64::total_cmp);
    let idx = ((1.0 - p) * (zs.len() - 1) as f64).round() as usize;
    zs[idx.min(zs.len() - 1)]
}

pub(super) struct FrequencyConsistency {
    /// The last `K` wrapped increments per (fixed?, partial index).
    history: HashMap<(bool, usize), Vec<f32>>,
    /// The numerical null's threshold per window.
    thresholds: HashMap<usize, f64>,
    p_fa: f64,
}

impl FrequencyConsistency {
    pub(super) fn new() -> Self {
        Self {
            history: HashMap::new(),
            thresholds: HashMap::new(),
            p_fa: 1.0e-3,
        }
    }
}

impl Arm for FrequencyConsistency {
    fn name(&self) -> &str {
        "finst"
    }

    fn consumers(&self) -> &[Consumer] {
        &[Consumer::G2, Consumer::G3]
    }

    fn prepare(&mut self, set: &SetCtx) {
        self.history.clear();
        self.p_fa = set.p_fa as f64;
        self.thresholds.clear();
        for window in [1024usize, 4096] {
            let z = numerical_threshold(window, self.p_fa, 0x5EED_0000 + window as u64);
            println!(
                "[{}] arm finst: numerical null at the {window}-sample window, Z ≥ {z:.3} at {:.1e} ({} trials; independent-phase series {:.3})",
                set.label,
                self.p_fa,
                NULL_TRIALS,
                rayleigh_threshold(K, self.p_fa)
            );
            self.thresholds.insert(window, z);
        }
    }

    fn begin_capture(&mut self, _cap: &super::CaptureCtx) {
        self.history.clear();
    }

    fn gate(&mut self, _hop: &HopCtx, probe: &GateProbe) -> GateVerdict {
        let Some(f_inst) = probe.f_inst else {
            return GateVerdict::default();
        };
        let Some(&z_threshold) = self.thresholds.get(&probe.window) else {
            return GateVerdict::default();
        };
        // The vocoder's own step inverted: the wrapped phase increment against
        // the target's expected advance.
        let t_hop = HOP_SIZE as f32 / SAMPLE_RATE as f32;
        let delta = (f_inst - probe.target_hz) * TAU * t_hop;
        let hist = self.history.entry((probe.fixed, probe.index)).or_default();
        hist.push(delta);
        if hist.len() > K {
            hist.remove(0);
        }
        if hist.len() < K {
            return GateVerdict::default();
        }
        GateVerdict {
            threshold: None,
            estimate: None,
            interval: None,
            admit: Some(z_statistic(hist) >= z_threshold),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rayleigh_threshold_is_the_tail_inverse() {
        let z = rayleigh_threshold(K, 1e-3);
        assert!((rayleigh_tail(z, K) - 1e-3).abs() < 1e-9);
        // Without the correction the point is −ln p = 6.91; the series moves it.
        assert!(z > 5.0 && z < 8.0, "{z}");
    }

    /// At the 1024-sample window the hops do not overlap, so the numerical null
    /// sits near the independent-phase series; the 4096-sample window's
    /// overlap moves it well above.
    #[test]
    fn numerical_null_orders_the_windows() {
        let series = rayleigh_threshold(K, 1e-2);
        let short = numerical_threshold(1024, 1e-2, 7);
        let long = numerical_threshold(4096, 1e-2, 7);
        assert!(
            (short - series).abs() < 0.35 * series,
            "1024: {short} vs {series}"
        );
        assert!(long > short, "4096: {long} must exceed 1024: {short}");
    }
}
