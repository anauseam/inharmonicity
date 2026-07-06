//! # Median-Adjustive Trajectories (MAT)
//!
//! Estimates the fundamental frequency ($f_0$) and inharmonicity coefficient ($B$) of a
//! struck string from its magnitude spectrum, using the Median-Adjustive Trajectories
//! method.
//!
//! Implements the estimator of:
//!   Hodgkinson, M., Wang, J., Timoney, J. & Lazzarini, V. (2009). "Handling Inharmonic
//!   Series with Median-Adjustive Trajectories." Proc. DAFx-09, Como, Italy, pp. 1–7.
//!
//! ## Equations (DAFx-09)
//!
//! The stiff-string inharmonic series relates the $k$-th partial $f_k$ to $(f_0, B)$:
//!
//! ```text
//!   Eq. (1)   f_k = k · f0 · √(1 + B·k²)
//!   Eq. (6)   f0  = f_m / ( m · √(1 + B·m²) )                  [back-calculate f0]
//!   Eq. (8)   B   = ( (f_k·m/k)² − f_m² )                      [B from two partials m,k]
//!                   / ( k²·f_m² − m²·(f_k·m/k)² )
//!   Eq. (9)   E   = (K² − K) / 2                               [pairwise B-estimates for K partials]
//! ```
//!
//! Eq. (8) (Galembo's two-partial relation, cited by the paper) lets any pair of correctly
//! numbered partials yield a $B$ estimate; Eq. (6) then back-calculates an $f_0$. The
//! method's robustness is the **median** over the resulting B- and Fo-arrays (§2.2).
//!
//! ## Method (§2.2–§2.4)
//!
//! 1. **Predict** each partial position from the running $(f_0, B)$ via Eq. (1).
//! 2. **Locate** the strongest peak in a *narrow* band around the prediction (§2.4), sub-bin
//!    refined (§2.3), keeping it only if it rises above the magnitude-spectrum average — the
//!    paper's significance gate, which also terminates the series when partials fade (§2.2).
//! 3. **Re-estimate** $(f_0, B)$: build the B-array (Eq. 8) and Fo-array (Eq. 6) over the
//!    located partials (up to Eq. 9 of them) and take their **medians** — the resilience
//!    filter that nullifies anomalous readings (missing harmonics, parallel-string and
//!    longitudinal peaks, beating). The medians become the running $(f_0, B)$.
//! 4. **Repeat** until $(f_0, B)$ converges.
//!
//! The low partials are nearly independent of $B$ and anchor the estimate; the running
//! median $B$ then re-centres the high-partial bands onto the genuinely stretched peaks
//! (Eq. 1). A single non-adjustive pass would seed those high bands from a prior that is
//! ~an order of magnitude too low in the bass, mis-numbering partials and yielding
//! impossible (negative) $B$.
//!
//! ## Relationship to the paper, and deliberate adaptations
//!
//! The estimator is faithful — Eqs. 1/6/8/9, the median combiner, the §2.2 significance
//! gate, and §2.4 bands sized to the fundamental are all as published. Two points adapt to a
//! real out-of-tune upright (a harder regime than the paper's clean Steinway / bass-guitar
//! tones):
//!
//! * **Convergence order — selectable via [`MatOrder`].** The paper grows the trajectory
//!   *serially*: locate one partial, re-median, predict the next (Fig. 3). Both orders are
//!   implemented and share the same equations; the iteration *style* is the textbook
//!   Gauss-Seidel (serial) vs Jacobi (simultaneous) pair. They are **not** equivalent in
//!   outcome, though: because simultaneous is partial-count-capped and serial is not, they
//!   fit *different* partial sets and so reach different estimates — serial is the full
//!   method, simultaneous a limited variant:
//!     - [`MatOrder::Serial`] (**the shipped default**) grows one partial at a time, refining
//!       $(f_0, B)$ before each next prediction, so correct numbering is established
//!       incrementally and the series reaches many partials (toward [`MAX_PARTIALS`]),
//!       exploiting the high-$n$ $B$ leverage. On the real captures it tracks 30+ bass
//!       partials, agrees with `Simultaneous` in the clean mid register, and — by the
//!       goodness-of-fit check in `validate_mat` — its $(f_0, B)$ explains the clean low
//!       partials as well as `Simultaneous`'s (≈6.6 vs ≈6.6 kppm residual) while also fitting
//!       the high partials `Simultaneous` cannot (≈9.9 kppm). That refutes the concern that
//!       its high partials might follow one parallel string (the paper's Conclusion, §4)
//!       and bias $B$. Final accuracy
//!       still awaits a second, in-tune instrument.
//!     - [`MatOrder::Simultaneous`] (the conservative fallback) predicts **all** partials each
//!       pass and iterates. A single mis-associated partial cannot cascade, but it is capped
//!       at `SIM_MAX_PARTIALS` (12): predicting all partials from one shared estimate
//!       mis-numbers the high ones, whose $O(n^2)$ self-consistent wrong pairs then out-vote
//!       the median (24 collapses bass $B$ to ~0), so the high-$n$ information is left unused.
//! * **Seeding.** The paper seeds partials 1 and 2 at $f_{0,ET}$ and $2 f_{0,ET}$, presuming
//!   prominent low partials (§2.2). This project seeds from the Goertzel-tracked $f_0$ (more
//!   accurate on a detuned piano) and — because the deep-bass fundamental is often absent
//!   (ADR 0005: A0 carries no energy at partial 1, a case the paper does not treat) — the
//!   §2.2 gate lets the estimate anchor on whichever low partials actually clear the floor.
//!
//! Sub-bin refinement is the paper's preferred **CSPE** (§2.3): the per-bin super-resolution
//! frequency map ([`crate::algorithms::spectral::cspe`]) is computed once by the
//! Worker and the located partial's frequency is read straight from it, bin-independently.
//! Parallel-string courses are handled as in the paper — the narrow band (§2.4) keeps
//! the trajectory on a single series; full multi-series separation remains future work there
//! (Conclusion, §4) as here. The band is held a little wider than the paper's tightest
//! $f_0/16$ so one
//! pass can bootstrap $B$ from the low/mid partials, and the $B$ ceiling is generous enough
//! not to clip the steep treble inharmonicity rise the paper measures (Fig. 10).
//!
//! ## Measurement vs. assumption
//!
//! The estimator always reports the *measured* median $B$ over the located partials, with
//! a confidence reflecting pairwise agreement and the amount of supporting evidence. It
//! never substitutes the Rigaud prior. The only failure mode is `None` from
//! [`detect_pitch_mat`], returned when fewer than two partials clear the gate (no pair to
//! solve) — a capture failure to surface, not a value to fabricate.
//!
//! The `confidence` field on [`MatEstimate`] is **ours, not part of DAFx-09** (the paper
//! outputs only $(f_0, B)$; the median is its robustness mechanism). It measures pairwise
//! *self-consistency* × supporting evidence — **not accuracy**: a coherent-but-wrong series
//! (e.g. an octave-mis-seeded fit yielding 4×B) scores high. It is a diagnostic signal only
//! — never persisted, never gates a decision (demoted by decision, ADR 0006 Corrections
//! item 4). Used by the `validate_mat` / `mat_b_recovery` harnesses, not the live pipeline.
//!
//! ## References
//!
//! [1] Hodgkinson, M., Wang, J., Timoney, J. & Lazzarini, V. (2009). "Handling Inharmonic
//!     Series with Median-Adjustive Trajectories." Proc. DAFx-09, Como, Italy. (Eqs. 1, 6,
//!     8, 9; method §2.2; sub-bin refinement §2.3; narrow bands §2.4; multi-series
//!     limitation: Conclusion, §4 — the paper has no §7; a pre-audit reference said
//!     otherwise, see faithfulness-audit-07.)
//! [2] Galembo, A. S. & Askenfelt, A. (1999). "Signal Representation and Estimation of
//!     Spectral Parameters by Inharmonic Comb Filters…" IEEE Trans. Speech Audio Process.
//!     7(2), pp. 197–203. (Origin of Eq. 8's two-partial $B$ relation.)
//! [3] Short, K. M. & Garcia, R. A. (2006). "Signal Analysis Using the Complex Spectral
//!     Phase Evolution (CSPE) Method." AES 120th Convention, Paris. Paper 6645. (The sub-bin
//!     refinement, DAFx-09 §2.3; see [`crate::algorithms::spectral::cspe`].)

// ─── Tuning constants ───────────────────────────────────────────────────────────

/// Maximum predict→extract→re-estimate passes. The trajectory typically converges in 2–4
/// passes; the cap bounds worst-case cost on incoherent captures. This runs on the async
/// Worker (Gatekeeper State-4 RELEASE), not the audio hot path, so a few passes of ≤12
/// narrow sub-bin searches is negligible against the capture budget.
const MAX_ITERATIONS: u32 = 6;

/// Relative change in $f_0$ below which the trajectory is considered converged.
const F0_REL_TOL: f32 = 1e-4;

/// Relative change in $B$ below which the trajectory is considered converged.
const B_REL_TOL: f32 = 1e-2;

/// Lowest physically plausible $B$ (a little negative is allowed for sub-bin jitter).
const B_MIN: f32 = -1e-3;

/// Highest physically plausible $B$. The Rigaud prior alone reaches ~0.026 at C8 (DAFx-09
/// Fig. 10 shows the steep treble rise, ~2.8e-3 already by C#7), so the ceiling is generous
/// — its only job is to drop nonsensical pairs (e.g. from mis-numbering) before the median.
const B_MAX: f32 = 5e-2;

/// Partial-buffer capacity — the most partials any order can track. The paper grows the
/// series "as far as it features sufficient energy" (its examples reach ~22–27 partials),
/// and the [`MatOrder::Serial`] growth realises that: it predicts each high partial from an
/// already-converged estimate, so the prediction stays accurate and a fixed band keeps
/// associating correctly out to high $n$ (where $B$ leverage $\propto n^2$ is greatest).
/// Public so the Worker and harness size their partial buffers to match.
pub const MAX_PARTIALS: usize = 32;

/// Partial cap for the [`MatOrder::Simultaneous`] order. It predicts *all* partials from one
/// running $(f_0, B)$, so the predicted position of partial $n$ moves $\propto n^3 f_0$ per
/// unit of $B$ error; beyond $n\approx 12$ a realistic $B$ uncertainty shifts the prediction
/// past the fixed §2.4 band, the high partials mis-*number*, and their $O(n^2)$
/// self-consistent wrong pairs out-vote the correct ones — dragging the median to ~0
/// (empirically: 24 collapses bass $B$). So the simultaneous order is capped where it stays
/// reliable; only the serial order may exceed it.
const SIM_MAX_PARTIALS: usize = 12;

/// Stop the serial growth after this many consecutive sub-significant predictions (the series
/// has faded, §2.2's stopping rule) — generous enough to step over an isolated missing
/// partial without truncating the trajectory.
const SERIAL_MAX_CONSECUTIVE_MISSES: u32 = 3;

/// Maximum number of pairwise $B$ estimates, $E = (K^2 - K)/2$ for $K$ partials (Eq. 9).
const MAX_PAIRS: usize = MAX_PARTIALS * (MAX_PARTIALS - 1) / 2;

/// Peak-detection band half-width as a fraction of $f_0$ for the [`MatOrder::Simultaneous`]
/// order (constant across partials, §2.4). Sized to the fundamental, not to $f_n$, so it never
/// balloons into a neighbouring partial (spacing ≈ $f_0$). Wide because the simultaneous first
/// pass predicts every partial from $B = 0$ (harmonic positions), far from the stretched
/// peaks, so the band must reach them on the bootstrap pass.
const BAND_HALFWIDTH_F0_FRAC_SIM: f32 = 0.25;

/// Peak-detection band half-width as a fraction of $f_0$ for the [`MatOrder::Serial`] order.
///
/// The paper's §2.4 band is *tight* (~$f_0/16$ full) — its whole point is rejecting
/// parallel-string / longitudinal peaks — and serial's accurate incremental predictions make
/// a tight band feasible in principle. But it was **tested empirically** (`validate_mat`,
/// $f_0/16$ and $f_0/8$ half-widths) and is *fragile on this out-of-tune upright*: where the
/// deep-bass fundamental is missing or the seed is a little off, a tight band misses the true
/// partial and the trajectory locks onto a self-consistent *wrong* series — e.g. A#0 jumped
/// to 279× the prior. The tight band lowered the *self*-fit residual (it fits its own,
/// sometimes-wrong, partial set very cleanly) but *raised* the cross-residual against the
/// clean low-mid partials (6.6 → 17.6 kppm), the metric that actually tracks correctness.
/// So serial uses the same forgiving $f_0/4$ band as simultaneous here. **Revisit on a clean,
/// in-tune instrument**, where the paper's tight band should become both faithful and safe.
const BAND_HALFWIDTH_F0_FRAC_SERIAL: f32 = 0.25;

/// Floor on the band half-width in bins, so the window stays resolvable for sub-bin
/// interpolation even when $f_0/4$ is sub-bin (deep bass at large FFT sizes).
const BAND_HALFWIDTH_MIN_BINS: f32 = 4.0;

/// Pair count at which the confidence's evidence term saturates. Below it, confidence is
/// scaled down to reflect that the median rests on few pairwise estimates.
const CONFIDENCE_EVIDENCE_PAIRS: f32 = 10.0;

// ─── Public API ─────────────────────────────────────────────────────────────────

/// Joint $(f_0, B)$ estimate produced by the MAT trajectory.
///
/// The Worker is the heavy async stage and is expected to commit a measurement for every
/// captured key, so `b` is always the measured median (never the Rigaud prior). Its
/// reliability is carried by `confidence` rather than by withholding the value — a low
/// confidence in the information-limited treble is reported, not nulled.
#[derive(Debug, Clone, Copy)]
pub struct MatEstimate {
    /// Refined fundamental frequency (Hz).
    pub f0: f32,
    /// Measured inharmonicity coefficient: the median of the pairwise algebraic solutions
    /// over the located partials. **Never the Rigaud prior** — when fewer than two partials
    /// clear the gate, `detect_pitch_mat` returns `None` rather than fabricating it.
    pub b: f32,
    /// Reliability of `b` in `[0, 1]`: pairwise-median agreement scaled by how many
    /// pairwise estimates backed the median (so a single-pair reading cannot read as 1.0).
    pub confidence: f32,
    /// Number of partials located on the final pass.
    pub partial_count: usize,
    /// Passes taken to converge (diagnostic).
    pub iterations: u32,
}

/// Order in which the trajectory estimates its partials. Both use the same equations (§2.2),
/// but they are *not* interchangeable: serial is the paper's full method and reaches many
/// partials, while simultaneous is a partial-count-capped variant that lands on a different,
/// lower-information estimate (and cannot exceed the cap without collapsing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatOrder {
    /// Grow the trajectory **one partial at a time**, re-estimating $(f_0, B)$ before
    /// predicting the next — the paper's serial procedure (Fig. 3, Gauss-Seidel-like). Each
    /// high partial is predicted from an already-converged estimate, so correct numbering is
    /// established incrementally and the series extends toward [`MAX_PARTIALS`], realising the
    /// high-$n$ $B$ leverage the simultaneous order cannot. **The shipped default** (the
    /// faithful method): on the real captures its $(f_0, B)$ explains the clean low partials
    /// as well as `Simultaneous` while also fitting the high partials `Simultaneous` discards
    /// (goodness-of-fit check in `validate_mat`). Still being confirmed on a second, in-tune
    /// instrument; revert to `Simultaneous` if a regression appears.
    #[default]
    Serial,
    /// Predict **all** partials from one running $(f_0, B)$ and iterate to convergence
    /// (Jacobi-like). The conservative fallback: a single mis-associated partial cannot
    /// cascade into the next prediction, but it is capped at 12 partials, beyond which the
    /// high partials mis-number under one shared estimate (see `SIM_MAX_PARTIALS`) and the
    /// median collapses — so it leaves the high-$n$ $B$ information unused.
    ///
    /// Retained only as the A/B baseline / fallback until a second, in-tune instrument
    /// confirms `Serial` generalises; **remove this variant once it does** (it originated as a
    /// workaround for an earlier broken serial implementation, now superseded).
    Simultaneous,
}

/// Estimates the fundamental frequency and inharmonicity from a magnitude spectrum and its
/// CSPE-refined per-bin frequency map, using the MAT adjustive trajectory procedure.
///
/// # Arguments
/// * `magnitudes` — Linear magnitude spectrum (`magnitude_spectrum` output), used to
///   locate the strongest peak in each band.
/// * `cspe_freqs` — Per-bin super-resolution frequency (`cspe` output), parallel
///   to `magnitudes`; supplies each located partial's sub-bin-accurate frequency (§2.3).
/// * `sample_rate` — Audio sample rate in Hz.
/// * `f0_seed` — Coarse fundamental seed (the Goertzel-tracked $f_0$, or ET if untracked).
/// * `order` — Estimation order ([`MatOrder`]); `Serial` is the shipped default (the
///   Worker passes it), with `Simultaneous` kept as the labeled fallback.
/// * `partial_freqs_out` — Storage buffer; on success holds the located partial frequencies.
/// * `partial_ns_out` — Storage buffer; on success holds the matching partial indices.
///
/// # Returns
/// `Some(MatEstimate)` on success, `None` if fewer than two partials clear the gate.
pub fn detect_pitch_mat(
    magnitudes: &[f32],
    cspe_freqs: &[f32],
    sample_rate: u32,
    f0_seed: f32,
    order: MatOrder,
    partial_freqs_out: &mut [f32; MAX_PARTIALS],
    partial_ns_out: &mut [u32; MAX_PARTIALS],
) -> Option<MatEstimate> {
    if f0_seed <= 0.0 || !f0_seed.is_finite() || magnitudes.len() < 4 {
        return None;
    }

    // The paper's significance threshold: the average of the magnitude spectrum (§2.2). A
    // located peak must rise above this to count as a partial; this skips missing/weak
    // partials and rejects noise instead of admitting it.
    let ctx = SpectrumCtx {
        magnitudes,
        cspe_freqs,
        sample_rate,
        mag_threshold: mean_magnitude(magnitudes),
    };

    let outcome = match order {
        MatOrder::Simultaneous => {
            run_simultaneous(&ctx, f0_seed, partial_freqs_out, partial_ns_out)
        }
        MatOrder::Serial => run_serial(&ctx, f0_seed, partial_freqs_out, partial_ns_out),
    }?;

    // Confidence folds pairwise-median agreement together with how much evidence backed
    // the median, so an under-evidenced reading (few in-band partials, as in the treble)
    // reports low confidence instead of being withheld.
    let evidence = (outcome.solved.b_count as f32 / CONFIDENCE_EVIDENCE_PAIRS).min(1.0);
    let confidence = outcome.solved.coherence * evidence;

    // Physical floor: string stiffness only ever raises partials, so B ≥ 0. A negative
    // median is measurement noise on an information-starved key (too few low-n partials to
    // constrain B); it is reported as ~0 with its low confidence, never sign-flipped.
    let b = outcome.solved.b.max(0.0);

    Some(MatEstimate {
        f0: outcome.solved.f0,
        b,
        confidence,
        partial_count: outcome.partial_count,
        iterations: outcome.iterations,
    })
}

// ─── Internals ──────────────────────────────────────────────────────────────────

/// Read-only spectrum context shared across a trajectory's passes.
struct SpectrumCtx<'a> {
    magnitudes: &'a [f32],
    /// Per-bin CSPE super-resolution frequency, parallel to `magnitudes` (§2.3).
    cspe_freqs: &'a [f32],
    sample_rate: u32,
    /// Significance threshold (the magnitude-spectrum average, §2.2).
    mag_threshold: f32,
}

/// Median $(f_0, B)$ solved over a set of located partials (Eqs. 6/8/9), plus diagnostics.
struct Solved {
    /// Median back-calculated fundamental.
    f0: f32,
    /// Median pairwise inharmonicity.
    b: f32,
    /// Count of in-range pairwise estimates the medians were taken over.
    b_count: usize,
    /// Fraction of pairwise $B$ estimates agreeing with the median.
    coherence: f32,
}

/// What an estimation order returns: the solved estimate, the partial count, and an
/// order-specific iteration count (passes for Simultaneous, partials grown for Serial).
struct Outcome {
    solved: Solved,
    partial_count: usize,
    iterations: u32,
}

/// [`MatOrder::Simultaneous`]: predict all (up to `SIM_MAX_PARTIALS`) partials from the
/// running $(f_0, B)$, solve the medians, and iterate to convergence (Jacobi-like).
fn run_simultaneous(
    ctx: &SpectrumCtx,
    f0_seed: f32,
    freqs: &mut [f32; MAX_PARTIALS],
    ns: &mut [u32; MAX_PARTIALS],
) -> Option<Outcome> {
    // Adjustive trajectory state, seeded harmonically (B = 0). Each pass re-seeds the
    // partial windows from the refined (f0, B).
    let mut f0 = f0_seed;
    let mut b = 0.0_f32;
    let mut best: Option<Solved> = None;
    let mut iterations = 0;

    for pass in 0..MAX_ITERATIONS {
        iterations = pass + 1;

        let count = extract_all(ctx, f0, b, SIM_MAX_PARTIALS, freqs, ns);
        let Some(solved) = solve_estimate(freqs, ns, count) else {
            break; // too few partials to solve; keep the previous best
        };

        let converged = ((solved.f0 - f0).abs() / f0 < F0_REL_TOL)
            && ((solved.b - b).abs() / (b.abs() + 1e-6) < B_REL_TOL);
        f0 = solved.f0;
        b = solved.b;
        best = Some(solved);
        if converged {
            break;
        }
    }

    let solved = best?;

    // Re-extract at the converged estimate so the caller's buffers and the reported
    // `partial_count` describe the SAME trajectory we return (the last pass extracted at the
    // *previous* prediction before refining the median).
    let partial_count = extract_all(ctx, solved.f0, solved.b, SIM_MAX_PARTIALS, freqs, ns);

    Some(Outcome {
        solved,
        partial_count,
        iterations,
    })
}

/// [`MatOrder::Serial`]: the paper's Fig. 3 growth. Step outward partial by partial; each is
/// predicted from the running median $(f_0, B)$ (Eq. 1), located in a band, and — if it
/// clears the §2.2 significance gate — added before re-solving the median for the next step.
/// Missing/weak partials are skipped; the series stops after a run of consecutive misses (it
/// has faded) or at Nyquist / [`MAX_PARTIALS`].
fn run_serial(
    ctx: &SpectrumCtx,
    f0_seed: f32,
    freqs: &mut [f32; MAX_PARTIALS],
    ns: &mut [u32; MAX_PARTIALS],
) -> Option<Outcome> {
    let nyquist = ctx.sample_rate as f32 / 2.0;
    let mut f0_hat = f0_seed;
    let mut b_hat = 0.0_f32;
    let mut k = 0_usize;
    let mut misses = 0_u32;

    for n in 1..=MAX_PARTIALS as u32 {
        let predicted = predicted_position(f0_hat, b_hat, n);
        if predicted >= nyquist {
            break;
        }

        if let Some((frequency, _mag)) =
            extract_significant(ctx, predicted, f0_hat, BAND_HALFWIDTH_F0_FRAC_SERIAL)
        {
            freqs[k] = frequency;
            ns[k] = n;
            k += 1;
            misses = 0;
            // Adjustive step: refine the running estimate before predicting the next partial.
            if let Some(solved) = solve_estimate(freqs, ns, k) {
                f0_hat = solved.f0;
                b_hat = solved.b;
            }
        } else {
            misses += 1;
            // Stop once the series has clearly faded (but only after it is anchored, so a
            // missing bass fundamental does not end the trajectory before it starts).
            if k >= 2 && misses >= SERIAL_MAX_CONSECUTIVE_MISSES {
                break;
            }
        }
    }

    let solved = solve_estimate(freqs, ns, k)?;
    Some(Outcome {
        solved,
        partial_count: k,
        iterations: k as u32,
    })
}

/// Eq. (1): predicted inharmonic position of partial `n`, $f_n = n f_0 \sqrt{1 + B n^2}$.
/// `max(0.0)` guards a transiently negative B from an early estimate producing a NaN.
fn predicted_position(f0: f32, b: f32, n: u32) -> f32 {
    let n_f = n as f32;
    n_f * f0 * (1.0 + b * n_f * n_f).max(0.0).sqrt()
}

/// Locates the strongest peak in the §2.4 band (half-width `band_frac · f0`) around
/// `center_hz`, returning its `(CSPE frequency, magnitude)` only if it clears the §2.2
/// significance gate. `band_frac` is the per-order band (currently both $f_0/4$ — the
/// paper's tight serial band was tested and reverted on this instrument; see
/// `BAND_HALFWIDTH_F0_FRAC_SERIAL`).
fn extract_significant(
    ctx: &SpectrumCtx,
    center_hz: f32,
    f0: f32,
    band_frac: f32,
) -> Option<(f32, f32)> {
    let hz_per_bin = ctx.sample_rate as f32 / (ctx.magnitudes.len() as f32 * 2.0);
    // Constant band half-width, sized to the fundamental so it never spans a neighbour.
    let half_width_hz = (f0 * band_frac).max(BAND_HALFWIDTH_MIN_BINS * hz_per_bin);
    let (frequency, magnitude) = extract_peak_in_band(ctx, center_hz, half_width_hz)?;
    (magnitude > ctx.mag_threshold).then_some((frequency, magnitude))
}

/// Predicts partials `1..=max_n` from a fixed `(f0, b)` and writes those clearing the gate
/// into `freqs` / `ns`, returning the count located (the simultaneous order's extractor).
fn extract_all(
    ctx: &SpectrumCtx,
    f0: f32,
    b: f32,
    max_n: usize,
    freqs: &mut [f32; MAX_PARTIALS],
    ns: &mut [u32; MAX_PARTIALS],
) -> usize {
    let nyquist = ctx.sample_rate as f32 / 2.0;
    let mut count = 0;

    for n in 1..=max_n as u32 {
        let predicted = predicted_position(f0, b, n);
        if predicted >= nyquist {
            break;
        }
        if let Some((frequency, _mag)) =
            extract_significant(ctx, predicted, f0, BAND_HALFWIDTH_F0_FRAC_SIM)
        {
            freqs[count] = frequency;
            ns[count] = n;
            count += 1;
        }
    }

    count
}

/// Solves the median $(f_0, B)$ over the `k` located partials: the B-array via Eq. 8 (up to
/// `MAX_PAIRS` pairwise entries, Eq. 9) and the Fo-array via Eq. 6 (one entry per partial,
/// the paper's K-entry construction). Returns `None` if no valid pair can be formed.
fn solve_estimate(
    freqs: &[f32; MAX_PARTIALS],
    ns: &[u32; MAX_PARTIALS],
    k: usize,
) -> Option<Solved> {
    if k < 2 {
        return None;
    }

    // B-array (Eq. 8): one inharmonicity estimate per index pair. Its median is the §2.2
    // resilience filter for B.
    let mut b_estimates = [0.0_f32; MAX_PAIRS];
    let mut count = 0_usize;
    for i in 0..k {
        for j in (i + 1)..k {
            if let Some((b_v, _f0_v)) = compute_pair(freqs[i], ns[i], freqs[j], ns[j])
                && count < b_estimates.len()
            {
                b_estimates[count] = b_v;
                count += 1;
            }
        }
    }
    if count == 0 {
        return None;
    }

    let median_b = median_f32(&mut b_estimates[..count]);
    if !median_b.is_finite() {
        return None;
    }

    // Fo-array (Eq. 6): exactly ONE f0 per located partial, back-calculated with the median
    // B (the paper's K-entry construction, page 3 — not one entry per pair).
    let mut f0_estimates = [0.0_f32; MAX_PARTIALS];
    for i in 0..k {
        let n_f = ns[i] as f32;
        let root = (1.0 + median_b * n_f * n_f).max(1e-6).sqrt();
        f0_estimates[i] = freqs[i] / (n_f * root);
    }
    let median_f0 = median_f32(&mut f0_estimates[..k]);
    if !median_f0.is_finite() || median_f0 <= 0.0 {
        return None;
    }

    // Coherence: fraction of pairwise $B$ within a band of the median. The absolute floor
    // keeps the band meaningful when the median is near zero (harmonic-ish).
    let tol = 0.5 * median_b.abs() + 5e-4;
    let agree = b_estimates[..count]
        .iter()
        .filter(|&&x| (x - median_b).abs() <= tol)
        .count();

    Some(Solved {
        f0: median_f0,
        b: median_b,
        b_count: count,
        coherence: agree as f32 / count as f32,
    })
}

/// Finds the strongest magnitude peak within `±half_width_hz` of `center_hz` and returns
/// its `(CSPE frequency, peak_magnitude)`. The frequency is read from the CSPE map at the
/// peak bin (§2.3) — super-resolved and bin-independent; the magnitude is the raw bin-peak
/// value, left for the caller to compare against the significance threshold.
fn extract_peak_in_band(
    ctx: &SpectrumCtx,
    center_hz: f32,
    half_width_hz: f32,
) -> Option<(f32, f32)> {
    let magnitudes = ctx.magnitudes;
    let buffer_size = magnitudes.len() as f32 * 2.0;
    let bins_per_hz = buffer_size / ctx.sample_rate as f32;

    let center_bin = center_hz * bins_per_hz;
    let half_width_bins = half_width_hz * bins_per_hz;

    let lo = (center_bin - half_width_bins).floor().max(1.0) as usize;
    let hi = ((center_bin + half_width_bins).ceil() as usize).min(magnitudes.len() - 1);
    if lo >= hi {
        return None;
    }

    let (rel, &peak_mag) = magnitudes[lo..=hi]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))?;
    let peak_bin = lo + rel;

    if peak_mag <= 0.0 || !peak_mag.is_finite() {
        return None;
    }

    // CSPE super-resolution frequency at the dominant bin (§2.3). Falls back to the bin
    // centre if the map is shorter than the magnitude spectrum (defensive).
    let frequency = ctx
        .cspe_freqs
        .get(peak_bin)
        .copied()
        .unwrap_or(peak_bin as f32 / bins_per_hz);

    if frequency > 0.0 && frequency.is_finite() {
        Some((frequency, peak_mag))
    } else {
        None
    }
}

/// Mean of the magnitude spectrum, excluding the DC bin — the DAFx-09 §2.2 significance
/// threshold against which each newly detected peak is compared.
fn mean_magnitude(magnitudes: &[f32]) -> f32 {
    if magnitudes.len() <= 1 {
        return 0.0;
    }
    let sum: f32 = magnitudes[1..].iter().sum();
    sum / (magnitudes.len() - 1) as f32
}

fn median_f32(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    values[values.len() / 2]
}

/// Pairwise $B$ (Eq. 8) and $f_0$ (Eq. 6) from two partials $(f_m, m)$ and $(f_n, n)$.
///
/// Writing $K_m = (f_m/m)^2$ and $K_n = (f_n/n)^2$, Eq. (8) reduces algebraically to
/// $B = (K_n - K_m) / (K_m n^2 - K_n m^2)$ (it cancels a common $m^2$, see the module
/// equation block), and Eq. (6) back-calculates $f_0 = f_m / (m\sqrt{1 + B m^2})$.
fn compute_pair(f_m: f32, n_m: u32, f_n: f32, n_n: u32) -> Option<(f32, f32)> {
    if n_m == n_n || n_m == 0 || n_n == 0 {
        return None;
    }
    // Eq. (8): B = (K_n − K_m) / (K_m·n² − K_n·m²),  K = (f/index)².
    let k_m = (f_m / n_m as f32).powi(2);
    let k_n = (f_n / n_n as f32).powi(2);
    let denom = k_m * (n_n as f32).powi(2) - k_n * (n_m as f32).powi(2);

    if denom.abs() < 1e-8 {
        return None;
    }

    let b = (k_n - k_m) / denom;

    // Drop physically impossible pairs (e.g. from mis-numbering) before the median. The
    // ceiling is deliberately generous so genuine treble inharmonicity is not filtered out.
    if b <= B_MIN || b >= B_MAX {
        return None;
    }

    // Eq. (6): f0 = f_m / (m·√(1 + B·m²)).
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algorithms::spectral::{cspe, fft, magnitude_spectrum};
    use realfft::RealFftPlanner;
    use rustfft::num_complex::Complex;
    use std::f32::consts::PI;

    const SAMPLE_RATE: u32 = 44100;
    const FFT_SIZE: usize = 16384;

    /// Synthesises a time-domain inharmonic tone (partials `lo..=hi` at f_n = n·f0·√(1+B·n²),
    /// amplitude 1/√n) and runs the real Worker spectral pipeline: two Hann-windowed FFTs
    /// (the frame and the same frame advanced one sample) and CSPE. Returns the magnitude
    /// spectrum and the CSPE per-bin frequency map that `detect_pitch_mat` consumes — so the
    /// tests exercise CSPE end-to-end, not a mocked sub-bin estimate.
    fn synth_inharmonic(f0: f32, b: f32, lo: u32, hi: u32) -> (Vec<f32>, Vec<f32>) {
        // One extra sample so the one-sample-shifted frame is fully populated.
        let mut signal = vec![0.0_f32; FFT_SIZE + 1];
        for (i, sample) in signal.iter_mut().enumerate() {
            let t = i as f32 / SAMPLE_RATE as f32;
            let mut acc = 0.0_f32;
            for n in lo..=hi {
                let n_f = n as f32;
                let f_n = n_f * f0 * (1.0 + b * n_f * n_f).sqrt();
                acc += (2.0 * PI * f_n * t).sin() / n_f.sqrt();
            }
            *sample = acc;
        }

        let mut planner = RealFftPlanner::<f32>::new();
        let r2c = planner.plan_fft_forward(FFT_SIZE);
        let mut time = vec![0.0_f32; FFT_SIZE];
        let mut x0 = vec![Complex { re: 0.0, im: 0.0 }; FFT_SIZE / 2 + 1];
        let mut x1 = vec![Complex { re: 0.0, im: 0.0 }; FFT_SIZE / 2 + 1];
        let mut mags = vec![0.0_f32; FFT_SIZE / 2];
        let mut cspe_map = vec![0.0_f32; FFT_SIZE / 2];

        fft(&signal[..FFT_SIZE], &mut time, &mut x0, &r2c, FFT_SIZE);
        fft(&signal[1..FFT_SIZE + 1], &mut time, &mut x1, &r2c, FFT_SIZE);
        magnitude_spectrum(&x0, FFT_SIZE, &mut mags);
        cspe(&x0, &x1, FFT_SIZE, SAMPLE_RATE, &mut cspe_map);

        (mags, cspe_map)
    }

    #[test]
    fn recovers_known_inharmonicity() {
        let f0 = 110.0;
        let b = 5.0e-4;
        let (mags, cspe) = synth_inharmonic(f0, b, 1, 12);

        let mut freqs = [0.0f32; MAX_PARTIALS];
        let mut ns = [0u32; MAX_PARTIALS];
        // Seed deliberately offset (+3%); the iteration must converge anyway.
        let est = detect_pitch_mat(
            &mags,
            &cspe,
            SAMPLE_RATE,
            f0 * 1.03,
            MatOrder::Simultaneous,
            &mut freqs,
            &mut ns,
        )
        .expect("should produce an estimate");

        assert!(
            (est.b - b).abs() / b < 0.20,
            "measured B {} not within 20% of true B {b}",
            est.b
        );
        assert!(
            (est.f0 - f0).abs() / f0 < 0.01,
            "f0 {} not within 1% of true {f0}",
            est.f0
        );
        assert!(est.confidence > 0.5);
    }

    #[test]
    fn recovers_high_treble_inharmonicity() {
        // A high-B treble series must NOT be filtered out by the pairwise B ceiling — the
        // bug the old B_MAX = 0.01 introduced (it rejected every genuine high-B pair).
        let f0 = 1760.0; // A6
        let b = 8.0e-3;
        let (mags, cspe) = synth_inharmonic(f0, b, 1, 8);

        let mut freqs = [0.0f32; MAX_PARTIALS];
        let mut ns = [0u32; MAX_PARTIALS];
        let est = detect_pitch_mat(
            &mags,
            &cspe,
            SAMPLE_RATE,
            f0,
            MatOrder::Simultaneous,
            &mut freqs,
            &mut ns,
        )
        .expect("should produce an estimate");
        assert!(
            (est.b - b).abs() / b < 0.30,
            "measured treble B {} not within 30% of true {b}",
            est.b
        );
    }

    #[test]
    fn empty_spectrum_reports_no_measurement() {
        // Silence must never be laundered into a prior-valued measurement.
        let mags = vec![0.0_f32; FFT_SIZE / 2];
        let cspe = vec![0.0_f32; FFT_SIZE / 2];
        let mut freqs = [0.0f32; MAX_PARTIALS];
        let mut ns = [0u32; MAX_PARTIALS];
        let est = detect_pitch_mat(
            &mags,
            &cspe,
            SAMPLE_RATE,
            220.0,
            MatOrder::Simultaneous,
            &mut freqs,
            &mut ns,
        );
        assert!(est.is_none());
    }

    #[test]
    fn skips_missing_fundamental() {
        // Deep-bass regime: partials 1–3 absent. The trajectory must still measure B from
        // the surviving high partials rather than seeding on the missing fundamental.
        let f0 = 30.0;
        let b = 7.0e-4;
        let (mags, cspe) = synth_inharmonic(f0, b, 4, 12);

        let mut freqs = [0.0f32; MAX_PARTIALS];
        let mut ns = [0u32; MAX_PARTIALS];
        let est = detect_pitch_mat(
            &mags,
            &cspe,
            SAMPLE_RATE,
            f0,
            MatOrder::Simultaneous,
            &mut freqs,
            &mut ns,
        )
        .expect("should measure from high partials alone");
        assert!(
            est.b > 0.0,
            "missing-fundamental B should still be positive"
        );
        assert!(
            (est.b - b).abs() / b < 0.40,
            "missing-fundamental B {} not within 40% of true {b}",
            est.b
        );
    }

    #[test]
    fn harmonic_series_measures_near_zero_b() {
        // A genuinely harmonic series should measure B ≈ 0, not be rejected.
        let f0 = 196.0;
        let (mags, cspe) = synth_inharmonic(f0, 0.0, 1, 12);
        let mut freqs = [0.0f32; MAX_PARTIALS];
        let mut ns = [0u32; MAX_PARTIALS];
        let est = detect_pitch_mat(
            &mags,
            &cspe,
            SAMPLE_RATE,
            f0,
            MatOrder::Simultaneous,
            &mut freqs,
            &mut ns,
        )
        .expect("should produce an estimate");
        assert!(
            est.b.abs() < 1.0e-4,
            "harmonic series measured B {} too large",
            est.b
        );
        assert!((est.f0 - f0).abs() / f0 < 0.01);
    }

    #[test]
    fn serial_recovers_known_inharmonicity() {
        // The serial order must reach the same fixed point as the simultaneous order.
        let f0 = 110.0;
        let b = 5.0e-4;
        let (mags, cspe) = synth_inharmonic(f0, b, 1, 12);

        let mut freqs = [0.0f32; MAX_PARTIALS];
        let mut ns = [0u32; MAX_PARTIALS];
        let est = detect_pitch_mat(
            &mags,
            &cspe,
            SAMPLE_RATE,
            f0 * 1.03,
            MatOrder::Serial,
            &mut freqs,
            &mut ns,
        )
        .expect("serial should produce an estimate");
        assert!(
            (est.b - b).abs() / b < 0.20,
            "serial measured B {} not within 20% of true B {b}",
            est.b
        );
        assert!((est.f0 - f0).abs() / f0 < 0.01);
    }

    #[test]
    fn serial_uses_high_partials() {
        // A long bass-like series (20 partials). Serial growth must walk past partial 12 —
        // which the simultaneous order caps at — and recover B from the high partials, where
        // the leverage is greatest. With this many partials the estimate should be tight.
        let f0 = 60.0;
        let b = 6.0e-4;
        let (mags, cspe) = synth_inharmonic(f0, b, 1, 20);

        let mut freqs = [0.0f32; MAX_PARTIALS];
        let mut ns = [0u32; MAX_PARTIALS];
        let est = detect_pitch_mat(
            &mags,
            &cspe,
            SAMPLE_RATE,
            f0,
            MatOrder::Serial,
            &mut freqs,
            &mut ns,
        )
        .expect("serial should produce an estimate");
        assert!(
            est.partial_count > SIM_MAX_PARTIALS,
            "serial should reach beyond the simultaneous cap (got {} partials)",
            est.partial_count
        );
        assert!(
            (est.b - b).abs() / b < 0.20,
            "serial high-partial B {} not within 20% of true B {b}",
            est.b
        );
    }
}
