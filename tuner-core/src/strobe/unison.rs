//! # Unison — the note's individual strings, resolved as spectral lines
//!
//! A multi-strung note must have its strings zero-beat against each other, and
//! the strobe's own front end already carries what that needs. Keeping the
//! per-reference Goertzel's amplitude *with* its phase gives a complex baseband
//! sampled once per hop; its spectrum resolves the strings as separate lines,
//! each a signed offset from the curve target. The beat rate a tuner listens for
//! is then the difference between any two of them.
//!
//! What this file owns is the cross-hop half: one growing baseband ring per
//! reference, held and restarted on the D3 gate exactly as
//! [`BandSlope`](super::band_slope) is — and dropped outright when the
//! Gatekeeper reports silence — and the goodness-of-fit test that decides
//! whether what it resolved is a unison at all. The estimator itself is stateless
//! and lives in [`peaks::resolve_lines`].
//!
//! **What it measures is lines; what it is for is unisons.** A *false beat* — one
//! string beating with itself, because the bridge's mechanical impedance splits a
//! partial's two transverse polarizations, or a defect or a soundboard mode does
//! (Weinreich 1977; strobe design §4) — presents identically to a second string.
//! [`Unison::verdict`] is what separates them, and it has to ship: without it the
//! bass display would label a false beat a unison on essentially every bass key
//! of both instruments (ADR 0012 §5).
//!
//! Resolution is set by observation time, so the ring **grows**: a 4 Hz split
//! resolves in ≈0.5 s, 1 Hz needs ≈2 s. Until it is long enough, two separated
//! strings report as *one line* — which reads as "clean" at exactly the moment a
//! tuner decides they are finished. [`Unison::resolution_hz`] is published for
//! that reason and is not cosmetic.

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::algorithms::peaks::{self, LineScratch, MAX_UNISON_LINES, UNISON_MIN_BINS};
use crate::algorithms::spectral;
use crate::audio::HOP_RATE_HZ;
use crate::models::UnisonLine;
use crate::strobe::MAX_STROBE_REFS;

/// Longest baseband record a reference accumulates before the ring slides.
///
/// A cap exists at all because of **Weinreich coupling**: two strings on a shared
/// bridge exchange energy rather than ringing independently, so the beat is not
/// stationary over a long window and more observation eventually stops buying
/// resolution. Where to put it is measured, not derived — a 0.65 s ring is worse
/// on both availability *and* bias (short records are biased high by
/// survivorship: close pairs merge, so only wide ones get reported), and nothing
/// longer has real-data support, the capture sets themselves being 1.5 s
/// (ADR 0012 §4).
pub const UNISON_RING_SECS: f32 = 1.30;

/// [`UNISON_RING_SECS`] in hops — the transform length at the cap. Rounded, not
/// truncated: the duration is the quantity that was measured, and the ±½-hop
/// either side of it is meaningless. `ring_cap_matches_its_duration` pins both.
pub const UNISON_RING_HOPS: usize = (UNISON_RING_SECS * HOP_RATE_HZ + 0.5) as usize;

/// Per-line frequency scatter, as a fraction of the transform's **bin width** —
/// the floor under the discriminator's estimated uncertainty.
///
/// Ours, measured (ADR 0012 §3): ≈0.05 Hz per line at the 56-point ring, whose
/// bins are 0.769 Hz apart, and essentially independent of SNR from 40 dB down
/// to 6 dB. Expressed as a fraction of a bin rather than in Hz because the ring
/// grows: an interpolated-DFT estimator's scatter is a roughly fixed fraction of
/// a bin at fixed SNR, so this form drifts with the record length while an
/// absolute figure would under-state σ on a short ring — exactly where the test
/// would then over-reject.
///
/// It is a **floor**, never the operating value: the split's scatter across
/// partials is physical as well as instrumental, and is measured to run several
/// times this (ADR 0012 §6).
const UNISON_LINE_SIGMA_BINS: f32 = 0.065;

/// Standard errors the fitted exponent must sit within of one hypothesis, and
/// outside of the other, before the discriminator commits to a verdict.
///
/// A significance level, in the same role as the `P_fa` the project's detection
/// gates carry, and dimensionless for the same reason. Three rather than two
/// because the two failure directions are not symmetric in cost: an
/// [`UnisonVerdict::Undetermined`] leaves the panel showing its per-partial
/// splits, which is what the tuner would read anyway, while a wrong verdict is
/// an assertion about the instrument.
const UNISON_FIT_SIGMAS: f32 = 3.0;

/// What the discriminator concluded about the lines this hop, over the whole
/// reference bank.
///
/// A **unison** is two strings at different f₀, so both strings' partials scale
/// together and the split, expressed in cents, is *constant across partials*. A
/// **false beat** is a mode splitting of a single partial and has no reason to be.
/// That is the whole test — see [`Unison::discriminate`] for the form it takes,
/// which is a comparison of two models rather than a threshold on the spread.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UnisonVerdict {
    /// The splits do not separate the two hypotheses — too few comparable
    /// partials, or too little spread in frequency between them. Not evidence
    /// either way, and the common answer over a short reference set.
    #[default]
    Undetermined,
    /// The splits are consistent with one constant cents interval across the
    /// partials that resolved: strings at different pitches.
    Unison,
    /// The splits are consistent with a separation fixed in **Hz** and not with
    /// one proportional to the partial frequency — which a pair of strings
    /// cannot produce.
    FalseBeat,
}

/// Per-reference baseband rings, their resolved lines, and the discriminator.
pub(super) struct Unison {
    /// Baseband record per reference, **oldest first** in `[..len]`.
    ring: [[Complex<f32>; UNISON_RING_HOPS]; MAX_STROBE_REFS],
    len: [u8; MAX_STROBE_REFS],
    /// A gated hop breaks the run; the next live hop restarts the ring.
    restart: [bool; MAX_STROBE_REFS],
    /// Last published lines, strongest first.
    lines: [[UnisonLine; MAX_UNISON_LINES]; MAX_STROBE_REFS],
    line_count: [u8; MAX_STROBE_REFS],
    /// `2/T` per reference (Hz) — what the current record is worth. `0.0` while
    /// nothing is published.
    resolution_hz: [f32; MAX_STROBE_REFS],
    verdict: UnisonVerdict,

    /// One transform per supported record length, planned at startup; index
    /// `n − UNISON_MIN_BINS`. The hot path allocates nothing, and `rustfft`'s
    /// planner allocates on every `plan_*` call.
    plans: Vec<Arc<dyn Fft<f32>>>,
    /// [`spectral::candan_c_n`] per supported length, parallel to `plans`.
    /// Evaluating Eq. 12 is `O(N)` in trigonometry and it is constant per length.
    c_n: Vec<f32>,

    spectrum: Box<[Complex<f32>]>,
    magnitudes: Box<[f32]>,
    fft_scratch: Box<[Complex<f32>]>,
}

impl Unison {
    /// Builds the rings and plans every transform length the ring can reach.
    pub(super) fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let plans: Vec<Arc<dyn Fft<f32>>> = (UNISON_MIN_BINS..=UNISON_RING_HOPS)
            .map(|n| planner.plan_fft_forward(n))
            .collect();
        let c_n = (UNISON_MIN_BINS..=UNISON_RING_HOPS)
            .map(spectral::candan_c_n)
            .collect();
        let scratch_len = plans
            .iter()
            .map(|p| p.get_inplace_scratch_len())
            .max()
            .unwrap_or(0);

        Self {
            ring: [[Complex::new(0.0, 0.0); UNISON_RING_HOPS]; MAX_STROBE_REFS],
            len: [0; MAX_STROBE_REFS],
            restart: [true; MAX_STROBE_REFS],
            lines: [[UnisonLine::default(); MAX_UNISON_LINES]; MAX_STROBE_REFS],
            line_count: [0; MAX_STROBE_REFS],
            resolution_hz: [0.0; MAX_STROBE_REFS],
            verdict: UnisonVerdict::Undetermined,
            plans,
            c_n,
            spectrum: vec![Complex::new(0.0, 0.0); UNISON_RING_HOPS].into_boxed_slice(),
            magnitudes: vec![0.0; UNISON_RING_HOPS].into_boxed_slice(),
            fft_scratch: vec![Complex::new(0.0, 0.0); scratch_len].into_boxed_slice(),
        }
    }

    /// Drops every record and everything published from it (retarget: the
    /// baseband is defined against reference frequencies that no longer apply).
    pub(super) fn reset(&mut self) {
        self.len = [0; MAX_STROBE_REFS];
        self.restart = [true; MAX_STROBE_REFS];
        self.line_count = [0; MAX_STROBE_REFS];
        self.resolution_hz = [0.0; MAX_STROBE_REFS];
        self.verdict = UnisonVerdict::Undetermined;
    }

    /// Gated hop: hold what was last published — a frozen band's baseband does
    /// not advance — and break the run so a re-strike starts a fresh record
    /// rather than splicing across the gap.
    pub(super) fn hold(&mut self, i: usize) {
        self.restart[i] = true;
    }

    /// Silent hop: break the run as [`Self::hold`] does, and drop what was
    /// published with it. The hold exists for a dip below the gate *within* a
    /// note; with no note sounding there are no strings for the lines to be
    /// about.
    pub(super) fn clear(&mut self, i: usize) {
        self.hold(i);
        self.line_count[i] = 0;
        self.resolution_hz[i] = 0.0;
    }

    /// Live hop: append this reference's baseband sample and re-resolve.
    ///
    /// `z` is the demodulated Goertzel value `A·e^{j2πθ}`, with `θ` the bank's
    /// accumulated beat phase. That is the demodulation of design §3 step 2 and
    /// not an approximation of it: the accumulated angle differs from
    /// `φ_h − 2π·f_ref·h·H/f_s` by the run's own first phase and a whole number
    /// of turns, i.e. by one constant rotation of the entire record, which
    /// changes neither `|Z|` nor the Candan ratio.
    pub(super) fn push(&mut self, i: usize, z: Complex<f32>) {
        if self.restart[i] {
            self.restart[i] = false;
            self.len[i] = 0;
            self.line_count[i] = 0;
            self.resolution_hz[i] = 0.0;
        }

        let len = self.len[i] as usize;
        if len == UNISON_RING_HOPS {
            self.ring[i].copy_within(1.., 0);
            self.ring[i][UNISON_RING_HOPS - 1] = z;
        } else {
            self.ring[i][len] = z;
            self.len[i] = len as u8 + 1;
        }

        let n = self.len[i] as usize;
        if n < UNISON_MIN_BINS {
            return;
        }
        let plan = n - UNISON_MIN_BINS;
        self.line_count[i] = peaks::resolve_lines(
            &self.ring[i][..n],
            self.plans[plan].as_ref(),
            self.c_n[plan],
            HOP_RATE_HZ,
            &mut LineScratch {
                spectrum: &mut self.spectrum,
                magnitudes: &mut self.magnitudes,
                fft: &mut self.fft_scratch,
            },
            &mut self.lines[i],
        ) as u8;
        // Two lines are resolved when they are `2/T` apart — the Hann main-lobe
        // half-width, and what the display must state alongside them.
        self.resolution_hz[i] = 2.0 * HOP_RATE_HZ / n as f32;
    }

    /// Runs the discriminator over everything the bank resolved this hop.
    ///
    /// Two strings at different f₀ put their partial *n* at frequencies whose
    /// ratio is the same for every *n*, so their separation is **proportional to
    /// the partial frequency** — constant in cents. One partial splitting against
    /// itself has no such reason. Both hypotheses are members of one family,
    ///
    /// ```text
    ///   ln Δ = ln a + p·ln f      p = 1 unison,  p = 0 fixed in Hz
    /// ```
    ///
    /// so the test is on the fitted exponent: a verdict is returned only when
    /// `p̂` is within [`UNISON_FIT_SIGMAS`] standard errors of one hypothesis and
    /// further than that from the other. Otherwise the data do not separate them
    /// and the answer is [`UnisonVerdict::Undetermined`] — which is common and
    /// correct, because a fit over three neighbouring partials has almost no
    /// lever arm in `ln f`.
    ///
    /// **The standard error is estimated from the residuals**, floored at the
    /// estimator's own precision, and that ordering is load-bearing. Do not test
    /// against the estimator's σ alone: the *physical* scatter of the split
    /// across partials — string-to-string differences in B, and coupling — runs
    /// several times that, and a null built from the instrument's precision calls
    /// 87 % of tenor unisons false beats (ADR 0012 §6). The floor stays because
    /// no fit can know a split better than it was measured.
    ///
    /// Only partials that resolved the **same number** of lines are compared.
    /// With three strings, a partial that resolved two of them is measuring a
    /// different pair from one that resolved all three, and the two are not the
    /// same quantity; mixing them would reject a genuine unison on nothing but
    /// availability.
    pub(super) fn discriminate(&mut self, refs: &[f32; MAX_STROBE_REFS], count: usize) {
        // Group by line count, then take the larger group. Ties go to the pairs:
        // a two-line split is the better-conditioned quantity of the two.
        let mut pairs = 0usize;
        let mut triples = 0usize;
        for (i, &f_ref) in refs.iter().enumerate().take(count.min(MAX_STROBE_REFS)) {
            if f_ref > 0.0 {
                match self.line_count[i] {
                    2 => pairs += 1,
                    3 => triples += 1,
                    _ => {}
                }
            }
        }
        let class = if triples > pairs { 3u8 } else { 2u8 };
        let used = if class == 3 { triples } else { pairs };
        if used < 2 {
            self.verdict = UnisonVerdict::Undetermined;
            return;
        }

        // The fit is over (ln f, ln Δ), where both hypotheses are slopes.
        let mut x = [0.0f32; MAX_STROBE_REFS];
        let mut y = [0.0f32; MAX_STROBE_REFS];
        // Relative precision of each split, from the estimator's own σ: a
        // measurement error of σ_Δ on Δ is σ_Δ/Δ in the log domain.
        let mut relative_sigma_sq = 0.0f32;
        let mut n_fit = 0usize;
        for (i, &f_ref) in refs.iter().enumerate().take(count.min(MAX_STROBE_REFS)) {
            if self.line_count[i] != class || f_ref <= 0.0 {
                continue;
            }
            let lines = &self.lines[i][..class as usize];
            let lo = lines
                .iter()
                .map(|l| l.offset_hz)
                .fold(f32::INFINITY, f32::min);
            let hi = lines
                .iter()
                .map(|l| l.offset_hz)
                .fold(f32::NEG_INFINITY, f32::max);
            let span_hz = hi - lo;
            if !span_hz.is_finite() || span_hz <= 0.0 {
                continue;
            }
            // Independent line estimates ⇒ the span's σ is √2 of a line's, and
            // the ring publishes 2/T, so its bin width is half that.
            let sigma_hz =
                std::f32::consts::SQRT_2 * UNISON_LINE_SIGMA_BINS * self.resolution_hz[i] / 2.0;
            relative_sigma_sq += (sigma_hz / span_hz).powi(2);
            x[n_fit] = f_ref.ln();
            y[n_fit] = span_hz.ln();
            n_fit += 1;
        }

        // Three partials, not two: the free slope costs two parameters, so two
        // points fit it exactly and leave no residual to judge it by.
        if n_fit < 3 {
            self.verdict = UnisonVerdict::Undetermined;
            return;
        }

        let n = n_fit as f32;
        let mean_x = x[..n_fit].iter().sum::<f32>() / n;
        let mean_y = y[..n_fit].iter().sum::<f32>() / n;
        let s_xx: f32 = x[..n_fit].iter().map(|x| (x - mean_x).powi(2)).sum();
        let s_xy: f32 = x[..n_fit]
            .iter()
            .zip(&y[..n_fit])
            .map(|(x, y)| (x - mean_x) * (y - mean_y))
            .sum();
        if s_xx <= 0.0 || !s_xx.is_finite() {
            self.verdict = UnisonVerdict::Undetermined;
            return;
        }
        let slope = s_xy / s_xx;
        let rss: f32 = x[..n_fit]
            .iter()
            .zip(&y[..n_fit])
            .map(|(x, y)| (y - mean_y - slope * (x - mean_x)).powi(2))
            .sum();
        // Residual variance, floored at the measurement's own: the data may be
        // scattered by more than the estimator's error but never by less.
        let variance = (rss / (n - 2.0)).max(relative_sigma_sq / n);
        let standard_error = (variance / s_xx).sqrt();

        self.verdict = if standard_error <= 0.0 || !standard_error.is_finite() || !slope.is_finite()
        {
            UnisonVerdict::Undetermined
        } else {
            let from_unison = (slope - 1.0).abs() / standard_error;
            let from_fixed = slope.abs() / standard_error;
            match (
                from_unison <= UNISON_FIT_SIGMAS,
                from_fixed <= UNISON_FIT_SIGMAS,
            ) {
                (true, false) => UnisonVerdict::Unison,
                (false, true) => UnisonVerdict::FalseBeat,
                // Consistent with both (no lever arm) or with neither (the
                // splits follow some third law): say so rather than guess.
                _ => UnisonVerdict::Undetermined,
            }
        };
    }

    /// The lines this reference resolved, strongest first.
    pub(super) fn lines(&self, i: usize) -> &[UnisonLine; MAX_UNISON_LINES] {
        &self.lines[i]
    }

    /// How many of [`Self::lines`] are valid.
    pub(super) fn line_count(&self, i: usize) -> u8 {
        self.line_count[i]
    }

    /// `2/T` for this reference's current record (Hz); `0.0` while it publishes
    /// nothing.
    pub(super) fn resolution_hz(&self, i: usize) -> f32 {
        self.resolution_hz[i]
    }

    /// The bank-wide verdict from the last [`Self::discriminate`].
    pub(super) fn verdict(&self) -> UnisonVerdict {
        self.verdict
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cap is a *duration* (ADR 0012 §4); the hop count must be the nearest
    /// one to it, and must clear the estimator's own floor.
    #[test]
    fn ring_cap_matches_its_duration() {
        assert_eq!(UNISON_RING_HOPS, 56);
        let secs = |hops: usize| hops as f32 / HOP_RATE_HZ;
        assert!((secs(UNISON_RING_HOPS) - UNISON_RING_SECS).abs() < 0.5 / HOP_RATE_HZ);
        const { assert!(UNISON_RING_HOPS >= UNISON_MIN_BINS) };
    }

    /// A perfect two-string unison — the split proportional to the partial
    /// frequency — must survive, and a split that is constant in *Hz* instead
    /// (the signature of one partial splitting against itself) must not.
    #[test]
    fn discriminator_separates_a_unison_from_a_false_beat() {
        let f1 = 220.0f32;
        let split_cents = 4.0f32;
        let mut refs = [0.0f32; MAX_STROBE_REFS];
        for (n, r) in refs.iter_mut().enumerate() {
            *r = f1 * (n + 1) as f32;
        }

        let place = |u: &mut Unison, spans: &[f32]| {
            for (i, &span) in spans.iter().enumerate() {
                u.lines[i][0] = UnisonLine {
                    offset_hz: 0.0,
                    relative_amplitude: 1.0,
                };
                u.lines[i][1] = UnisonLine {
                    offset_hz: span,
                    relative_amplitude: 0.9,
                };
                u.line_count[i] = 2;
                u.resolution_hz[i] = 2.0 * HOP_RATE_HZ / UNISON_RING_HOPS as f32;
            }
        };

        // Unison: the split scales with the partial, so the cents are constant.
        let mut u = Unison::new();
        let spans: Vec<f32> = (0..4)
            .map(|n| refs[n] * ((split_cents / 1200.0).exp2() - 1.0))
            .collect();
        place(&mut u, &spans);
        u.discriminate(&refs, 4);
        assert_eq!(u.verdict(), UnisonVerdict::Unison);

        // The same split in Hz at every partial: the cents then fall as 1/n,
        // which a pair of strings cannot do.
        let mut u = Unison::new();
        let flat = vec![spans[0]; 4];
        place(&mut u, &flat);
        u.discriminate(&refs, 4);
        assert_eq!(u.verdict(), UnisonVerdict::FalseBeat);

        // Measurement scatter must not flip a unison: 10 % of the split is far
        // more than the estimator's own σ and still leaves Δ ∝ f the better fit.
        let mut u = Unison::new();
        let jittered: Vec<f32> = spans
            .iter()
            .enumerate()
            .map(|(k, s)| s * (1.0 + 0.1 * if k % 2 == 0 { 1.0 } else { -1.0 }))
            .collect();
        place(&mut u, &jittered);
        u.discriminate(&refs, 4);
        assert_eq!(u.verdict(), UnisonVerdict::Unison);

        // Two partials are not enough to tell the families apart.
        let mut u = Unison::new();
        place(&mut u, &spans[..2]);
        u.discriminate(&refs, 2);
        assert_eq!(u.verdict(), UnisonVerdict::Undetermined);
    }

    /// A gated hop holds the published lines; the re-strike after it drops them
    /// and starts a fresh record, so the panel never shows a split measured
    /// across a gap.
    #[test]
    fn a_gate_holds_then_the_restart_drops() {
        let mut u = Unison::new();
        for h in 0..UNISON_MIN_BINS + 4 {
            u.push(
                0,
                Complex::new((h as f32 * 0.7).cos(), (h as f32 * 0.7).sin()),
            );
        }
        assert!(u.resolution_hz(0) > 0.0, "a filled ring must publish");
        let held = u.resolution_hz(0);

        u.hold(0);
        assert_eq!(u.resolution_hz(0), held, "a gated hop holds");

        u.push(0, Complex::new(1.0, 0.0));
        assert_eq!(u.line_count(0), 0, "the re-strike drops the old lines");
        assert_eq!(u.resolution_hz(0), 0.0);
    }

    /// Silence drops the same lines a gate holds, without waiting for the next
    /// strike to do it.
    #[test]
    fn silence_drops_what_a_gate_holds() {
        let mut u = Unison::new();
        for h in 0..UNISON_MIN_BINS + 4 {
            u.push(
                0,
                Complex::new((h as f32 * 0.7).cos(), (h as f32 * 0.7).sin()),
            );
        }
        assert!(u.line_count(0) > 0, "a filled ring must publish");

        u.clear(0);
        assert_eq!(u.line_count(0), 0);
        assert_eq!(u.resolution_hz(0), 0.0);

        // And the run is broken, as after a hold: the next strike starts fresh.
        u.push(0, Complex::new(1.0, 0.0));
        assert_eq!(u.line_count(0), 0);
    }
}
