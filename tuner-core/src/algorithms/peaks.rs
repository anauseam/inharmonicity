//! # Spectral Peak Extraction
//!
//! Stateless DSP module for extracting sub-bin accurate spectral peaks
//! from magnitude spectra. Two consumers, both scan-then-refine:
//!
//! - [`extract_peaks`] — Discovery's *global* peak list (every local maximum
//!   above an absolute threshold), fed to TWM after [`mask_peaks`].
//! - [`coarse_read`] — the tuning readout's *bounded* single-partial search
//!   around a known reference, admitted by an ordered-statistic CFAR gate.
//!
//! Do not fold the second onto the first: the global list is built only in the
//! discovery branch (`identified_key.is_none()`), so it is unavailable exactly
//! while a locked note is being tuned, and its Neyman–Pearson gate and masking
//! can drop a weak target partial the readout still needs.

use rustfft::num_complex::Complex;

use crate::algorithms::spectral;
use crate::models::SpectralPeak;

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

/// False-alarm probability the gate is calibrated to — the same 0.001
/// [`spectral::neyman_pearson_k`] commits to, so the two gates differ only in
/// *which* noise they measure, never in how permissive they are.
const COARSE_P_FA: f32 = 0.001;

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
        noise * cfar_multiplier((n_ref / 2).max(2), (rank / 2).max(1), COARSE_P_FA / m_eff);
    if !(threshold.is_finite() && threshold > 0.0 && best_mag >= threshold) {
        return None;
    }

    let f = spectral::jacobsen(complex_spectrum, best, fft_size, sample_rate);
    (f.is_finite() && f > 0.0).then_some(f)
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
}
