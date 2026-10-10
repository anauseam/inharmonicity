//! Arm 1: a per-bin floor learned between notes — the mean power of each bin
//! over the Gatekeeper-silent hops seen so far in the session, updated only on
//! hops it may learn from, thresholded by the finite-N law of a floor that is
//! itself an estimate (Nitzberg 1986's clutter map in its gated form; Talbot &
//! Thrane 2020 for the law).
//!
//! The multiplier is the Student-Rayleigh one: the ratio of one periodogram
//! value to the mean of `M` independent ones is `F(2, 2M)`, so the threshold on
//! a bin of mean power `P̄` is `√(P̄ · M(P_fa^(−1/M) − 1))`, with `M` the
//! equivalent independent periodograms in the hops averaged. Hann frames at a
//! hop of an eighth overlap seven eighths, so a contiguous run of `k` hops is
//! worth fewer than `k`; runs from different captures are independent, so `M`
//! is summed over runs.

use super::{
    Arm, Availability, Block, CaptureCtx, Consumer, GateProbe, GateVerdict, HopCtx, SetCtx,
    analytic_nu_half, raw_to_gate,
};
use crate::gates::stats;
use tuner_core::audio::{BASS_WINDOW_SIZE, SAMPLE_RATE};
use tuner_core::pipeline::CAPTURE_ANALYSIS_SAMPLES;

/// Fewest silent hops before the floor answers: under this the finite-N
/// multiplier exceeds twice the known-σ one.
const MIN_HOPS: usize = 8;

pub(super) struct RunningFloor {
    /// Σ|X|² per bin over the hops learned from, in the raw frame's units.
    sum: Vec<f64>,
    hops: usize,
    /// Lengths of the contiguous runs of hops learned from, the open run last.
    runs: Vec<usize>,
    last_hop: Option<i64>,
    /// ν/2 per hop of averaging as Stage B measured it, or the analytic value.
    nu_half_per_hop: Option<f32>,
    /// `m_eff` for the hops learned so far, refreshed when one is added.
    m_eff: f32,
    p_fa: f32,
}

impl RunningFloor {
    pub(super) fn new() -> Self {
        Self {
            sum: vec![0.0; BASS_WINDOW_SIZE / 2],
            hops: 0,
            runs: Vec::new(),
            last_hop: None,
            nu_half_per_hop: None,
            m_eff: 0.0,
            p_fa: 1.0e-3,
        }
    }

    /// Equivalent independent periodograms in the hops averaged so far.
    fn refresh_m_eff(&mut self) {
        self.m_eff = match self.nu_half_per_hop {
            Some(per_hop) => per_hop * self.hops as f32,
            None => self.runs.iter().map(|&k| analytic_nu_half(k)).sum(),
        };
    }

    /// Median level at `bin`, the arm's threshold and its 5–95 % interval on
    /// the level, in the raw frame's magnitude units.
    fn at_bin(&self, bin: usize) -> Option<(f32, f32, (f32, f32))> {
        if self.hops < MIN_HOPS {
            return None;
        }
        let m = self.m_eff;
        let mean_power = (self.sum[bin] / self.hops as f64) as f32;
        let threshold =
            (mean_power * stats::f_2_2m_threshold(m as f64, self.p_fa as f64) as f32).sqrt();
        let median = stats::rayleigh_median(mean_power as f64) as f32;
        // The mean of M exponentials scatters as Gamma(M): its 5–95 % band in
        // power, by the normal approximation at these M.
        let rel = 1.645 / m.sqrt();
        let lo = (median * median * (1.0 - rel).max(0.05)).sqrt();
        let hi = (median * median * (1.0 + rel)).sqrt();
        Some((median, threshold, (lo, hi)))
    }
}

impl Arm for RunningFloor {
    fn name(&self) -> &str {
        "1"
    }

    fn consumers(&self) -> &[Consumer] {
        &[Consumer::G1, Consumer::G2, Consumer::G3, Consumer::Bg]
    }

    fn prepare(&mut self, set: &SetCtx) {
        self.sum.iter_mut().for_each(|s| *s = 0.0);
        self.hops = 0;
        self.runs.clear();
        self.last_hop = None;
        self.nu_half_per_hop = set.nu_half_per_frame;
        self.m_eff = 0.0;
        self.p_fa = set.p_fa;
        println!(
            "[{}] arm 1: the floor starts empty and learns from the build half of each capture's silence; ν/2 per frame {}",
            set.label,
            match set.nu_half_per_frame {
                Some(v) => format!("{v:.3} (Stage B)"),
                None => "analytic Hann, summed over runs".to_string(),
            }
        );
    }

    fn begin_capture(&mut self, _cap: &CaptureCtx) {
        self.last_hop = None;
    }

    fn observe(&mut self, hop: &HopCtx) {
        // Learn from silence only, and only from the half of it that is not
        // scored; the note's hops never enter.
        if hop.is_silence && hop.block == Block::Build {
            for (s, &m) in self.sum.iter_mut().zip(hop.mag) {
                *s += (m as f64) * (m as f64);
            }
            self.hops += 1;
            match self.last_hop {
                Some(prev) if hop.hop == prev + 1 => {
                    *self.runs.last_mut().expect("an open run") += 1
                }
                _ => self.runs.push(1),
            }
            self.last_hop = Some(hop.hop);
            self.refresh_m_eff();
        }
    }

    fn g1_threshold(&mut self, _hop: &HopCtx, out: &mut [f32]) -> Availability {
        if self.hops < MIN_HOPS {
            return Availability::Unavailable;
        }
        for (b, o) in out.iter_mut().enumerate() {
            if let Some((_, t, _)) = self.at_bin(b) {
                *o = t;
            }
        }
        Availability::Available
    }

    fn gate(&mut self, hop: &HopCtx, probe: &GateProbe) -> GateVerdict {
        let bin = (probe.target_hz / hop.hz_per_bin).round() as usize;
        if bin >= self.sum.len() {
            return GateVerdict::default();
        }
        match self.at_bin(bin) {
            Some((median, threshold, (lo, hi))) => {
                // The floor's level and threshold at the Goertzel's window.
                let t = raw_to_gate(threshold, probe.window);
                GateVerdict {
                    threshold: Some(t),
                    estimate: Some(raw_to_gate(median, probe.window)),
                    interval: Some((raw_to_gate(lo, probe.window), raw_to_gate(hi, probe.window))),
                    admit: Some(probe.amplitude >= t),
                }
            }
            None => GateVerdict::default(),
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
        if self.hops < MIN_HOPS {
            return Availability::Unavailable;
        }
        // The 8192-point floor's power at a 2¹⁶-point bin scales with the
        // window's Σw², eight times, so the magnitude by √8.
        let scale = (CAPTURE_ANALYSIS_SAMPLES as f32 / BASS_WINDOW_SIZE as f32).sqrt();
        let coarse_hz = SAMPLE_RATE as f32 / BASS_WINDOW_SIZE as f32;
        for (o, &f) in out.iter_mut().zip(partials) {
            let bin = ((f / coarse_hz).round() as usize).min(self.sum.len() - 1);
            *o = self
                .at_bin(bin)
                .map(|(median, _, _)| median * scale)
                .unwrap_or(f32::NAN);
        }
        Availability::Available
    }
}
