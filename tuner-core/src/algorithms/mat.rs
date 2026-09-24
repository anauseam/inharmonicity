//! # Median-Adjustive Trajectories (MAT)
//!
//! Estimates a struck string's fundamental ($f_0$) and inharmonicity coefficient ($B$)
//! from its magnitude spectrum by the Median-Adjustive Trajectories method \[1\].
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
//! Eq. (8), Galembo's two-partial relation \[2\], gives a $B$ estimate from any pair of
//! correctly numbered partials, and Eq. (6) back-calculates $f_0$; the method's
//! robustness is the median over both arrays (§2.2).
//!
//! ## Method (§2.2–§2.4)
//!
//! 1. **Predict** each partial position from the running $(f_0, B)$ via Eq. (1).
//! 2. **Locate** the strongest peak in a narrow band around the prediction (§2.4), sub-bin
//!    refined (§2.3), keeping it only above the magnitude-spectrum average, the §2.2
//!    significance gate that also ends the series as partials fade.
//! 3. **Re-estimate** $(f_0, B)$ as the medians of the B-array (Eq. 8) and the Fo-array
//!    (Eq. 6) over the located partials, which discounts missing harmonics, parallel-string
//!    and longitudinal peaks, and beating.
//! 4. **Repeat** until $(f_0, B)$ converges.
//!
//! The low partials barely depend on $B$ and anchor the estimate; the running median $B$
//! then re-centres the high-partial bands on the stretched peaks. A single pass from the
//! prior, an order of magnitude low in the bass, mis-numbers partials and yields negative
//! $B$.
//!
//! ## Adaptations
//!
//! The equations, the median combiner, the §2.2 gate and the §2.4 bands sized to the
//! fundamental are as published. For an out-of-tune piano:
//!
//! * **Order.** [`MatOrder::Serial`] is the paper's growth (Fig. 3);
//!   [`MatOrder::Simultaneous`] is a capped variant kept as the fallback. They fit
//!   different partial sets and reach different estimates.
//! * **Seeding.** From the tracked $f_0$ rather than $f_{0,ET}$. The deep-bass fundamental
//!   is often absent, a case the paper does not treat, so the §2.2 gate lets the estimate
//!   anchor on whichever low partials clear it.
//! * **Bands.** $f_0/4$, four times the paper's tightest $f_0/16$: on a detuned piano
//!   a tight band misses the true partial and locks onto a self-consistent wrong series.
//!
//! Sub-bin frequencies are read from a CSPE map (§2.3, \[3\]). Parallel-string courses are
//! handled as in the paper, by the narrow band; full multi-series separation is future
//! work there (§4) as here.
//!
//! ## Output
//!
//! The estimate is always the measured median $B$, never the Rigaud prior; `None` when
//! fewer than two partials clear the gate. `confidence` is ours: pairwise
//! self-consistency times supporting evidence, not accuracy, since a coherent but wrong
//! series (an octave mis-seed giving 4×B) scores high. Nothing gates on it.
//!
//! ## References
//!
//! \[1\] Hodgkinson, M., Wang, J., Timoney, J. & Lazzarini, V. (2009). "Handling Inharmonic
//!     Series with Median-Adjustive Trajectories." Proc. DAFx-09, Como, Italy. (Eqs. 1, 6,
//!     8, 9; method §2.2; sub-bin refinement §2.3; narrow bands §2.4; multi-series
//!     limitation: Conclusion, §4.)
//! \[2\] Galembo, A. S. & Askenfelt, A. (1999). "Signal Representation and Estimation of
//!     Spectral Parameters by Inharmonic Comb Filters…" IEEE Trans. Speech Audio Process.
//!     7(2), pp. 197–203. (Origin of Eq. 8's two-partial $B$ relation.)
//! \[3\] Short, K. M. & Garcia, R. A. (2006). "Signal Analysis Using the Complex Spectral
//!     Phase Evolution (CSPE) Method." AES 120th Convention, Paris. Paper 6645. (The sub-bin
//!     refinement, DAFx-09 §2.3; see [`crate::algorithms::spectral::cspe`].)

// ─── Tuning constants ───────────────────────────────────────────────────────────

/// Maximum predict–extract–re-estimate passes. A trajectory typically converges in 2–4;
/// the cap bounds the cost of an incoherent capture.
const MAX_ITERATIONS: u32 = 6;

/// Relative change in $f_0$ below which the trajectory is considered converged.
const F0_REL_TOL: f32 = 1e-4;

/// Relative change in $B$ below which the trajectory is considered converged.
const B_REL_TOL: f32 = 1e-2;

/// Lowest physically plausible $B$ (a little negative is allowed for sub-bin jitter).
const B_MIN: f32 = -1e-3;

/// Highest physically plausible $B$: generous, since the Rigaud prior reaches ~0.026 at C8
/// (DAFx-09 Fig. 10's treble rise), and there only to drop nonsensical pairs before the
/// median.
const B_MAX: f32 = 5e-2;

/// Partial-buffer capacity: the most partials any order can track. The paper grows the
/// series "as far as it features sufficient energy" (its examples reach ~22–27), which
/// [`MatOrder::Serial`] realises out to high $n$, where $B$ leverage ($\propto n^2$) is
/// greatest.
pub const MAX_PARTIALS: usize = 32;

/// Partial cap for the [`MatOrder::Simultaneous`] order. Predicting every partial from one
/// $(f_0, B)$ moves partial $n$ by $\propto n^3 f_0$ per unit of $B$ error, so past
/// $n \approx 12$ the predictions leave the §2.4 band, the high partials mis-number, and
/// their $O(n^2)$ self-consistent wrong pairs drag the median to ~0.
// Do not raise: a cap of 24 collapses bass B.
// audit 07
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

/// Peak-detection band half-width as a fraction of $f_0$ for the [`MatOrder::Serial`] order:
/// the same forgiving $f_0/4$ as the simultaneous order, not the paper's tight §2.4 band. On
/// an out-of-tune piano, with the fundamental missing or the seed a little off, a tight band
/// misses the true partial and locks onto a self-consistent wrong series.
// Do not tighten on detuned data. At f0/16 and f0/8 half-widths (`cargo lab mat validate`)
// A#0 jumped to 279× the prior, and the cross-residual against the clean low-mid partials
// rose 6.6 → 17.6 kppm while the self-fit residual fell.
// audit 07
const BAND_HALFWIDTH_F0_FRAC_SERIAL: f32 = 0.25;

/// Floor on the band half-width, in bins, so the band stays resolvable where $f_0/4$ is
/// under a few bins: in the deep bass on a capture short enough to shrink the FFT (at 2¹⁴,
/// 4 bins is 10.8 Hz against A0's 6.9 Hz $f_0/4$). Inert on a full-length capture.
// Ours: the paper states no floor, though §2.4's worked example band is 4 bins. Not
// `peaks::COARSE_SPAN_MIN_BINS` despite the shared value: that one is safe only because
// of a neighbour cap this band lacks. Do not factor them together.
// audit 07
const BAND_HALFWIDTH_MIN_BINS: f32 = 4.0;

/// Pair count at which the confidence's evidence term saturates. Below it, confidence is
/// scaled down to reflect that the median rests on few pairwise estimates.
const CONFIDENCE_EVIDENCE_PAIRS: f32 = 10.0;

// ─── Public API ─────────────────────────────────────────────────────────────────

/// The joint $(f_0, B)$ estimate of a MAT trajectory. A weak reading is reported with low
/// `confidence`, not withheld.
#[derive(Debug, Clone, Copy)]
pub struct MatEstimate {
    /// Refined fundamental frequency (Hz).
    pub f0: f32,
    /// Measured inharmonicity coefficient: the median of the pairwise solutions over the
    /// located partials.
    pub b: f32,
    /// Self-consistency of `b` in `[0, 1]`, not its accuracy: the fraction of pairwise
    /// estimates agreeing with the median, scaled down when few backed it, so a
    /// single-pair reading cannot read as 1.0.
    pub confidence: f32,
    /// Number of partials located on the final pass.
    pub partial_count: usize,
    /// Passes taken to converge (diagnostic).
    pub iterations: u32,
}

/// Order in which the trajectory estimates its partials. Both use the same equations (§2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatOrder {
    /// Grow the trajectory one partial at a time, re-estimating $(f_0, B)$ before predicting
    /// the next: the paper's serial procedure (Fig. 3, Gauss–Seidel-like). Each high partial
    /// is predicted from a converged estimate, so the series extends toward
    /// [`MAX_PARTIALS`].
    // Measured with `cargo lab mat validate` on detuned captures: serial tracks 30+ bass
    // partials, agrees with simultaneous in the clean mid register, and fits the clean low
    // partials as well (≈6.6 kppm residual each) while also fitting the high partials
    // simultaneous drops (≈9.9 kppm).
    #[default]
    Serial,
    /// Predict every partial from one running $(f_0, B)$ and iterate (Jacobi-like). A
    /// mis-associated partial cannot cascade, but it stops at 12 partials, beyond which the
    /// median collapses, so the high-$n$ information goes unused. The fallback.
    Simultaneous,
}

/// Estimates the fundamental frequency and inharmonicity from a magnitude spectrum and its
/// CSPE-refined per-bin frequency map, using the MAT adjustive trajectory procedure.
///
/// `magnitudes` is the linear spectrum (`magnitude_spectrum` output) that locates the strongest
/// peak in each band, and `cspe_freqs` the parallel per-bin frequency map (`cspe` output) that
/// gives each located partial its sub-bin frequency (§2.3). `f0_seed` is the coarse fundamental
/// in Hz — the Goertzel-tracked $f_0$, or ET if untracked. The located partials' frequencies and
/// indices are written to `partial_freqs_out` and `partial_ns_out`, and the result is `None` if
/// fewer than two partials clear the gate.
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

    let evidence = (outcome.solved.b_count as f32 / CONFIDENCE_EVIDENCE_PAIRS).min(1.0);
    let confidence = outcome.solved.coherence * evidence;

    // Stiffness only raises partials, so B ≥ 0: a negative median is noise on a key with
    // too few low partials, reported as 0 with its low confidence, never sign-flipped.
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
    // `partial_count` describe the same trajectory we return (the last pass extracted at the
    // previous prediction before refining the median).
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
/// significance gate.
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

    // Fo-array (Eq. 6): exactly one f0 per located partial, back-calculated with the median
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
/// Writing $\nu_m = f_m/m$ — the fundamental partial $m$ would imply were the string
/// harmonic — Eq. (1) squared is $\nu_m^2 = f_0^2(1 + Bm^2)$, and Eq. (8) reduces to
/// $B = (\nu_n^2 - \nu_m^2) / (n^2\nu_m^2 - m^2\nu_n^2)$: the module block's printed form with
/// a common $m^2$ cancelled. Eq. (6) then back-calculates $f_0 = f_m / (m\sqrt{1 + B m^2})$.
/// ($\nu$ is our shorthand; the paper writes Eq. (8) out in $f_m$, $f_n$ and reserves $K$
/// for the partial count of Eq. (9).)
fn compute_pair(f_m: f32, n_m: u32, f_n: f32, n_n: u32) -> Option<(f32, f32)> {
    if n_m == n_n || n_m == 0 || n_n == 0 {
        return None;
    }
    // Eq. (8): B = (ν_n² − ν_m²) / (n²·ν_m² − m²·ν_n²),  ν = f/index.
    let nu_m_sq = (f_m / n_m as f32).powi(2);
    let nu_n_sq = (f_n / n_n as f32).powi(2);
    let denom = (n_n as f32).powi(2) * nu_m_sq - (n_m as f32).powi(2) * nu_n_sq;

    if denom.abs() < 1e-8 {
        return None;
    }

    let b = (nu_n_sq - nu_m_sq) / denom;

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

    /// The reduced ν-form `compute_pair` evaluates is Eq. (8) as printed, with a common m²
    /// cancelled: both forms give the same B on a synthetic pair, and Eq. (6) returns its f0.
    #[test]
    fn pair_form_matches_printed_eq8() {
        let (f0, b) = (440.0_f32, 7.6e-4_f32);
        let (m, n) = (2u32, 5u32);
        let f_m = m as f32 * f0 * (1.0 + b * (m * m) as f32).sqrt();
        let f_n = n as f32 * f0 * (1.0 + b * (n * n) as f32).sqrt();

        // DAFx-09 Eq. (8) verbatim: ((f_n·m/n)² − f_m²) / (n²·f_m² − m²·(f_n·m/n)²).
        let r = f_n * m as f32 / n as f32;
        let printed = (r * r - f_m * f_m) / ((n * n) as f32 * f_m * f_m - (m * m) as f32 * r * r);

        let (b_pair, f0_pair) = compute_pair(f_m, m, f_n, n).expect("valid pair");
        assert!(
            (b_pair - printed).abs() / b < 1e-4,
            "reduced {b_pair} vs printed {printed}"
        );
        assert!((b_pair - b).abs() / b < 1e-3, "B {b_pair} vs truth {b}");
        assert!((f0_pair - f0).abs() < 1e-2, "f0 {f0_pair} vs truth {f0}");
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
        // A high-B treble series must not be filtered out by the pairwise B ceiling:
        // a ceiling of 0.01 rejects every genuine high-B pair.
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
