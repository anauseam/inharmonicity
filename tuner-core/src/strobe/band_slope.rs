//! # Band slope — the strobe band's rotation *rate*
//!
//! A sliding-window ordinary-least-squares fit of one reference's unwrapped
//! beat phase (cycles) against hop index. Rotation is exactly `f_live − f_ref`
//! (strobe design §3), so the slope is the detuning in Hz — the fine half of
//! the readout pair, phase-integrated and therefore far steadier than a per-hop
//! frequency estimate.
//!
//! The hop cadence is regular, so the abscissae are a fixed integer sequence:
//! the normal equations reduce to running sums, and the window's span follows
//! from its point count.

use crate::audio::HOP_RATE_HZ;

/// Sliding window the band-slope fit spans (strobe design §5.5 / R12).
///
/// The trade is **linear cost against an inelastic benefit** (ADR 0011 §11): the
/// fit lags a turning peg by exactly `T/2` — an OLS slope estimates the rate at
/// the window's *midpoint* — while the jitter it buys falls far slower than the
/// `T⁻³` law, because the bank's analysis windows overlap 75–87 % and extra hops
/// are mostly the same audio. Measured, tripling `T` buys 1.8× on piano and
/// nothing measurable on guitar.
///
/// 0.6 s is the responsive-enough end of that trade in use, and 1 ¢ of steadiness
/// is inside the range a professional tuner works in. To trade the 300 ms of lag
/// back, shorten this; two lengths are derived rather than arbitrary, and each
/// *removes* a constant by absorbing [`BAND_SLOPE_MIN_SPAN_SECS`] (which can never
/// exceed it): `BASS_WINDOW_SIZE / SAMPLE_RATE` ≈ 0.186 s matches the coarse
/// read's own group delay, so the two readouts lag equally and the source switch
/// stops stepping the number; 0.25 s simply collapses the two constants. Escaping
/// the trade rather than moving along it needs a two-state (g–h / Kalman)
/// estimator on (phase, rate), which weights old data instead of discarding it.
pub const BAND_SLOPE_WINDOW_SECS: f32 = 0.6;

/// Points the fit retains — one per hop across [`BAND_SLOPE_WINDOW_SECS`], plus
/// the current hop.
pub const BAND_SLOPE_POINTS: usize = (BAND_SLOPE_WINDOW_SECS * HOP_RATE_HZ) as usize + 1;

/// Minimum span before a rate is published; the coarse read covers the fill-in.
///
/// Floored by the analysis window itself: the bank integrates over
/// `BASS_WINDOW_SIZE` samples ≈ 0.186 s, so a fit spanning less than that is
/// drawing a line through repeats of *one* window's audio and has no independent
/// information about the rate. This value clears that floor by 1.34×
/// (`span_clears_the_analysis_window` pins it).
pub const BAND_SLOPE_MIN_SPAN_SECS: f32 = 0.25;

/// Points needed to span [`BAND_SLOPE_MIN_SPAN_SECS`]: `n` points bridge `n − 1`
/// hop intervals, and the truncating cast has to round *up* to reach the span —
/// hence `+ 2`, which `point_counts_bracket_their_durations` pins in both
/// directions.
pub const BAND_SLOPE_MIN_POINTS: usize = (BAND_SLOPE_MIN_SPAN_SECS * HOP_RATE_HZ) as usize + 2;

/// Sliding-window OLS fit of one reference's unwrapped beat phase, indexed by hop.
pub(super) struct BandSlope {
    /// Retained unwrapped phase, oldest at `head`.
    y: [f32; BAND_SLOPE_POINTS],
    head: usize,
    len: usize,
    /// `Σ y_j` and `Σ j·y_j` over the retained points, `j = 0` at the oldest.
    sum_y: f64,
    sum_jy: f64,
    /// Running unwrapped phase (cycles). Rebased at every restart so it cannot
    /// grow past f32's useful precision during a long sustain.
    unwrapped: f32,
    /// Last published slope, cycles per hop.
    pub(super) rate: Option<f32>,
    /// A gated hop breaks the run; the next live hop restarts the fit.
    restart: bool,
}

impl BandSlope {
    pub(super) const fn new() -> Self {
        Self {
            y: [0.0; BAND_SLOPE_POINTS],
            head: 0,
            len: 0,
            sum_y: 0.0,
            sum_jy: 0.0,
            unwrapped: 0.0,
            rate: None,
            restart: true,
        }
    }

    /// Drops the fit and its published rate, and breaks the run so the next
    /// live hop starts a fresh baseline. Points past `len` are never read, so
    /// the ring itself needs no clearing.
    pub(super) fn reset(&mut self) {
        self.head = 0;
        self.len = 0;
        self.sum_y = 0.0;
        self.sum_jy = 0.0;
        self.unwrapped = 0.0;
        self.rate = None;
        self.restart = true;
    }

    /// Gated hop: hold the last rate — a frozen band's angle does not advance,
    /// so integrating the gap would read the detuning toward zero — and break
    /// the run so a re-strike resumes without a phantom jump across it.
    pub(super) fn hold(&mut self) {
        self.restart = true;
    }

    /// Live hop: `cycles` is the wrapped beat-phase advance since the last hop.
    pub(super) fn push(&mut self, cycles: f32) {
        if self.restart {
            self.reset();
            self.restart = false;
        } else {
            self.unwrapped += cycles;
        }

        if self.len == BAND_SLOPE_POINTS {
            // Evicting the oldest point shifts every survivor one place down,
            // so `Σ j·y` loses the whole of `Σ y` less the evicted term.
            let old = self.y[self.head] as f64;
            self.sum_jy -= self.sum_y - old;
            self.sum_y -= old;
            self.head = (self.head + 1) % BAND_SLOPE_POINTS;
            self.len -= 1;
        }
        let j = self.len as f64;
        self.y[(self.head + self.len) % BAND_SLOPE_POINTS] = self.unwrapped;
        self.sum_y += self.unwrapped as f64;
        self.sum_jy += j * self.unwrapped as f64;
        self.len += 1;

        if self.len >= BAND_SLOPE_MIN_POINTS {
            self.rate = Some(self.slope());
        }
    }

    /// OLS slope in cycles per hop. The abscissae are `0..n`, so
    /// `n·Σj² − (Σj)²` reduces to `n²(n²−1)/12` — nonzero for every `n ≥ 2`,
    /// which [`BAND_SLOPE_MIN_POINTS`] guarantees.
    fn slope(&self) -> f32 {
        debug_assert!(self.len >= 2, "a slope needs two points");
        let n = self.len as f64;
        let sum_j = n * (n - 1.0) / 2.0;
        ((n * self.sum_jy - sum_j * self.sum_y) / (n * n * (n * n - 1.0) / 12.0)) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::BASS_WINDOW_SIZE;

    /// The fit's minimum span must clear one analysis window, or it is drawing a
    /// line through repeats of the same audio (ADR 0011 §11).
    #[test]
    fn span_clears_the_analysis_window() {
        let analysis_secs = BASS_WINDOW_SIZE as f32 / 44_100.0;
        assert!(
            BAND_SLOPE_MIN_SPAN_SECS >= analysis_secs,
            "min span {BAND_SLOPE_MIN_SPAN_SECS} s is under one analysis window ({analysis_secs} s)"
        );
    }

    /// Both point counts are truncating casts of a duration, so pin them from
    /// both sides: the retained window must cover its seconds, and the minimum
    /// must be the *first* count that spans its own.
    #[test]
    fn point_counts_bracket_their_durations() {
        let span = |points: usize| (points - 1) as f32 / HOP_RATE_HZ;
        assert!(span(BAND_SLOPE_POINTS) <= BAND_SLOPE_WINDOW_SECS);
        assert!(span(BAND_SLOPE_POINTS + 1) > BAND_SLOPE_WINDOW_SECS);
        assert!(span(BAND_SLOPE_MIN_POINTS) >= BAND_SLOPE_MIN_SPAN_SECS);
        assert!(span(BAND_SLOPE_MIN_POINTS - 1) < BAND_SLOPE_MIN_SPAN_SECS);
    }

    /// The running sums must reproduce a textbook OLS over the retained points,
    /// evictions included — the fit's whole correctness argument.
    #[test]
    fn matches_a_direct_fit() {
        let mut band = BandSlope::new();
        let mut pushed: Vec<f64> = Vec::new();
        let mut y = 0.0f32;
        for h in 0..BAND_SLOPE_POINTS * 3 {
            // Curved, so a mis-weighted eviction cannot cancel out.
            let cycles = 0.02 + 0.004 * (h as f32 * 0.3).sin();
            if h > 0 {
                y += cycles;
            }
            band.push(cycles);
            pushed.push(y as f64);

            let w = &pushed[pushed.len().saturating_sub(BAND_SLOPE_POINTS)..];
            assert_eq!(band.len, w.len(), "retained count at hop {h}");
            if w.len() < BAND_SLOPE_MIN_POINTS {
                assert!(band.rate.is_none(), "no rate before the minimum span");
                continue;
            }
            let n = w.len() as f64;
            let (sum_j, sum_y) = (n * (n - 1.0) / 2.0, w.iter().sum::<f64>());
            let sum_jy: f64 = w.iter().enumerate().map(|(j, v)| j as f64 * v).sum();
            let sum_jj: f64 = (0..w.len()).map(|j| (j * j) as f64).sum();
            let direct = (n * sum_jy - sum_j * sum_y) / (n * sum_jj - sum_j * sum_j);
            let got = band.rate.expect("rate past the minimum span") as f64;
            assert!(
                (got - direct).abs() < 1e-6,
                "hop {h}: incremental {got} vs direct {direct}"
            );
        }
    }

    /// A gated run holds the last rate and then drops it: the first live hop
    /// after the gap restarts the fit, so a re-strike shows nothing until the
    /// window refills rather than a stale rate.
    #[test]
    fn holds_then_restarts_across_a_gate() {
        let mut band = BandSlope::new();
        for _ in 0..BAND_SLOPE_MIN_POINTS + 1 {
            band.push(0.02);
        }
        let held = band.rate.expect("filled");
        band.hold();
        assert_eq!(band.rate, Some(held), "a gated hop holds the last rate");
        band.push(0.02);
        assert!(band.rate.is_none(), "the re-strike restarts the fit");
        assert_eq!(band.len, 1);
    }
}
