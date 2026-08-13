//! # Spectral Peak Extraction
//!
//! Stateless DSP module for extracting sub-bin accurate spectral peaks
//! from magnitude spectra. Two consumers, both scan-then-refine:
//!
//! - [`extract_peaks`] — Discovery's *global* peak list (every local maximum
//!   above an absolute threshold), fed to TWM after [`mask_peaks`].
//! - [`coarse_read`] — the tuning readout's *bounded* single-partial search
//!   around a known reference, admitted by an ordered-statistic CFAR gate.
//! - [`resolve_lines`] — the unison estimator's search over one reference
//!   partial's *baseband*, admitted by the same gate against a sliding local
//!   reference window.
//!
//! Do not fold the second onto the first: the global list is built only in the
//! discovery branch (`identified_key.is_none()`), so it is unavailable exactly
//! while a locked note is being tuned, and its Neyman–Pearson gate and masking
//! can drop a weak target partial the readout still needs.
//!
//! The third shares [`cfar_multiplier`] with the second and nothing else: it
//! reads a decimated baseband rather than a magnitude spectrum, and its
//! reference geometry is deliberately the opposite one
//! ([`UNISON_CFAR_GUARD_BINS`]).

use rustfft::Fft;
use rustfft::num_complex::Complex;

use crate::algorithms::spectral;
use crate::models::{SpectralPeak, UnisonLine};

/// Extracts all significant spectral peaks from a magnitude spectrum with sub-bin
/// interpolated frequencies using the complex-domain Jacobsen estimator.
///
/// # Algorithm
/// 1. Walk magnitudes to find local maxima (`mag[i] > mag[i-1]` AND `mag[i] > mag[i+1]`).
/// 2. Filter out peaks below `min_magnitude` (absolute threshold).
/// 3. For each surviving peak, apply the Jacobsen estimator on the `complex_spectrum`
///    for Hann-optimal sub-bin frequency interpolation.
/// 4. Sort peaks by magnitude descending. Store in `peaks_out`.
///
/// # Arguments
/// * `magnitudes` — Linear magnitude spectrum (output of `magnitude_spectrum`).
/// * `complex_spectrum` — Complex frequency spectrum from the RFFT.
/// * `sample_rate` — Audio sample rate in Hz.
/// * `fft_size` — FFT window size (e.g. 8192).
/// * `min_magnitude` — Absolute minimum linear magnitude threshold for a peak to be
///   considered. (Discovery passes a Neyman–Pearson AWGN false-alarm threshold
///   computed per frame — see the Kay 1998 derivation in `engine.rs`.)
/// * `peaks_out` — Mutable slice to write peaks into.
///
/// # Returns
/// The number of peaks extracted (up to `peaks_out.len()`).
pub fn extract_peaks(
    magnitudes: &[f32],
    complex_spectrum: &[Complex<f32>],
    sample_rate: u32,
    fft_size: usize,
    min_magnitude: f32,
    peaks_out: &mut [SpectralPeak],
) -> usize {
    if magnitudes.len() < 3 || peaks_out.is_empty() {
        return 0;
    }

    let noise_floor = min_magnitude;
    if noise_floor <= 0.0 {
        return 0; // Empty spectrum or invalid threshold
    }

    let mut temp_peaks = [SpectralPeak::default(); 128];
    let mut num_found = 0;

    // Walk magnitudes to find local maxima (avoid boundaries)
    for i in 1..(magnitudes.len() - 1) {
        let mag = magnitudes[i];

        if mag > noise_floor && mag > magnitudes[i - 1] && mag > magnitudes[i + 1] {
            // Sub-bin refinement via the complex-domain Jacobsen estimator (Candan 2015).
            let frequency = spectral::jacobsen(complex_spectrum, i, fft_size, sample_rate);

            if frequency > 0.0 && num_found < temp_peaks.len() {
                temp_peaks[num_found] = SpectralPeak {
                    frequency,
                    magnitude: mag,
                };
                num_found += 1;
            }
        }
    }

    let valid_peaks = &mut temp_peaks[..num_found];
    // Sort temp_peaks by magnitude descending
    valid_peaks.sort_unstable_by(|a, b| {
        b.magnitude
            .partial_cmp(&a.magnitude)
            .unwrap_or(core::cmp::Ordering::Equal)
    });

    // Copy to peaks_out up to its capacity
    let count = num_found.min(peaks_out.len());
    peaks_out[..count].copy_from_slice(&valid_peaks[..count]);

    count
}

/// ── Peak Masking & Dynamic-Range Gate (OURS — empirically validated) ─────
/// Filters out acoustic side-lobes, sympathetic resonance, and intermodulation
/// distortion that cause TWM to sub-harmonically false-lock.
///
/// # Provenance (faithfulness-audit-04)
/// This is the codebase's own heuristic, NOT a paper port — validated on real
/// captures in ADR 0002 (2026-05-28: replaced the failed geometric gate; 8/8
/// keys, zero false locks; known limitation: environments with SNR ≲ 30 dB).
/// * The **global dynamic-range gate** adapts Cano (1998) §4.3, which accepts
///   only peaks "less than 40 dB below the highest peak"; we ship the stricter
///   −30 dB that ADR 0002 validated.
/// * The **dominance masking** (a louder peak suppresses smaller peaks within
///   a proportional band) is ours; the 20 % bandwidth matches the textbook
///   critical-band approximation (CB ≈ 0.2·f above ~500 Hz) — inspiration,
///   not a port. No masking procedure exists in Gómez (2006) or Cano (1998);
///   do not re-cite them for it (see faithfulness-audit-04).
///
/// # Preconditions
/// The `peaks` slice must contain no more than 64 elements. If it is larger,
/// it will be artificially truncated to 64 to fit the internal tracking array.
///
/// # Reference
/// 1. ADR 0002 (`docs/adr/0002-twm-peak-masking-validation.md`) — the
///    empirical basis for the mechanism and the −30 dB values.
/// 2. Cano, P. (1998). "Fundamental Frequency Estimation in the SMS Analysis."
///    DAFx-98, §4.3 — the dynamic-range rule the global gate adapts.
///
/// # Algorithm
/// Peaks are evaluated in descending amplitude order. First, any peak more
/// than 30 dB below the global maximum is discarded (Cano's 40 dB rule,
/// tightened per ADR 0002). Then, a dominant peak masks any smaller peak that
/// falls within its proportional critical band if the smaller peak is below a
/// relative masking threshold.
pub fn mask_peaks(peaks: &mut [SpectralPeak]) -> usize {
    if peaks.is_empty() {
        return 0;
    }

    let k = peaks.len().min(64);
    let active_peaks = &mut peaks[..k];

    // 1. Sort by magnitude descending
    active_peaks.sort_unstable_by(|a, b| b.magnitude.partial_cmp(&a.magnitude).unwrap());

    let mut valid_count = 0;
    let mut masked = [false; 64];
    let global_max = active_peaks[0].magnitude;

    // OUR constants, ADR 0002-validated (not from Gómez/Cano — see doc-comment).
    const GLOBAL_THRESHOLD_DB: f32 = 0.0316; // −30 dB from global max (Cano §4.3 proposes 40 dB; ADR 0002 validated 30)
    const MASK_THRESHOLD_DB: f32 = 0.0316; // −30 dB relative to masker
    const MASK_BANDWIDTH_PROPORTION: f32 = 0.20; // ≈ textbook critical band (CB ≈ 0.2·f above ~500 Hz)

    for i in 0..k {
        if masked[i] {
            continue;
        }

        // Global dynamic-range gate: −30 dB from the frame's maximum (ADR 0002).
        // Prevents the engine from analyzing isolated microscopic acoustic room noise.
        if active_peaks[i].magnitude < global_max * GLOBAL_THRESHOLD_DB {
            continue;
        }

        let masker_freq = active_peaks[i].frequency;
        let masker_mag = active_peaks[i].magnitude;

        let mask_threshold = masker_mag * MASK_THRESHOLD_DB;
        let mask_bw = masker_freq * MASK_BANDWIDTH_PROPORTION;

        // Mask neighboring weaker peaks
        for j in (i + 1)..k {
            if !masked[j] {
                let target_freq = active_peaks[j].frequency;
                let target_mag = active_peaks[j].magnitude;

                if (target_freq - masker_freq).abs() < mask_bw && target_mag < mask_threshold {
                    masked[j] = true;
                }
            }
        }

        // Retain valid peak
        active_peaks[valid_count] = active_peaks[i];
        valid_count += 1;
    }

    // Sort by frequency ascending for O(N+K) two-pointer sweep in TWM
    active_peaks[..valid_count]
        .sort_unstable_by(|a, b| a.frequency.partial_cmp(&b.frequency).unwrap());

    valid_count
}

// ─── Coarse Readout: bounded search + OS-CFAR ────────────────────────────────

/// Search half-width in **cents** — scale-invariant, so one value serves A0 and
/// C8. Ours (ADR 0011).
///
/// This *is* the readout's reach: a peak outside the band cannot be the argmax,
/// so the measured cliff — 100 % availability at 0.1–3.1 ¢ of error out to 75 ¢
/// of detuning, nothing past 100 ¢ — is the band edge, not a phenomenon. Widening
/// buys reach during a pitch raise; narrowing buys nothing, because it is **not**
/// what prevents mis-selection. That is the neighbour cap in
/// [`search_halfwidth_hz`]: within one sounding note there is no competitor at
/// the neighbouring key's pitch, and the cap keeps the band inside `spacing/2`
/// regardless of this value. The residual risk is a *second sounding key* —
/// sympathetic ring or an adjacent strike — which is a measurable failure mode,
/// not something a narrower span fixes.
const COARSE_SPAN_CENTS: f32 = 100.0;

/// Floor on the search half-width in **bins**, for registers where
/// [`COARSE_SPAN_CENTS`] is sub-bin (±100 ¢ at A0 is ±1.6 Hz — under a third of
/// one 8192 bin). Ours (ADR 0011).
///
/// What it prevents is a **degenerate band**, not a precision loss: below one bin
/// the search collapses to `lo >= hi` and [`coarse_read`] returns `None`, so
/// wherever the cents span is sub-bin the read would simply vanish. The derived
/// lower bound is therefore ~1 bin, and the exact value above that is not
/// load-bearing — 3 vs 4 bins is inert on real captures (availability 92.1 % both,
/// |e| 23.67 vs 23.68 ¢, jitter 3.32 vs 3.28 ¢ over keys 8–26).
///
/// Do not adopt this floor without the neighbour cap in [`search_halfwidth_hz`]:
/// uncapped at 2048 it is an 86 Hz half-width and the read returns the 2nd
/// partial (+1200 ¢). The cap is also what makes it inert in the deep bass, where
/// `spacing/2` is under 3 bins; between there and ≈ key 44 (the point where the
/// cents span overtakes it) the floor is the term that sets the band.
const COARSE_SPAN_MIN_BINS: f32 = 4.0;

/// Order statistic taken as the local noise estimate, as a fraction of the
/// reference count — Rohling's rank parameter `k/N`.
///
/// Fixed by the paper's own interference criterion (§V): an inhomogeneity in the
/// reference window is tolerable only while it "affects less than (N − k)
/// resolution cells". Here the interferer is the harmonic comb itself, so with
/// partial spacing `s` bins and a Hann main lobe `W_lobe = 4` bins wide
/// null-to-null,
///
/// ```text
///   (W_lobe / s)·N  ≤  N − k      ⇒      k/N  ≤  1 − W_lobe / s
/// ```
///
/// A0 is the binding case and this value meets its bound with no margin:
/// `s = 27.5/5.383 = 5.11` bins, measured lobe occupancy 75 % of reference cells
/// ⇒ `k/N ≤ 0.25`. The bound relaxes monotonically upward (0.53 at F1), so the
/// deep bass sets it for the whole keyboard.
///
/// The departure from the paper's own `k > N/2` recommendation is forced by that
/// same criterion, not taken against it: a radar reference window is mostly
/// clutter with a few interfering targets, ours is mostly partials with few
/// background cells, so the inequality binds from the other side. Its cost is the
/// one Rohling names for `k < N/2` — "erosion", under-estimation at an edge —
/// bounded by the realized-P_fa measurement in ADR 0011 §5.
///
/// Do not raise it: at the median the bound is violated for every key up to F1,
/// and the deep bass admits ±400 ¢ junk.
const COARSE_CFAR_QUANTILE: f32 = 0.25;

/// No guard cells: this detector has none, by Rohling §V — "in OS CFAR
/// processing these guard cells become unnecessary since a small number of target
/// amplitudes occurring within the reference area have almost no influence on the
/// clutter level estimation by quantiles" (his Fig. 9 window, as against Fig. 3's
/// guarded CA/CAGO one). Structurally so here: references come from *outside* the
/// search band, so the only cells of the peak's own lobe that can enter are the
/// ones just past a band edge, and those are high magnitudes that sort above a
/// low quantile. Measured: a ±0…4-bin guard moves availability, error and jitter
/// by nothing on any of the three capture sets (audit 13).
/// Reference half-width as a multiple of the partial spacing. Ours, measured
/// (ADR 0011).
const COARSE_CFAR_FLANK_SPACINGS: f32 = 1.5;

/// Floor on the reference half-width, in **Hz, not bins** — a bin is 5.4 Hz at
/// 8192 and 21.5 Hz at 2048, so a bin-specified floor would silently quadruple
/// when the read switches size. Ours, measured (ADR 0011).
///
/// It exists because deep-bass partials are ≈ 5 bins apart at 8192, so 75 % of
/// cells lie inside some partial's main lobe and there is no inter-partial valley
/// to reach: [`COARSE_CFAR_FLANK_SPACINGS`] × spacing would sample only the
/// *strong* low partials and the order statistic would read signal as noise.
/// Widening to 172 Hz spans partials ≈ 1–11 at A0 and so imports the **weak
/// upper** ones, whose skirts sit 19–36 dB below the band peak; that is what the
/// low quantile lands on. Measured: the selected cell is a partial's lobe in
/// ≈ 56–68 % of deep-bass hops, and a valley cell in ≥ 95 % of hops from F1 up.
///
/// **Hz is the correct unit here, not a fallback.** The floor's job is to widen
/// the flank in the register where the comb is dense and to *disappear* where it
/// is not, and only an absolute frequency does both: a floor in **bins** is a
/// different physical width at each FFT size (5.4 Hz at 8192, 21.5 at 2048), and
/// a floor in **partial spacings** never turns off — 6.3 spacings is 172 Hz at A0
/// but 2.8 kHz at A4, a reference window spanning DC to 3 kHz around a 440 Hz
/// partial. Expressing it as "reach partial *m*" is also refuted by measurement:
/// the cell the order statistic selects is whichever partial is weakest at that
/// hop, and its index ranges over n1–n11 with no stable median across keys, so
/// there is no *m* to reach.
///
/// What remains is an amplitude-envelope property rather than window geometry,
/// so it is not reducible to a scale-free formula on one instrument. It is
/// active only where `1.5 × spacing < 172 Hz`, i.e. spacing below 115 Hz
/// (≈ key ≤ 25), and inert above — an absolute frequency, but a bounded one.
const COARSE_CFAR_FLANK_MIN_HZ: f32 = 172.0;

/// False-alarm probability both CFAR gates in this file are calibrated to — the
/// same 0.001 [`spectral::neyman_pearson_k`] commits to, so the gates differ only
/// in *which* noise they measure, never in how permissive they are.
const CFAR_P_FA: f32 = 0.001;

/// Minimum reference cells for a usable noise estimate: below this the local
/// null is unidentifiable and the read is withheld rather than guessed. Reached
/// when the flanks are clipped by the spectrum's own edges — a reference close
/// to DC or to Nyquist.
///
/// A refusal floor, not an operating point. The structural minimum is **2** (the
/// rank clamp below needs `n_ref ≥ 2` for an order statistic to exist at all);
/// the margin above it is free because the gate is already effectively closed
/// there — at four references the multiplier is `cfar_multiplier(2, 1, ·)`, i.e.
/// `T_q = 2/p − 2` ⇒ `T_lin ≈ 45`, some 6× the working threshold. For scale, the
/// shipped read normally has 53–57 references, and Rohling's own OS-CFAR window
/// sizes are `N = 24 … 32 and more`.
const COARSE_CFAR_MIN_REFS: usize = 4;

/// Search half-width in Hz. Three terms, in order of precedence:
///
/// 1. [`COARSE_SPAN_CENTS`] — register-proportional;
/// 2. [`COARSE_SPAN_MIN_BINS`] — a floor where that span is sub-bin;
/// 3. a **neighbour cap at half the partial spacing**, which overrides both.
///
/// `spacing_hz` is the distance to the neighbouring partial (≈ f₀) and is
/// **not** interchangeable with `center_hz`: they coincide only at n = 1. A read
/// centred on A0's 4th partial has `center_hz ≈ 110` but `spacing_hz ≈ 27.5`,
/// and a cap at `center_hz / 2` would admit a ±55 Hz band spanning two
/// neighbours.
///
/// A band the cap leaves under one bin means that FFT size cannot serve that
/// register — the size-selection rule doing its job, not a knob to widen.
fn search_halfwidth_hz(center_hz: f32, spacing_hz: f32, hz_per_bin: f32) -> f32 {
    let span = center_hz * (2f32.powf(COARSE_SPAN_CENTS / 1200.0) - 1.0);
    span.max(COARSE_SPAN_MIN_BINS * hz_per_bin)
        .min(spacing_hz / 2.0)
}

/// **Exact finite-`N` OS-CFAR threshold multiplier** — a port of Rohling
/// (1983) Eqs. 14 + 17.
///
/// His Eq. 14 gives the false-alarm probability of an ordered-statistic CFAR
/// detector with `n_ref` reference cells selecting rank `k`, for an
/// **exponentially** distributed (square-law detector) parent population:
///
/// ```text
///   P_fa = k·C(N,k)·Γ(k)·Γ(T+N−k+1) / Γ(T+N+1)
/// ```
///
/// The gamma ratio telescopes for integer `k` — `Γ(T+N−k+1)/Γ(T+N+1)` is
/// `1/∏_{j=0}^{k−1}(T+N−j)` — and the combinatorial prefactor reduces to
/// `N!/(N−k)!`, leaving the product form evaluated here:
///
/// ```text
///   P_fa = ∏_{j=0}^{k−1} (N−j)/(T+N−j)
/// ```
///
/// which is exact, strictly decreasing in `T`, and needs no gamma function —
/// so `T` follows by bisection.
///
/// Our cells are Rayleigh **magnitudes**, not exponential powers. The paper's
/// closing section derives the linear-detector conversion **`T_lin = √T_q`** (its
/// `T_q` is the square-law factor of Table II — not a quantile) for exactly the
/// case where the receiver takes the absolute value and the cells "obey a
/// Rayleigh distribution", and scopes it explicitly to this detector: "this
/// simple conversion, however, does not apply for CA or CAGO CFAR". That
/// conversion is the `sqrt` below, making this a port of Eqs. 14 + 17 rather than
/// a bespoke calibration.
///
/// Table II itself is tabulated at `P_fa = 10⁻⁶`, so it is not the operating
/// point here, but it is a direct check on this implementation:
/// `coarse_cfar_multiplier_table_ii` reproduces all 32 of its `N = 32` entries.
///
/// As `N → ∞` with `k = q·N` the product tends to
/// `T_lin → √(ln P_fa / ln(1−q))` (3.157 at the median for P_fa = 0.001) — the
/// asymptotic quantile form, pinned against this exact one by
/// `coarse_cfar_multiplier_pinned`.
///
/// Returns infinity for an unusable rank, so a caller that cannot form a
/// legitimate order statistic admits nothing.
///
/// # Reference
/// Rohling, H. (1983). "Radar CFAR Thresholding in Clutter and Multiple Target
/// Situations." IEEE Trans. Aerospace and Electronic Systems, AES-19(4),
/// pp. 608–621. DOI: 10.1109/TAES.1983.309350. (Eqs. 9–10, 12, 14, 17.)
/// Lineage: Finn, H. M. & Johnson, R. S. (1968). "Adaptive Detection Mode with
/// Threshold Control as a Function of Spatially Sampled Clutter-Level
/// Estimates." RCA Review 29(3), pp. 414–464 — the cell-averaging predecessor.
fn cfar_multiplier(n_ref: usize, k: usize, p_fa: f32) -> f32 {
    if n_ref == 0 || k == 0 || k > n_ref {
        return f32::INFINITY; // Unusable rank ⇒ admit nothing.
    }
    let n = n_ref as f64;
    let pfa = |t: f64| -> f64 {
        let mut p = 1.0f64;
        for j in 0..k {
            let jf = j as f64;
            p *= (n - jf) / (t + n - jf);
        }
        p
    };
    // P_fa is strictly decreasing in T. 60 halvings of [0, 1e6] resolve T_sq
    // far below f32 precision, so the cast below is exact for any wider search.
    let (mut lo, mut hi) = (0.0f64, 1.0e6f64);
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if pfa(mid) > p_fa as f64 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    // Rohling Eq. 17: the linear (magnitude) detector takes the square root.
    (0.5 * (lo + hi)).sqrt() as f32
}

/// **The coarse readout** — bounded argmax around one reference partial,
/// admitted by an ordered-statistic CFAR gate, refined sub-bin by
/// [`spectral::jacobsen`]. Returns the partial's frequency in Hz, or `None`
/// when nothing at the reference clears the local noise.
///
/// The wide-range companion to the strobe band, whose phase read is the more
/// accurate one but aliases past ±0.5·fs/HOP ≈ 21.5 Hz — in cents ≈ 37200/f, so
/// only ±9 ¢ at C8. A magnitude read costs jitter and buys range, so it stays
/// correct through a pitch raise.
///
/// # Gate
/// Ordered-statistic CFAR (Rohling 1983; Finn & Johnson 1968 lineage) sets the
/// threshold from the *neighbourhood* of the cell under test rather than from a
/// calibrated absolute floor, so the null tracks the note that is sounding
/// instead of the quiet room. Reference cells come from **flanks outside** the
/// search band, never from inside it — in the deep bass the capped band is
/// ≈ 5 bins and the guard cells consume all of it.
///
/// The low [`COARSE_CFAR_QUANTILE`] follows the paper's own §V interference
/// criterion applied to a harmonic comb; the one adaptation that is ours is the
/// **search-loss correction**, measured (ADR 0011 §5). Rohling's
/// P_fa governs *one* cell under test, but this detector takes the argmax over
/// the whole band and so gets one chance to false-alarm per cell; the
/// multiple-comparisons budget is therefore `P_fa / M`, with `M` the band width
/// halved because Hann correlation makes adjacent bins non-independent. Without
/// it the realized rate runs ~32× nominal.
///
/// # Arguments
/// * `magnitudes` — linear magnitude spectrum (`fft_size / 2` bins).
/// * `complex_spectrum` — the same frame's complex spectrum, for the refiner.
/// * `fft_size` — FFT length behind those spectra (2048 or 8192).
/// * `sample_rate` — audio sample rate in Hz.
/// * `center_hz` — the reference frequency of the partial being read.
/// * `spacing_hz` — the key's partial spacing (≈ f₀), which sets both the
///   neighbour cap and the reference flank width. **Not** `center_hz` — see
///   [`search_halfwidth_hz`].
/// * `scratch` — reference-cell workspace, reused across hops so the hot path
///   allocates nothing. Must hold at least `magnitudes.len()` elements, the
///   most cells the flanks can ever yield.
///
/// # Panics
/// In debug builds, if `scratch` is shorter than `magnitudes`. In release it
/// truncates the reference set, which biases the order statistic — a caller
/// bug, not a runtime condition.
pub fn coarse_read(
    magnitudes: &[f32],
    complex_spectrum: &[Complex<f32>],
    fft_size: usize,
    sample_rate: u32,
    center_hz: f32,
    spacing_hz: f32,
    scratch: &mut [f32],
) -> Option<f32> {
    debug_assert!(
        scratch.len() >= magnitudes.len(),
        "coarse_read scratch must hold one cell per bin"
    );

    let n_bins = magnitudes.len();
    if n_bins < 4
        || fft_size == 0
        || !center_hz.is_finite()
        || center_hz <= 0.0
        || !spacing_hz.is_finite()
        || spacing_hz <= 0.0
    {
        return None;
    }
    let hz_per_bin = sample_rate as f32 / fft_size as f32;

    // ── Band geometry ──
    let half = search_halfwidth_hz(center_hz, spacing_hz, hz_per_bin);
    if !half.is_finite() {
        return None;
    }
    let lo = ((center_hz - half) / hz_per_bin).floor().max(1.0) as usize;
    let hi = (((center_hz + half) / hz_per_bin).ceil() as usize).min(n_bins - 2);
    if lo >= hi {
        return None;
    }

    let (mut best, mut best_mag) = (lo, 0.0f32);
    for (offset, &m) in magnitudes[lo..=hi].iter().enumerate() {
        if m > best_mag {
            best_mag = m;
            best = lo + offset;
        }
    }

    // ── Local noise estimate from the flanking reference cells ──
    // Both terms in Hz, converted once: the floor and the spacing rule must be
    // compared in physical units, not bins.
    let flank_hz = (COARSE_CFAR_FLANK_SPACINGS * spacing_hz).max(COARSE_CFAR_FLANK_MIN_HZ);
    // Saturating: a large `spacing_hz` saturates the float→int cast to
    // `usize::MAX`, and a plain `+` would then overflow.
    let flank = (flank_hz / hz_per_bin).ceil() as usize;
    let outer_lo = lo.saturating_sub(flank).max(1);
    let outer_hi = hi.saturating_add(flank).min(n_bins - 2);

    let mut n_ref = 0usize;
    for bin in (outer_lo..lo).chain((hi + 1)..=outer_hi) {
        if n_ref < scratch.len() {
            scratch[n_ref] = magnitudes[bin];
            n_ref += 1;
        }
    }
    if n_ref < COARSE_CFAR_MIN_REFS {
        return None;
    }

    let refs = &mut scratch[..n_ref];
    let rank = (((n_ref - 1) as f32 * COARSE_CFAR_QUANTILE).round() as usize).clamp(1, n_ref - 1);
    let (_, noise, _) = refs.select_nth_unstable_by(rank, f32::total_cmp);
    let noise = *noise;

    // Three factors of two, calibrated as one: Hann correlation is taken to halve
    // both the effective reference count and the band's independent cells, and
    // the argmax costs a per-cell P_fa budget over the latter. The textbook
    // figure is Hann's ENBW of 1.5 bins, not 2, so each is individually
    // conservative — but the composite lands on nominal (realized 0.00097 vs
    // 0.001, ADR 0011 §5). Do not "fix" one of them alone; they are only
    // validated together.
    let m_eff = (hi - lo).div_ceil(2).max(1) as f32;
    let threshold =
        noise * cfar_multiplier((n_ref / 2).max(2), (rank / 2).max(1), CFAR_P_FA / m_eff);
    if !(threshold.is_finite() && threshold > 0.0 && best_mag >= threshold) {
        return None;
    }

    let f = spectral::jacobsen(complex_spectrum, best, fft_size, sample_rate);
    (f.is_finite() && f > 0.0).then_some(f)
}

// ─── Unison lines: baseband zoom-DFT + sliding local OS-CFAR ─────────────────

/// Lines one reference can report — a three-string unison, the widest the piano
/// builds. The cap means "the three strongest *admitted* lines", since candidates
/// are magnitude-sorted and the CFAR loop stops at the first rejection.
pub const MAX_UNISON_LINES: usize = 3;

/// Guard cells either side of the cell under test, in bins — the Hann main lobe
/// of the cell itself, whose skirt is not noise.
///
/// Do not give this detector [`coarse_read`]'s reference geometry: references
/// drawn from flanks *outside* the search band exclude the dominant line's own
/// skirt, so a secondary maximum riding that skirt is compared against distant
/// background and admitted. Measured on a *single* synthetic string, up to
/// **26.7 % false second lines** (ADR 0012 §2).
const UNISON_CFAR_GUARD_BINS: usize = 2;

/// Reference cells per side, at circular distance
/// `(GUARD, GUARD + UNISON_CFAR_WINDOW_BINS]` from the cell under test — 32
/// total, the same order as Rohling's own `N = 24 … 32 and more`.
///
/// The window slides with the cell under test rather than flanking a fixed band,
/// so the reference is always the local background. Where the record is too short
/// to reach this far the set degrades to *every* bin outside the guard, which is
/// the same thing said with fewer cells; [`UNISON_MIN_BINS`] is the length below
/// which that stops being enough.
const UNISON_CFAR_WINDOW_BINS: usize = 16;

/// Order statistic taken as the local noise estimate, as a fraction of the
/// reference count — Rohling's rank parameter `k/N`, here the paper's own median.
///
/// Unlike [`COARSE_CFAR_QUANTILE`], which the harmonic comb drives *below* the
/// paper's `k > N/2` recommendation, this window sees at most the note's other
/// strings. Rohling §V binds from the usual side and is satisfied with margin:
/// two interfering lines occupy `2 × (2·GUARD + 1) = 10` cells of the reference
/// window against the `N_ref − k = 16` the criterion allows.
///
/// Do not raise it to the paper's own worked `q = 0.75`: with three lines
/// present the reference window is signal-dominated at the upper quantile, and a
/// three-string unison is lost outright (detection 100 % → 0 %, ADR 0012 §2).
const UNISON_CFAR_QUANTILE: f32 = 0.50;

/// Rayleigh criterion, in bins: two lines closer than the Hann main-lobe
/// half-width (`2/T` Hz) are one line, and reporting them as two is a measured
/// failure mode. Applied to the *refined* positions — at the integer grid the
/// test is inert, because two distinct local maxima are already two bins apart.
const UNISON_MERGE_BINS: f32 = 2.0;

/// Reference cells Rohling's §V interference criterion demands of this geometry:
/// an inhomogeneity is tolerable only while it "affects less than `(N − k)`
/// resolution cells", and here the inhomogeneity is the note's *other* strings —
/// [`MAX_UNISON_LINES`] − 1 of them, each occupying its own main lobe
/// (`2·GUARD + 1` cells).
const UNISON_MIN_REFS: usize = (((MAX_UNISON_LINES - 1) * (2 * UNISON_CFAR_GUARD_BINS + 1)) as f32
    / (1.0 - UNISON_CFAR_QUANTILE)) as usize;

/// Shortest transform [`resolve_lines`] will run — the length at which the
/// reference window still holds [`UNISON_MIN_REFS`] cells once the cell under
/// test and its guard are excluded.
///
/// This is the estimator's own floor, not a display policy: below it the
/// reference window is signal-dominated whenever a second string is present, so
/// "one line" stops meaning "no second line" and starts meaning "the detector is
/// blind". [`strobe::unison`](crate::strobe::unison) honours it as the ring's
/// publish floor.
pub(crate) const UNISON_MIN_BINS: usize = UNISON_MIN_REFS + 2 * UNISON_CFAR_GUARD_BINS + 1;

/// Caller-owned working buffers for [`resolve_lines`], reused across hops so the
/// hot path allocates nothing. [`Self::spectrum`] and [`Self::magnitudes`] must
/// hold at least the transform length, [`Self::fft`] at least the plan's
/// `get_inplace_scratch_len()`.
pub struct LineScratch<'a> {
    /// Windowed baseband, transformed in place.
    pub spectrum: &'a mut [Complex<f32>],
    /// `|Z[m]|`, read by the candidate scan and the reference cells.
    pub magnitudes: &'a mut [f32],
    /// `rustfft`'s own in-place scratch.
    pub fft: &'a mut [Complex<f32>],
}

/// **The unison line estimator** — resolves the individual strings of one
/// reference partial as separate spectral lines, each a signed Hz offset from
/// that reference. Returns how many were written to `out`.
///
/// # What it is
/// A **zoom FFT** (Lyons, *Understanding DSP*, ch. 13) whose front end is already
/// running: the strobe's Hann-windowed Goertzel at `f_ref` is the mixer and
/// anti-alias filter, and taking one output per hop is the decimator, so
/// `baseband[h]` is a sum of damped complex exponentials turning at each string's
/// **offset** from the target. Sampled at `hop_rate_hz`, it is unambiguous over
/// ±`hop_rate_hz/2` ≈ ±21.5 Hz. Resolution is set by observation time, not bin
/// count — 50 % of pairs resolve at `2/T`, 100 % at ≈1.35·`2/T` — which is why
/// the caller must publish `2/T` alongside the lines.
///
/// # Steps
/// 1. Hann-window the record and take an `N`-point complex DFT, `N` =
///    `baseband.len()`. **Natural Fourier bins, no zero-padding**: padded bins are
///    interpolated rather than independent, and the CFAR null below assumes
///    independence.
/// 2. Take circular local maxima of `|Z|` in descending magnitude — the baseband
///    spectrum wraps, so bin 0 and bin `N−1` are neighbours and there is no edge
///    case.
/// 3. Refine each sub-bin by the three-bin Candan estimator on the complex bins
///    (the same Eq. 1 [`spectral::jacobsen`] ports, evaluated circularly), then
///    reject any candidate within [`UNISON_MERGE_BINS`] of an accepted stronger
///    one.
/// 4. Admit by an ordered-statistic CFAR gate against a **sliding local**
///    reference window ([`UNISON_CFAR_WINDOW_BINS`], [`UNISON_CFAR_GUARD_BINS`],
///    [`UNISON_CFAR_QUANTILE`]), reusing [`cfar_multiplier`]. Candidates are
///    magnitude-sorted, so the first rejection ends the list.
///
/// The Hann halvings and the `m_eff` search-loss divisor follow [`coarse_read`]'s
/// calibrated pattern: this detector also takes an argmax, so ADR 0011 §5's
/// correction applies, over the whole record rather than a bounded band.
///
/// # Arguments
/// * `baseband` — the per-reference complex baseband, **oldest first**. Its
///   length is the transform length; below [`UNISON_MIN_BINS`] nothing is
///   reported. There is no upper bound — how long a record is worth keeping is
///   the caller's policy, not this function's.
/// * `fft` — a complex forward transform planned for exactly `baseband.len()`,
///   built once at startup and held by the caller.
/// * `c_n` — [`spectral::candan_c_n`] at that length. Passed in rather than
///   evaluated here because it is `O(N)` in trigonometry and constant per length;
///   the 2.0 asymptote is a 2.4 % scale error on every offset at these sizes.
/// * `hop_rate_hz` — the baseband's sample rate, i.e. the DSP hop rate.
/// * `scratch` — see [`LineScratch`].
/// * `out` — receives up to `min(out.len(), MAX_UNISON_LINES)` lines, strongest
///   first, with `relative_amplitude` normalised to that strongest line.
///
/// # Panics
/// In debug builds, if `fft`'s length disagrees with `baseband`'s or a scratch
/// buffer is too short. In release those are runtime `0`-returns rather than
/// wrong answers.
///
/// # Reference
/// Rohling, H. (1983). "Radar CFAR Thresholding in Clutter and Multiple Target
/// Situations." IEEE Trans. AES-19(4) — the admission gate; §V sets the rank.
/// Candan, Ç. (2015). Signal Processing 114, Eq. 1 — the sub-bin refinement.
/// Lyons, R. (2010). *Understanding DSP*, ch. 13 — the zoom-FFT structure.
pub fn resolve_lines(
    baseband: &[Complex<f32>],
    fft: &dyn Fft<f32>,
    c_n: f32,
    hop_rate_hz: f32,
    scratch: &mut LineScratch<'_>,
    out: &mut [UnisonLine],
) -> usize {
    let n = baseband.len();
    debug_assert_eq!(
        fft.len(),
        n,
        "resolve_lines: the plan must match the record"
    );
    debug_assert!(
        scratch.spectrum.len() >= n
            && scratch.magnitudes.len() >= n
            && scratch.fft.len() >= fft.get_inplace_scratch_len(),
        "resolve_lines scratch is undersized"
    );
    let max_lines = out.len().min(MAX_UNISON_LINES);
    if n < UNISON_MIN_BINS
        || max_lines == 0
        || fft.len() != n
        || scratch.spectrum.len() < n
        || scratch.magnitudes.len() < n
        || scratch.fft.len() < fft.get_inplace_scratch_len()
        || !hop_rate_hz.is_finite()
        || hop_rate_hz <= 0.0
    {
        return 0;
    }

    // ── Window and transform ──
    // Hann over [0, N−1] — the window `c_n` is derived for and the one the rest
    // of the pipeline applies.
    let n_minus_1 = (n - 1) as f32;
    for (i, (dst, &src)) in scratch.spectrum[..n].iter_mut().zip(baseband).enumerate() {
        let w = 0.5 * (1.0 - (2.0 * core::f32::consts::PI * i as f32 / n_minus_1).cos());
        *dst = src * w;
    }
    let scratch_len = fft.get_inplace_scratch_len();
    fft.process_with_scratch(&mut scratch.spectrum[..n], &mut scratch.fft[..scratch_len]);
    for (mag, z) in scratch.magnitudes[..n]
        .iter_mut()
        .zip(scratch.spectrum.iter())
    {
        *mag = z.norm();
    }

    // ── Candidates, strongest first ──
    let half = n as f32 / 2.0;
    let bin_hz = hop_rate_hz / n as f32;
    let mut positions = [0.0f32; MAX_UNISON_LINES];
    let mut found = 0usize;
    let mut strongest = 0.0f32;
    // The last candidate examined. Advancing a cursor through a strict total
    // order is what lets the scan skip what it has already taken without a
    // visited set — and so without any bound on the record length.
    let mut cursor: Option<(f32, usize)> = None;

    while found < max_lines {
        let mut best: Option<(f32, usize)> = None;
        for m in 0..n {
            let mag = scratch.magnitudes[m];
            let prev = scratch.magnitudes[(m + n - 1) % n];
            let next = scratch.magnitudes[(m + 1) % n];
            if !(mag > prev && mag > next) {
                continue;
            }
            let candidate = (mag, m);
            if cursor.is_some_and(|taken| !precedes(taken, candidate)) {
                continue;
            }
            if best.is_none_or(|b| precedes(candidate, b)) {
                best = Some(candidate);
            }
        }
        let Some((mag, bin)) = best else { break };
        cursor = best;

        // Rayleigh merge, on the refined positions: at the integer grid two
        // distinct local maxima are already two bins apart, so the test would
        // never fire. A merged candidate is skipped, not terminal — the next one
        // down may be a genuine third string.
        let position = wrap_signed(
            refine_circular(scratch.spectrum, bin, n, c_n),
            n as f32,
            half,
        );
        if positions[..found]
            .iter()
            .any(|p| circular_gap(*p, position, n as f32) < UNISON_MERGE_BINS)
        {
            continue;
        }

        if !admits(scratch.magnitudes, bin, n, mag) {
            break; // magnitude-sorted ⇒ the first rejection ends the list
        }

        if found == 0 {
            strongest = mag;
        }
        positions[found] = position;
        out[found] = UnisonLine {
            offset_hz: position * bin_hz,
            relative_amplitude: if strongest > 0.0 {
                mag / strongest
            } else {
                0.0
            },
        };
        found += 1;
    }

    found
}

/// Strict total order on candidates: descending magnitude, ties broken by
/// ascending bin. Total rather than merely descending because two bins of equal
/// magnitude must still order, or a scan that advances by "strictly after the
/// last one taken" would never leave them.
fn precedes(a: (f32, usize), b: (f32, usize)) -> bool {
    match a.0.total_cmp(&b.0) {
        core::cmp::Ordering::Greater => true,
        core::cmp::Ordering::Less => false,
        core::cmp::Ordering::Equal => a.1 < b.1,
    }
}

/// Circular distance between two bin indices.
fn circular_bins(a: usize, b: usize, n: usize) -> usize {
    let d = a.abs_diff(b);
    d.min(n - d)
}

/// Circular distance between two fractional bin positions on a length-`n` ring.
fn circular_gap(a: f32, b: f32, n: f32) -> f32 {
    let d = (a - b).abs();
    d.min(n - d)
}

/// Folds a bin position into `[−n/2, n/2)` — the baseband is signed around its
/// reference, so bin `n − 1` is offset −1, not +(n − 1).
fn wrap_signed(position: f32, n: f32, half: f32) -> f32 {
    let folded = position.rem_euclid(n);
    if folded >= half { folded - n } else { folded }
}

/// Candan Eq. 1 on the three complex bins around `bin`, taken circularly.
/// Falls back to the bin centre on a degenerate denominator, as
/// [`spectral::jacobsen`] does.
fn refine_circular(spectrum: &[Complex<f32>], bin: usize, n: usize, c_n: f32) -> f32 {
    let prev = spectrum[(bin + n - 1) % n];
    let peak = spectrum[bin];
    let next = spectrum[(bin + 1) % n];
    let numerator = prev - next;
    let denominator = Complex::new(2.0, 0.0) * peak - prev - next;
    let delta = if denominator.norm_sqr() > 1e-12 {
        c_n * (numerator / denominator).re
    } else {
        0.0
    };
    bin as f32 + if delta.is_finite() { delta } else { 0.0 }
}

/// The ordered-statistic CFAR admission test for one cell under test.
fn admits(magnitudes: &[f32], bin: usize, n: usize, mag: f32) -> bool {
    let mut cells = [0.0f32; 2 * UNISON_CFAR_WINDOW_BINS];
    let mut n_ref = 0usize;
    for (b, &m) in magnitudes[..n].iter().enumerate() {
        let d = circular_bins(b, bin, n);
        if d > UNISON_CFAR_GUARD_BINS
            && d <= UNISON_CFAR_GUARD_BINS + UNISON_CFAR_WINDOW_BINS
            && n_ref < cells.len()
        {
            cells[n_ref] = m;
            n_ref += 1;
        }
    }
    if n_ref < UNISON_MIN_REFS {
        return false;
    }

    let refs = &mut cells[..n_ref];
    let rank = (((n_ref - 1) as f32 * UNISON_CFAR_QUANTILE).round() as usize).clamp(1, n_ref - 1);
    let (_, noise, _) = refs.select_nth_unstable_by(rank, f32::total_cmp);
    let noise = *noise;

    // The same three calibrated factors of two as `coarse_read`: Hann
    // correlation halves the effective reference count and the record's
    // independent cells, and the argmax costs a per-cell budget over the latter.
    // The search here spans the whole record, there being no bounded band.
    let m_eff = (n / 2).max(1) as f32;
    let threshold =
        noise * cfar_multiplier((n_ref / 2).max(2), (rank / 2).max(1), CFAR_P_FA / m_eff);
    threshold.is_finite() && threshold > 0.0 && mag >= threshold
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{BASS_WINDOW_SIZE, SAMPLE_RATE, WINDOW_SIZE};

    /// Rohling Eq. 14 + 17 at the ranks the gate actually uses, against values
    /// computed independently from the closed form, plus the asymptotic limit
    /// `T_lin → √(ln P_fa / ln(1−q))` = 3.157 at the median for P_fa = 0.001.
    /// These pin the *calibration*: a drift here silently changes how permissive
    /// every coarse read is.
    #[test]
    fn coarse_cfar_multiplier_pinned() {
        for &(n, k, want) in &[
            (10usize, 5usize, 4.8346f32),
            (16, 8, 4.0894),
            (32, 16, 3.5822),
            (64, 32, 3.3604),
        ] {
            let got = cfar_multiplier(n, k, 0.001);
            assert!(
                (got - want).abs() < 1e-3,
                "N={n} k={k}: expected {want}, got {got}"
            );
        }
        // Asymptotic agreement (the two forms are mutually validating).
        let limit = cfar_multiplier(100_000, 50_000, 0.001);
        assert!(
            (limit - 3.1569).abs() < 5e-3,
            "finite-N must converge to the quantile form, got {limit}"
        );
        // Monotone in P_fa: a tighter budget demands a higher threshold — the
        // property the search-loss correction relies on.
        assert!(cfar_multiplier(64, 16, 1e-5) > cfar_multiplier(64, 16, 1e-3));
        // Unusable rank admits nothing rather than defaulting to something.
        assert!(cfar_multiplier(0, 1, 0.001).is_infinite());
        assert!(cfar_multiplier(8, 9, 0.001).is_infinite());
    }

    /// **Rohling Table II reproduced.** The paper tabulates the square-law
    /// scaling factor `T_q` at `P_fa = 10⁻⁶` for `N ∈ {8, 16, 24, 32}`; squaring
    /// this function's output (Eq. 17 inverted) must return it. That checks Eq. 14
    /// against the source's own numbers rather than against our re-derivation of
    /// it — the strongest available guard on the port.
    ///
    /// `k = 1` is excluded: `T_q = N/P_fa − N` reaches 3.2 × 10⁷ there, above the
    /// bisection's `1e6` interval. Unreachable in use — the widest shipped band
    /// puts `T_q` under 2.5 × 10⁴ — but it is a real bound on the search, not a
    /// property of the formula.
    #[test]
    fn coarse_cfar_multiplier_table_ii() {
        // (N, k, T_q) transcribed from Table II, journal p. 616.
        for &(n, k, t_q) in &[
            (8usize, 2usize, 7475.8f32),
            (8, 4, 196.0),
            (8, 8, 16.8),
            (16, 4, 442.7),
            (16, 8, 56.6),
            (16, 16, 8.3),
            (24, 8, 94.1),
            (24, 17, 18.6), // the paper's own worked example (N = 24, k = 17)
            (24, 24, 6.3),
            (32, 2, 31464.5),
            (32, 8, 131.3),
            (32, 17, 29.2),
            (32, 24, 14.4),
            (32, 32, 5.4),
        ] {
            let got = cfar_multiplier(n, k, 1e-6).powi(2);
            let rel = (got - t_q).abs() / t_q;
            assert!(
                rel < 0.01,
                "N={n} k={k}: Table II gives T_q={t_q}, got {got} (rel {rel:.1e})"
            );
        }
    }

    /// Band geometry: the cents span rules mid/treble, the bin floor rescues
    /// the sub-bin bass, and the neighbour cap overrides both.
    #[test]
    fn coarse_band_geometry() {
        let bin_8192 = SAMPLE_RATE as f32 / BASS_WINDOW_SIZE as f32; // ≈ 5.383 Hz
        let bin_2048 = SAMPLE_RATE as f32 / WINDOW_SIZE as f32; // ≈ 21.53 Hz

        // A4: ±100 ¢ ≈ 26.2 Hz, well over the 4-bin floor and under f₀/2.
        let a4 = search_halfwidth_hz(440.0, 440.0, bin_8192);
        assert!((a4 - 440.0 * (2f32.powf(1.0 / 12.0) - 1.0)).abs() < 1e-3);

        // A0 n = 1: ±100 ¢ is 1.6 Hz — sub-bin — so the 4-bin floor takes over,
        // and the cap (f₀/2 = 13.75) then overrides that floor.
        let a0 = search_halfwidth_hz(27.5, 27.5, bin_8192);
        assert!((a0 - 13.75).abs() < 1e-4, "cap must win at A0, got {a0}");

        // A0 n = 4: the cap follows the *spacing*, not the centre. Capping at
        // centre/2 would give ±55 Hz and span two neighbouring partials.
        let a0_n4 = search_halfwidth_hz(110.0, 27.5, bin_8192);
        assert!((a0_n4 - 13.75).abs() < 1e-4);

        // At 2048 the four-bin floor is 86 Hz — wider than a bass fundamental.
        // The cap is what keeps that from returning the 2nd partial.
        assert!(COARSE_SPAN_MIN_BINS * bin_2048 > 80.0);
        assert!((search_halfwidth_hz(82.4, 82.4, bin_2048) - 41.2).abs() < 1e-3);
    }

    /// One Hann-windowed sine in AWGN, read at 8192. The gate must admit it and
    /// `jacobsen` must land within a small fraction of a bin — the end-to-end
    /// contract the readout depends on.
    #[test]
    fn coarse_read_finds_a_tone_off_reference() {
        let fs = SAMPLE_RATE;
        let f_true = 223.7; // ≈ 41.6 bins at 8192: deliberately off-centre
        let f_ref = 220.0; // reference ≈ 29 ¢ below the string
        let (mag, spec) = spectrum_of(&sine(f_true, 0.15, BASS_WINDOW_SIZE), BASS_WINDOW_SIZE);

        let mut scratch = vec![0.0f32; mag.len()];
        let hz = coarse_read(
            &mag,
            &spec,
            BASS_WINDOW_SIZE,
            fs,
            f_ref,
            f_ref,
            &mut scratch,
        )
        .expect("a clean tone within ±100 ¢ must be admitted");
        assert!(
            (hz - f_true).abs() < 0.6,
            "expected ≈{f_true} Hz, got {hz} Hz"
        );
    }

    /// Noise alone must be rejected. The gate's whole purpose: the ambient
    /// Neyman–Pearson threshold admits 100 % of this during a sustain, because
    /// its null is a quiet room rather than the local spectrum.
    #[test]
    fn coarse_read_rejects_noise() {
        let mut noise = Vec::with_capacity(BASS_WINDOW_SIZE);
        let mut x = 0x1234_5678u32;
        for _ in 0..BASS_WINDOW_SIZE {
            // xorshift → uniform in [−0.05, 0.05); no rand dependency.
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            noise.push((x as f32 / u32::MAX as f32 - 0.5) * 0.1);
        }
        let (mag, spec) = spectrum_of(&noise, BASS_WINDOW_SIZE);
        let mut scratch = vec![0.0f32; mag.len()];
        assert!(
            coarse_read(
                &mag,
                &spec,
                BASS_WINDOW_SIZE,
                SAMPLE_RATE,
                440.0,
                440.0,
                &mut scratch
            )
            .is_none(),
            "broadband noise must not produce a reading"
        );
    }

    /// The neighbour cap in force: a strong 2nd partial one spacing above the
    /// reference must not be returned in place of the (present, weaker)
    /// fundamental. Uncapped at 2048 this is the measured +1200 ¢ failure.
    #[test]
    fn coarse_read_never_returns_the_neighbour() {
        let f0 = 82.4; // guitar E2 — spacing < the 2048 four-bin floor
        let mut signal = sine(f0, 0.05, BASS_WINDOW_SIZE);
        for (i, s) in signal.iter_mut().enumerate() {
            *s +=
                0.5 * (2.0 * std::f32::consts::PI * 2.0 * f0 * i as f32 / SAMPLE_RATE as f32).sin();
        }
        let (mag, spec) = spectrum_of(&signal, WINDOW_SIZE);
        let mut scratch = vec![0.0f32; mag.len()];
        if let Some(hz) = coarse_read(&mag, &spec, WINDOW_SIZE, SAMPLE_RATE, f0, f0, &mut scratch) {
            assert!(
                hz < 1.5 * f0,
                "read must stay inside the ±spacing/2 band, got {hz} Hz"
            );
        }
    }

    /// A band the spectrum cannot hold withholds rather than guesses — the
    /// tier-1 size selection reads `None` as "this size cannot serve this
    /// register", so a fabricated number here would defeat it.
    #[test]
    fn coarse_read_withholds_without_a_band() {
        let (mag, spec) = spectrum_of(&sine(440.0, 0.2, WINDOW_SIZE), WINDOW_SIZE);
        let mut scratch = vec![0.0f32; mag.len()];
        let mut read = |center: f32, spacing: f32| {
            coarse_read(
                &mag,
                &spec,
                WINDOW_SIZE,
                SAMPLE_RATE,
                center,
                spacing,
                &mut scratch,
            )
        };
        // Above Nyquist: a high partial of a treble key has no bins at all.
        assert!(read(30_000.0, 30_000.0).is_none());
        // Hard against DC: the band clamps to a single bin.
        assert!(read(5.0, 5.0).is_none());
        // Non-physical inputs are runtime conditions, not panics.
        assert!(read(0.0, 27.5).is_none());
        assert!(read(440.0, f32::NAN).is_none());
        // A finite but absurd spacing saturates the flank's float→int cast to
        // `usize::MAX`. The flank arithmetic must clamp to the spectrum rather
        // than overflow; the read then degrades to a whole-spectrum noise
        // reference, which is sane, so the contract here is "does not panic".
        let _ = read(440.0, 1.0e30);
        let _ = read(440.0, f32::MAX);
    }

    fn sine(f: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (2.0 * std::f32::consts::PI * f * i as f32 / SAMPLE_RATE as f32).sin())
            .collect()
    }

    /// Hann-windowed magnitude + complex spectra, the same path the pipeline
    /// hands to [`coarse_read`].
    fn spectrum_of(signal: &[f32], fft_size: usize) -> (Vec<f32>, Vec<Complex<f32>>) {
        let fft = realfft::RealFftPlanner::<f32>::new().plan_fft_forward(fft_size);
        let mut time = vec![0.0f32; fft_size];
        let mut spec = vec![Complex { re: 0.0, im: 0.0 }; fft_size / 2 + 1];
        spectral::fft(
            &signal[signal.len() - fft_size..],
            &mut time,
            &mut spec,
            &fft,
            fft_size,
        );
        let mut mag = vec![0.0f32; fft_size / 2];
        spectral::magnitude_spectrum(&spec, fft_size, &mut mag);
        (mag, spec)
    }

    // ── resolve_lines ────────────────────────────────────────────────────────

    /// The hop rate the strobe's baseband is sampled at.
    const HOP_HZ: f32 = 44_100.0 / 1024.0;

    /// One string: a damped complex exponential at `offset_hz` from the
    /// reference, which is what the strobe's demodulated Goertzel produces.
    struct Source {
        offset_hz: f32,
        amplitude: f32,
        tau_secs: f32,
    }

    /// Builds a baseband record of `n` hops from the given strings plus
    /// deterministic circular noise at `noise` RMS per component.
    fn baseband(strings: &[Source], n: usize, noise: f32, seed: u32) -> Vec<Complex<f32>> {
        let mut x = seed | 1;
        let mut rand = move || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as f32 / u32::MAX as f32 - 0.5
        };
        (0..n)
            .map(|h| {
                let t = h as f32 / HOP_HZ;
                let mut z = Complex::new(noise * rand(), noise * rand());
                for (k, s) in strings.iter().enumerate() {
                    // Distinct start phases: two strings struck by one hammer do
                    // not start in phase, and an in-phase pair is the easy case.
                    let phase = 2.0 * std::f32::consts::PI * (s.offset_hz * t + 0.17 * k as f32);
                    let decay = (-t / s.tau_secs).exp();
                    z += Complex::new(phase.cos(), phase.sin()) * (s.amplitude * decay);
                }
                z
            })
            .collect()
    }

    /// Runs the shipped estimator over a record, planning its transform the way
    /// the component does at startup.
    fn resolve(record: &[Complex<f32>]) -> Vec<UnisonLine> {
        let n = record.len();
        let fft = rustfft::FftPlanner::<f32>::new().plan_fft_forward(n);
        let mut spectrum = vec![Complex::new(0.0, 0.0); n];
        let mut magnitudes = vec![0.0f32; n];
        let mut fft_scratch = vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len()];
        let mut out = [UnisonLine::default(); MAX_UNISON_LINES];
        let count = resolve_lines(
            record,
            fft.as_ref(),
            spectral::candan_c_n(n),
            HOP_HZ,
            &mut LineScratch {
                spectrum: &mut spectrum,
                magnitudes: &mut magnitudes,
                fft: &mut fft_scratch,
            },
            &mut out,
        );
        out[..count].to_vec()
    }

    /// [`UNISON_MIN_BINS`] is Rohling §V solved for the record length, so pin the
    /// solution: at that length the reference window still holds enough cells for
    /// the other two strings to be tolerable inhomogeneities, and one bin shorter
    /// it does not.
    #[test]
    fn unison_min_record_satisfies_rohlings_interference_criterion() {
        let lobe = 2 * UNISON_CFAR_GUARD_BINS + 1;
        let interferers = MAX_UNISON_LINES - 1;
        assert_eq!(UNISON_MIN_REFS, 20, "2 interferers × 5 lobe cells at q = ½");
        assert_eq!(UNISON_MIN_BINS, 25);

        // k/N ≤ 1 − (occupied / N_ref) is the criterion; check it holds at the
        // floor and fails one cell below it.
        let cells = |n: usize| (2 * UNISON_CFAR_WINDOW_BINS).min(n - lobe);
        let ok =
            |n: usize| UNISON_CFAR_QUANTILE <= 1.0 - (interferers * lobe) as f32 / cells(n) as f32;
        assert!(ok(UNISON_MIN_BINS));
        assert!(!ok(UNISON_MIN_BINS - 1));
    }

    /// Two strings 2.0 Hz apart over the 56-hop record must resolve as two lines
    /// at the right places. This is the feature: a tuner watching two markers
    /// converge.
    #[test]
    fn resolve_lines_finds_two_strings() {
        let lines = resolve(&baseband(
            &[
                Source {
                    offset_hz: -1.0,
                    amplitude: 1.0,
                    tau_secs: 1.5,
                },
                Source {
                    offset_hz: 1.0,
                    amplitude: 1.0,
                    tau_secs: 1.5,
                },
            ],
            56,
            0.01,
            0x1234_5678,
        ));
        assert_eq!(lines.len(), 2, "a 2 Hz split at 1.3 s must resolve");

        let mut found: Vec<f32> = lines.iter().map(|l| l.offset_hz).collect();
        found.sort_by(f32::total_cmp);
        for (got, want) in found.iter().zip([-1.0f32, 1.0]) {
            assert!(
                (got - want).abs() < 0.15,
                "line at {got} Hz, expected {want} Hz"
            );
        }
        assert_eq!(lines[0].relative_amplitude, 1.0);
        assert!(lines[1].relative_amplitude > 0.5);
    }

    /// **The null.** One string must report one line — no matter how clean, how
    /// fast it decays, or how long the record. A false second line here is a
    /// tuner chasing a beat that does not exist, and the flanking reference
    /// geometry `coarse_read` uses produced them at up to 26.7 % (ADR 0012 §2).
    #[test]
    fn resolve_lines_reports_one_line_for_one_string() {
        for (n, tau, noise, seed) in [
            (30usize, 1.5f32, 0.03f32, 0x9e37_79b9u32),
            (56, 1.5, 0.03, 0x85eb_ca6b),
            (56, 0.4, 0.03, 0xc2b2_ae35),
            (56, 1.5, 0.18, 0x27d4_eb2f), // SNR ≈ 15 dB
            (40, 0.4, 0.18, 0x1656_67b1),
        ] {
            let lines = resolve(&baseband(
                &[Source {
                    offset_hz: 0.7,
                    amplitude: 1.0,
                    tau_secs: tau,
                }],
                n,
                noise,
                seed,
            ));
            assert_eq!(
                lines.len(),
                1,
                "n={n} τ={tau} noise={noise}: one string must give one line, got {lines:?}"
            );
        }
    }

    /// Below the Rayleigh criterion two components *are* one line, and saying so
    /// is the honest answer — the caller publishes `2/T` beside it so the display
    /// can say how much "one line" is worth.
    #[test]
    fn resolve_lines_merges_inside_the_rayleigh_criterion() {
        let lines = resolve(&baseband(
            &[
                Source {
                    offset_hz: -0.2,
                    amplitude: 1.0,
                    tau_secs: 1.5,
                },
                Source {
                    offset_hz: 0.2,
                    amplitude: 1.0,
                    tau_secs: 1.5,
                },
            ],
            56,
            0.01,
            0x3c6e_f372,
        ));
        assert_eq!(lines.len(), 1, "0.4 Hz is under 2/T = 1.54 Hz");
    }

    /// A record too short for the gate to stand behind reports nothing rather
    /// than something. There is no ceiling to match it: a record longer than
    /// anything the ring currently keeps must still resolve, because how long to
    /// keep is the caller's policy and not this function's.
    #[test]
    fn resolve_lines_has_a_floor_and_no_ceiling() {
        // 5 Hz apart, so the pair clears 2/T at the floor (3.45 Hz) as well as
        // at the long record — the test is about length limits, not resolution.
        let two = [
            Source {
                offset_hz: -2.5,
                amplitude: 1.0,
                tau_secs: 1.5,
            },
            Source {
                offset_hz: 2.5,
                amplitude: 1.0,
                tau_secs: 1.5,
            },
        ];
        assert!(resolve(&baseband(&two, UNISON_MIN_BINS - 1, 0.01, 7)).is_empty());
        assert_eq!(resolve(&baseband(&two, UNISON_MIN_BINS, 0.01, 7)).len(), 2);
        // Well past the shipped ring cap, and past the 64 a bitmask scan allowed.
        assert_eq!(resolve(&baseband(&two, 100, 0.01, 7)).len(), 2);

        // No room to write an answer into ⇒ no work.
        let record = baseband(&two, 56, 0.01, 7);
        let fft = rustfft::FftPlanner::<f32>::new().plan_fft_forward(record.len());
        let mut spectrum = vec![Complex::new(0.0, 0.0); record.len()];
        let mut magnitudes = vec![0.0f32; record.len()];
        let mut scratch = vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len()];
        assert_eq!(
            resolve_lines(
                &record,
                fft.as_ref(),
                spectral::candan_c_n(record.len()),
                HOP_HZ,
                &mut LineScratch {
                    spectrum: &mut spectrum,
                    magnitudes: &mut magnitudes,
                    fft: &mut scratch,
                },
                &mut [],
            ),
            0
        );
    }

    /// A silent baseband has no lines, and a lopsided pair still finds the
    /// quiet string: sensitivity reaches ≈26 dB below the strongest (ADR 0012 §3).
    #[test]
    fn resolve_lines_handles_the_extremes() {
        let silence = vec![Complex::new(0.0f32, 0.0); 56];
        assert!(resolve(&silence).is_empty());

        let lines = resolve(&baseband(
            &[
                Source {
                    offset_hz: -1.5,
                    amplitude: 1.0,
                    tau_secs: 1.5,
                },
                Source {
                    offset_hz: 1.5,
                    amplitude: 0.1, // −20 dB
                    tau_secs: 1.5,
                },
            ],
            56,
            0.002,
            0x2545_f491,
        ));
        assert_eq!(lines.len(), 2, "a −20 dB second string must still be found");
        assert!(lines[1].relative_amplitude < 0.3);
    }
}
