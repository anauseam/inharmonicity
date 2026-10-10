//! The baselines every stage reads against: the shipped scalar (A), the record's
//! oracle (B), the coarse read's OS-CFAR as a per-bin gate (C), and no gate at
//! all (N).

use super::{
    Arm, Availability, CaptureCtx, Consumer, GateProbe, GateVerdict, HopCtx, LOBE_HALF_BINS,
    SetCtx, median_to_threshold, raw_to_gate,
};
use crate::truth::cfar_multiplier_finite;
use tuner_core::audio::BASS_WINDOW_SIZE;
use tuner_core::pipeline::CAPTURE_ANALYSIS_SAMPLES;

/// Kay's magnitude threshold on an unnormalised Hann frame of `n` samples with
/// time-domain noise σ, at `p_fa`: `√(σ²·0.375n·ln(1/p_fa))` — discovery's peak
/// floor at 10⁻³ (`Engine::process`).
fn kay_raw(sigma: f32, n: usize, p_fa: f32) -> f32 {
    (-sigma * sigma * 0.375 * n as f32 * p_fa.ln()).sqrt()
}

/// The same threshold in a Goertzel's `4/n` units: `σ·(4/n)·√(0.375n·ln(1/p_fa))`,
/// `σ·K(n)` at 10⁻³ (`spectral::neyman_pearson_k`).
fn kay_gate(sigma: f32, n: usize, p_fa: f32) -> f32 {
    kay_raw(sigma, n, p_fa) * 4.0 / n as f32
}

/// Arm A: the one scalar at every site, `T = σ·K(N)` in the consumer's units
/// (Kay 1998, Ch. 9, with σ the silence threshold), at each capture's recorded σ
/// or at one fixed value.
pub(super) struct Shipped {
    fixed: Option<f32>,
    p_fa: f32,
    name: String,
}

impl Shipped {
    pub(super) fn recorded() -> Self {
        Self {
            fixed: None,
            p_fa: 1.0e-3,
            name: "A".to_string(),
        }
    }

    pub(super) fn fixed(sigma: f32) -> Self {
        Self {
            fixed: Some(sigma),
            p_fa: 1.0e-3,
            name: format!("A@{sigma:.0e}"),
        }
    }

    fn sigma(&self, hop: &HopCtx) -> f32 {
        self.fixed.unwrap_or(hop.sigma)
    }
}

impl Arm for Shipped {
    fn name(&self) -> &str {
        &self.name
    }

    fn consumers(&self) -> &[Consumer] {
        &[Consumer::G1, Consumer::G2, Consumer::G3, Consumer::Bg]
    }

    fn prepare(&mut self, set: &SetCtx) {
        if self.fixed.is_none() {
            self.fixed = set.sigma_override;
        }
        self.p_fa = set.p_fa;
    }

    fn g1_threshold(&mut self, hop: &HopCtx, out: &mut [f32]) -> Availability {
        out.fill(kay_raw(self.sigma(hop), BASS_WINDOW_SIZE, self.p_fa));
        Availability::Available
    }

    fn gate(&mut self, hop: &HopCtx, probe: &GateProbe) -> GateVerdict {
        let t = kay_gate(self.sigma(hop), probe.window, self.p_fa);
        GateVerdict {
            threshold: Some(t),
            estimate: Some(t / median_to_threshold(self.p_fa)),
            interval: None,
            admit: Some(probe.amplitude >= t),
        }
    }

    fn bg_floor(
        &mut self,
        cap: &CaptureCtx,
        _spectrum: &[f32],
        _hz_per_bin: f32,
        partials: &[f32],
        out: &mut [f32],
    ) -> Availability {
        // The same rule extended to the Worker's window: the median level the
        // scalar implies for a 2¹⁶-point Hann frame.
        let sigma = self.fixed.unwrap_or(cap.sigma_rec);
        let level =
            kay_raw(sigma, CAPTURE_ANALYSIS_SAMPLES, self.p_fa) / median_to_threshold(self.p_fa);
        for (o, _) in out.iter_mut().zip(partials) {
            *o = level;
        }
        Availability::Available
    }
}

/// Arm B: the oracle, `3.157 × N_local` on a real set (report 0015 §1, valid from
/// the tenor up) and the exact noise level on a synthetic one. `T ÷ T_ideal ≡ 1`
/// by construction; the bound a perfect local reference approaches.
pub(super) struct Oracle {
    p_fa: f32,
}

impl Oracle {
    pub(super) fn new() -> Self {
        Self { p_fa: 1.0e-3 }
    }
}

impl Arm for Oracle {
    fn name(&self) -> &str {
        "B"
    }

    fn consumers(&self) -> &[Consumer] {
        &[Consumer::G1, Consumer::G2, Consumer::G3]
    }

    fn prepare(&mut self, set: &SetCtx) {
        self.p_fa = set.p_fa;
    }

    fn g1_threshold(&mut self, hop: &HopCtx, out: &mut [f32]) -> Availability {
        let k = median_to_threshold(self.p_fa);
        if let Some(level) = hop.truth_level {
            for (o, &l) in out.iter_mut().zip(level) {
                *o = if l.is_finite() { k * l } else { f32::INFINITY };
            }
            return Availability::Available;
        }
        // On a real set the oracle exists only at the partials.
        let cap = hop.capture;
        if cap.register == 0 || hop.oracle.is_empty() {
            return Availability::Unavailable;
        }
        for (i, &f) in cap.f_meas.iter().enumerate() {
            let Some(o) = hop.oracle.get(i).copied().flatten() else {
                continue;
            };
            let c = (f / hop.hz_per_bin).round() as usize;
            for b in c.saturating_sub(LOBE_HALF_BINS)..=(c + LOBE_HALF_BINS).min(out.len() - 1) {
                out[b] = k * o;
            }
        }
        Availability::Available
    }

    fn gate(&mut self, hop: &HopCtx, probe: &GateProbe) -> GateVerdict {
        let level = match (probe.truth_level, hop.truth_level) {
            (Some(l), _) => Some(l),
            (None, Some(lvl)) => {
                let b = (probe.target_hz / hop.hz_per_bin).round() as usize;
                lvl.get(b)
                    .copied()
                    .filter(|v| v.is_finite())
                    .map(|v| raw_to_gate(v, probe.window))
            }
            (None, None) if hop.capture.register > 0 => hop
                .oracle
                .get(probe.index)
                .copied()
                .flatten()
                .map(|o| raw_to_gate(o, probe.window)),
            (None, None) => None,
        };
        match level {
            Some(l) => {
                let t = median_to_threshold(self.p_fa) * l;
                GateVerdict {
                    threshold: Some(t),
                    estimate: Some(l),
                    interval: None,
                    admit: Some(probe.amplitude >= t),
                }
            }
            None => GateVerdict::default(),
        }
    }
}

/// Arm C: the coarse read's ordered-statistic CFAR (Rohling 1983; `peaks::coarse_read`)
/// as a per-bin gate for the picker: at every local maximum, a band of ±4 bins
/// capped at half the key's partial spacing, reference cells in flanks of 1.5
/// spacings floored at 172 Hz, the 0.25 quantile, the finite-N multiplier with
/// the search-loss budget `P_fa / M` over the band. Only the picker: a Goertzel
/// has no reference cells. Its budget is per band, so its per-bin rate sits
/// below the operating point by the band's independent cells.
pub(super) struct Cfar {
    p_fa: f32,
    /// Rohling's multiplier by (reference count, rank, band bins): a bisection
    /// each, and only a few distinct triples occur in a set.
    multipliers: std::collections::HashMap<(usize, usize, usize), f32>,
    scratch: Vec<f32>,
}

impl Cfar {
    pub(super) fn new() -> Self {
        Self {
            p_fa: 1.0e-3,
            multipliers: std::collections::HashMap::new(),
            scratch: Vec::new(),
        }
    }
}

const CFAR_QUANTILE: f32 = 0.25;
const CFAR_FLANK_SPACINGS: f32 = 1.5;
const CFAR_FLANK_MIN_HZ: f32 = 172.0;
const CFAR_MIN_REFS: usize = 4;
const CFAR_SPAN_MIN_BINS: f32 = 4.0;
const CFAR_SPAN_CENTS: f32 = 100.0;

/// The threshold `coarse_read` would apply to the band around `bin` at `p_fa`,
/// in the spectrum's units, or `INFINITY` where the reference set is too small.
/// `multipliers` caches the finite-N factor by (reference count, rank, band bins).
pub(super) fn cfar_threshold_at(
    mag: &[f32],
    bin: usize,
    spacing_hz: f32,
    hz_per_bin: f32,
    p_fa: f32,
    scratch: &mut Vec<f32>,
    multipliers: &mut std::collections::HashMap<(usize, usize, usize), f32>,
) -> f32 {
    let center_hz = bin as f32 * hz_per_bin;
    let span = center_hz * (2f32.powf(CFAR_SPAN_CENTS / 1200.0) - 1.0);
    let half = span
        .max(CFAR_SPAN_MIN_BINS * hz_per_bin)
        .min(spacing_hz / 2.0);
    let n_bins = mag.len();
    let lo = ((center_hz - half) / hz_per_bin).floor().max(1.0) as usize;
    let hi = (((center_hz + half) / hz_per_bin).ceil() as usize).min(n_bins - 2);
    if lo >= hi {
        return f32::INFINITY;
    }
    let flank_hz = (CFAR_FLANK_SPACINGS * spacing_hz).max(CFAR_FLANK_MIN_HZ);
    let flank = (flank_hz / hz_per_bin).ceil() as usize;
    let outer_lo = lo.saturating_sub(flank).max(1);
    let outer_hi = hi.saturating_add(flank).min(n_bins - 2);
    scratch.clear();
    scratch.extend((outer_lo..lo).chain((hi + 1)..=outer_hi).map(|b| mag[b]));
    let n_ref = scratch.len();
    if n_ref < CFAR_MIN_REFS {
        return f32::INFINITY;
    }
    let rank = (((n_ref - 1) as f32 * CFAR_QUANTILE).round() as usize).clamp(1, n_ref - 1);
    let (_, noise, _) = scratch.select_nth_unstable_by(rank, f32::total_cmp);
    let noise = *noise;
    let band = hi - lo;
    let mult = *multipliers.entry((n_ref, rank, band)).or_insert_with(|| {
        let m_eff = band.div_ceil(2).max(1) as f32;
        cfar_multiplier_finite((n_ref / 2).max(2), (rank / 2).max(1), p_fa / m_eff)
    });
    noise * mult
}

impl Arm for Cfar {
    fn name(&self) -> &str {
        "C"
    }

    fn consumers(&self) -> &[Consumer] {
        &[Consumer::G1]
    }

    fn prepare(&mut self, set: &SetCtx) {
        self.p_fa = set.p_fa;
        self.multipliers.clear();
    }

    fn g1_threshold(&mut self, hop: &HopCtx, out: &mut [f32]) -> Availability {
        // The band cap and the flanks are set by the key's spacing; without one
        // (the picker's case) there is no reference geometry to apply.
        if !hop.capture.spacing_hz.is_finite() {
            return Availability::Unavailable;
        }
        let mag = hop.mag;
        for b in 1..mag.len() - 1 {
            if mag[b] > mag[b - 1] && mag[b] > mag[b + 1] {
                out[b] = cfar_threshold_at(
                    mag,
                    b,
                    hop.capture.spacing_hz,
                    hop.hz_per_bin,
                    self.p_fa,
                    &mut self.scratch,
                    &mut self.multipliers,
                );
            }
        }
        Availability::Available
    }
}

/// Arm N: no gate. Every local maximum is a candidate and every probe is
/// admitted: the floor every rate is read against.
pub(super) struct NoGate;

impl Arm for NoGate {
    fn name(&self) -> &str {
        "N"
    }

    fn consumers(&self) -> &[Consumer] {
        &[Consumer::G1, Consumer::G2, Consumer::G3, Consumer::Bg]
    }

    fn g1_threshold(&mut self, _hop: &HopCtx, out: &mut [f32]) -> Availability {
        out.fill(0.0);
        Availability::Available
    }

    fn gate(&mut self, _hop: &HopCtx, probe: &GateProbe) -> GateVerdict {
        GateVerdict {
            threshold: Some(0.0),
            estimate: None,
            interval: None,
            admit: Some(probe.amplitude > 0.0),
        }
    }

    fn bg_floor(
        &mut self,
        _cap: &CaptureCtx,
        _spectrum: &[f32],
        _hz_per_bin: f32,
        partials: &[f32],
        out: &mut [f32],
    ) -> Availability {
        for (o, _) in out.iter_mut().zip(partials) {
            *o = 0.0;
        }
        Availability::Available
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::truth::SAMPLE_RATE;
    use realfft::RealFftPlanner;
    use tuner_core::algorithms::peaks;
    use tuner_core::algorithms::spectral::{fft, magnitude_spectrum, neyman_pearson_k};

    /// The generalised Kay thresholds reproduce the shipped constants at 10⁻³.
    #[test]
    fn kay_matches_the_shipped_gates() {
        let sigma = 2.5e-3;
        assert!((kay_gate(sigma, 1024, 1e-3) - sigma * neyman_pearson_k(1024)).abs() < 1e-7);
        assert!((kay_gate(sigma, 4096, 1e-3) - sigma * neyman_pearson_k(4096)).abs() < 1e-7);
        let engine = (sigma * sigma * 0.375 * BASS_WINDOW_SIZE as f32 * -(0.001f32.ln())).sqrt();
        assert!((kay_raw(sigma, BASS_WINDOW_SIZE, 1e-3) - engine).abs() < 1e-5);
    }

    /// The per-bin form agrees with the shipped `coarse_read` on whether a tone
    /// is admitted, at the band's own argmax, over a sweep of levels: the two
    /// read the same reference cells and the same multiplier.
    #[test]
    fn cfar_per_bin_matches_the_shipped_read() {
        let n = BASS_WINDOW_SIZE;
        let fs = SAMPLE_RATE;
        let plan = RealFftPlanner::<f32>::new().plan_fft_forward(n);
        let mut time = vec![0.0f32; n];
        let mut spec = vec![rustfft::num_complex::Complex::new(0.0, 0.0); n / 2 + 1];
        let mut mag = vec![0.0f32; n / 2];
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut noise = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f32 / (1u64 << 53) as f32 - 0.5
        };
        let f_tone = 440.0f32;
        let spacing = 440.0f32;
        let hz_per_bin = fs as f32 / n as f32;
        let mut agree = 0;
        let mut trials = 0;
        for amp_db in -40..=10 {
            let amp = 10f32.powf(amp_db as f32 / 20.0) * 0.01;
            let signal: Vec<f32> = (0..n)
                .map(|i| {
                    amp * (2.0 * std::f32::consts::PI * f_tone * i as f32 / fs as f32).sin()
                        + 0.01 * noise()
                })
                .collect();
            fft(&signal, &mut time, &mut spec, &plan, n);
            magnitude_spectrum(&spec, n, &mut mag);
            let mut shipped_scratch = vec![0.0f32; mag.len()];
            let shipped =
                peaks::coarse_read(&mag, &spec, n, fs, f_tone, spacing, &mut shipped_scratch);
            // The band's argmax, as `coarse_read` finds it.
            let half = (f_tone * (2f32.powf(100.0 / 1200.0) - 1.0))
                .max(4.0 * hz_per_bin)
                .min(spacing / 2.0);
            let lo = ((f_tone - half) / hz_per_bin).floor() as usize;
            let hi = ((f_tone + half) / hz_per_bin).ceil() as usize;
            let best = (lo..=hi)
                .max_by(|&a, &b| mag[a].total_cmp(&mag[b]))
                .unwrap();
            let mut scratch = Vec::new();
            let mut cache = std::collections::HashMap::new();
            let t = cfar_threshold_at(
                &mag,
                best,
                spacing,
                hz_per_bin,
                1e-3,
                &mut scratch,
                &mut cache,
            );
            trials += 1;
            if (mag[best] >= t) == shipped.is_some() {
                agree += 1;
            }
        }
        assert_eq!(
            agree, trials,
            "per-bin CFAR and coarse_read disagree on admission"
        );
    }
}
