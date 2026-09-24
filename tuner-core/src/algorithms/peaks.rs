//! # Spectral peaks
//!
//! Local maxima of a spectrum, refined sub-bin: a frame's global peak list
//! ([`extract_peaks`], thinned by [`mask_peaks`]), one partial read in a band
//! around a known reference ([`coarse_read`]), and the lines within one partial's
//! baseband ([`resolve_lines`]). The last two admit a peak by an ordered-statistic
//! CFAR gate.

use rustfft::Fft;
use rustfft::num_complex::Complex;

use crate::algorithms::spectral;
use crate::models::{SpectralPeak, UnisonLine};

/// Every local maximum of `magnitudes` above `min_magnitude`, refined sub-bin by
/// [`spectral::jacobsen`] on `complex_spectrum` and written to `peaks_out` strongest
/// first. Returns how many were written. A non-positive `min_magnitude` writes
/// nothing, and only the first 128 maxima in bin order are kept.
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

    if min_magnitude <= 0.0 {
        return 0;
    }

    let mut temp_peaks = [SpectralPeak::default(); 128];
    let mut num_found = 0;

    for i in 1..(magnitudes.len() - 1) {
        let mag = magnitudes[i];

        if mag > min_magnitude && mag > magnitudes[i - 1] && mag > magnitudes[i + 1] {
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
    valid_peaks.sort_unstable_by(|a, b| {
        b.magnitude
            .partial_cmp(&a.magnitude)
            .unwrap_or(core::cmp::Ordering::Equal)
    });

    let count = num_found.min(peaks_out.len());
    peaks_out[..count].copy_from_slice(&valid_peaks[..count]);

    count
}

/// Removes side lobes, sympathetic resonance and intermodulation products from a
/// peak list: every peak more than 30 dB below the strongest, and every peak within
/// ±20 % of a stronger one's frequency and more than 30 dB below it. Only the first
/// 64 entries are considered. Returns how many survive, written to `peaks[..count]`
/// by ascending frequency.
///
/// # Panics
/// On a NaN magnitude or frequency.
///
/// # References
/// Cano, P. (1998). "Fundamental Frequency Estimation in the SMS Analysis."
/// DAFx-98, §4.3, which accepts a peak "less than 40 dB below the highest peak";
/// the global gate is that rule at 30 dB.
// Ours: the masking, and 30 dB where Cano has 40, validated at 8/8 keys with zero
// false locks. Neither Cano nor Gómez (2006) has a masking procedure, so neither is
// its citation; the 20 % band borrows the critical-band approximation CB ≈ 0.2·f
// above ≈ 500 Hz. Below ≈ 30 dB SNR, sympathetic noise survives the mask.
// report 0002
// audit 04
pub fn mask_peaks(peaks: &mut [SpectralPeak]) -> usize {
    if peaks.is_empty() {
        return 0;
    }

    let k = peaks.len().min(64);
    let active_peaks = &mut peaks[..k];

    active_peaks.sort_unstable_by(|a, b| b.magnitude.partial_cmp(&a.magnitude).unwrap());

    let mut valid_count = 0;
    let mut masked = [false; 64];
    let global_max = active_peaks[0].magnitude;

    // Amplitude ratios: 0.0316 is −30 dB.
    const GLOBAL_THRESHOLD_RATIO: f32 = 0.0316;
    const MASK_THRESHOLD_RATIO: f32 = 0.0316;
    const MASK_BANDWIDTH_PROPORTION: f32 = 0.20;

    for i in 0..k {
        if masked[i] {
            continue;
        }

        if active_peaks[i].magnitude < global_max * GLOBAL_THRESHOLD_RATIO {
            continue;
        }

        let masker_freq = active_peaks[i].frequency;
        let masker_mag = active_peaks[i].magnitude;

        let mask_threshold = masker_mag * MASK_THRESHOLD_RATIO;
        let mask_bw = masker_freq * MASK_BANDWIDTH_PROPORTION;

        for j in (i + 1)..k {
            if !masked[j] {
                let target_freq = active_peaks[j].frequency;
                let target_mag = active_peaks[j].magnitude;

                if (target_freq - masker_freq).abs() < mask_bw && target_mag < mask_threshold {
                    masked[j] = true;
                }
            }
        }

        active_peaks[valid_count] = active_peaks[i];
        valid_count += 1;
    }

    active_peaks[..valid_count]
        .sort_unstable_by(|a, b| a.frequency.partial_cmp(&b.frequency).unwrap());

    valid_count
}

// ─── Coarse Readout: bounded search + OS-CFAR ────────────────────────────────

/// Search half-width in cents, so one value serves every register. A partial
/// detuned further cannot be read.
// Ours, measured: availability holds at 100 % out to 75 ¢ of detuning, then ends
// at this edge. Widening buys reach in a pitch raise; narrowing buys nothing, since
// the neighbour cap in `search_halfwidth_hz` prevents mis-selection and a second
// sounding key is not narrowed away.
// report 0011
const COARSE_SPAN_CENTS: f32 = 100.0;

/// Floor on the search half-width in bins, for registers where
/// [`COARSE_SPAN_CENTS`] is sub-bin (±1.6 Hz at A0).
// Ours, measured; 3 vs 4 bins is inert on keys 8–26 (availability 92.1 % both,
// |e| 23.67 vs 23.68 ¢, jitter 3.32 vs 3.28 ¢). At 8192 it sets the band from the
// deep bass, where the neighbour cap binds, to ≈ key 44, where the cents span
// overtakes it. Never without the cap: at 2048 it is an 86 Hz half-width, and the
// read returns the 2nd partial (+1200 ¢).
// report 0011
const COARSE_SPAN_MIN_BINS: f32 = 4.0;

/// Rank of the order statistic taken as the local noise, as a fraction of the
/// reference count: Rohling's `k/N`. His §V criterion sets it: an inhomogeneity in
/// the reference window is tolerable only while it "affects less than (N − k)
/// resolution cells". Here the harmonic comb is that inhomogeneity, so with partial
/// spacing `s` bins and a Hann main lobe `W_lobe = 4` bins wide null-to-null,
///
/// ```text
///   (W_lobe / s)·N  ≤  N − k      ⇒      k/N  ≤  1 − W_lobe / s
/// ```
///
/// That binds below the paper's own `k > N/2`: a radar reference window is mostly
/// clutter, this one mostly partials. The cost is the one Rohling names for
/// `k < N/2`, erosion: under-estimation at an edge.
// A0 binds, with no margin: s = 5.11 bins and 75 % of reference cells in a lobe
// give k/N ≤ 0.25, and the bound relaxes upward (0.53 at F1). Do not raise it: at
// the median the bound fails for every key up to F1, and the deep bass admits
// ±400 ¢ junk.
// report 0011
// audit 13
const COARSE_CFAR_QUANTILE: f32 = 0.25;

/// Reference flank width either side of the search band, in partial spacings.
// Ours, measured.
// report 0011
const COARSE_CFAR_FLANK_SPACINGS: f32 = 1.5;

/// Floor on the flank width, in Hz: in bins it would change width with the FFT
/// size.
// Ours, measured. Where partials sit ≈ 5 bins apart, 75 % of cells lie in a lobe,
// so 1.5 spacings would sample only the strong low partials and read signal as
// noise; 172 Hz reaches the weak upper ones (≈ 1–11 at A0), whose skirts the low
// quantile lands on. Inert where the spacing passes 115 Hz (≈ key 25). Not in
// spacings either: that floor never turns off (2.8 kHz at A4).
// report 0011
// audit 13
const COARSE_CFAR_FLANK_MIN_HZ: f32 = 172.0;

/// False-alarm probability both CFAR gates are calibrated to: the 0.001
/// [`spectral::neyman_pearson_k`] uses, so the gates differ only in the noise they
/// measure.
const CFAR_P_FA: f32 = 0.001;

/// Fewest reference cells for a noise estimate. With fewer, as where the flanks
/// meet DC or Nyquist, the read is withheld.
// A refusal floor, not an operating point: the order statistic needs 2, and at 4
// the multiplier is already ≈ 45, some 6× the working threshold. A read normally
// has 53–57.
// audit 13
const COARSE_CFAR_MIN_REFS: usize = 4;

/// Search half-width in Hz: [`COARSE_SPAN_CENTS`], floored at
/// [`COARSE_SPAN_MIN_BINS`], then capped at half the partial spacing, even below
/// one bin.
///
/// `spacing_hz` (≈ f₀) is not `center_hz`; they coincide only at n = 1. A0's 4th
/// partial has `center_hz ≈ 110` but `spacing_hz ≈ 27.5`, and a cap at
/// `center_hz / 2` would admit a ±55 Hz band spanning two neighbours.
fn search_halfwidth_hz(center_hz: f32, spacing_hz: f32, hz_per_bin: f32) -> f32 {
    let span = center_hz * (2f32.powf(COARSE_SPAN_CENTS / 1200.0) - 1.0);
    span.max(COARSE_SPAN_MIN_BINS * hz_per_bin)
        .min(spacing_hz / 2.0)
}

/// Exact finite-`N` OS-CFAR threshold multiplier, a port of Rohling (1983)
/// Eqs. 14 + 17.
///
/// Eq. 14 gives the false-alarm probability of an ordered-statistic CFAR detector
/// with `N = n_ref` reference cells selecting rank `k`, for exponentially
/// distributed (square-law) cells:
///
/// ```text
///   P_fa = k·C(N,k)·Γ(k)·Γ(T+N−k+1) / Γ(T+N+1)
/// ```
///
/// For integer `k` the gamma ratio telescopes to `1/∏_{j=0}^{k−1}(T+N−j)` and the
/// prefactor reduces to `N!/(N−k)!`, leaving
///
/// ```text
///   P_fa = ∏_{j=0}^{k−1} (N−j)/(T+N−j)
/// ```
///
/// which is exact and strictly decreasing in `T`, so `T` follows by bisection.
///
/// Our cells are Rayleigh magnitudes, not exponential powers. For a receiver taking
/// the absolute value, Eq. 17 converts the square-law factor to `T_lin = √T_q`
/// (`T_q` is Table II's factor, not a quantile), and Rohling scopes the conversion
/// to this detector: "this simple conversion, however, does not apply for CA or
/// CAGO CFAR".
///
/// As `N → ∞` with `k = q·N` the result tends to `√(ln P_fa / ln(1−q))`, 3.157 at
/// the median for P_fa = 0.001.
///
/// Returns infinity for an unusable rank, so a gate that cannot form an order
/// statistic admits nothing.
///
/// # References
/// Rohling, H. (1983). "Radar CFAR Thresholding in Clutter and Multiple Target
/// Situations." IEEE Trans. Aerospace and Electronic Systems, AES-19(4),
/// pp. 608–621. DOI: 10.1109/TAES.1983.309350. (Eqs. 9–10, 12, 14, 17.)
/// Lineage: Finn, H. M. & Johnson, R. S. (1968). "Adaptive Detection Mode with
/// Threshold Control as a Function of Spatially Sampled Clutter-Level
/// Estimates." RCA Review 29(3), pp. 414–464 — the cell-averaging predecessor.
// asserted: coarse_cfar_multiplier_table_ii, coarse_cfar_multiplier_pinned
fn cfar_multiplier(n_ref: usize, k: usize, p_fa: f32) -> f32 {
    if n_ref == 0 || k == 0 || k > n_ref {
        return f32::INFINITY;
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
    // The bracket caps T_q at 1e6 (T_lin at 1000); 60 halvings resolve it far below
    // f32 precision.
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

/// The coarse readout: the strongest bin in a bounded band around one reference
/// partial, admitted by an ordered-statistic CFAR gate and refined sub-bin by
/// [`spectral::jacobsen`]. Returns the partial's frequency in Hz, or `None` when
/// nothing in the band clears the local noise.
///
/// The gate (Rohling 1983; Finn & Johnson 1968 lineage) sets its threshold from
/// reference cells in flanks either side of the band, so the null is the spectrum
/// around the sounding note rather than a quiet room. The argmax gives every cell
/// in the band a chance to false-alarm, so each gets `P_fa / M`, `M` the band's
/// bins halved for Hann correlation.
///
/// `magnitudes` (`fft_size / 2` bins) and `complex_spectrum` are one frame, and the
/// band centres on `center_hz`. `spacing_hz`, the key's partial spacing (≈ f₀), sets
/// the neighbour cap and the flank width. `scratch` holds the reference cells and
/// needs at least `magnitudes.len()` elements.
///
/// # Panics
/// In debug builds, if `scratch` is shorter than `magnitudes`. In release the
/// reference set is truncated, which biases the order statistic.
// Not a lookup in `extract_peaks`' list: `Engine` builds that list only while
// discovering, and its absolute threshold and `mask_peaks` can drop the weak
// partial being tuned.
// Ours: the flanks and the `P_fa / M` budget. Without the budget the realized
// false-alarm rate was 39× nominal.
// report 0011
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
    let flank_hz = (COARSE_CFAR_FLANK_SPACINGS * spacing_hz).max(COARSE_CFAR_FLANK_MIN_HZ);
    // A huge `spacing_hz` saturates this cast to `usize::MAX`, so the bounds
    // saturate too.
    let flank = (flank_hz / hz_per_bin).ceil() as usize;
    let outer_lo = lo.saturating_sub(flank).max(1);
    let outer_hi = hi.saturating_add(flank).min(n_bins - 2);

    // No guard cells: "in OS CFAR processing these guard cells become unnecessary"
    // (Rohling §V). The only lobe cells that can enter lie just past a band edge,
    // and they sort above a low quantile; a 0–4-bin guard measured inert.
    // audit 13
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
    // both the effective reference count and the band's independent cells, and the
    // argmax divides the P_fa budget over the latter. Hann's ENBW is 1.5 bins, not
    // 2, so each alone is conservative, but together they land on nominal (realized
    // 0.00097 against 0.001). Do not change one alone.
    // report 0011
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

/// Most lines one reference reports, the strongest admitted: a piano unison has at
/// most three strings.
pub const MAX_UNISON_LINES: usize = 3;

/// Guard cells either side of the cell under test, in bins: the cell's own Hann
/// main lobe, whose skirt is not noise.
// Not `coarse_read`'s flanks outside the band: they exclude the dominant line's
// skirt, so a secondary maximum riding it is admitted, up to 26.7 % false second
// lines on one synthetic string.
// report 0012
const UNISON_CFAR_GUARD_BINS: usize = 2;

/// Reference cells either side, beyond the guard: 32 in all, within Rohling's
/// `N = 24 … 32 and more`. The window slides with the cell under test, and a record
/// too short to fill it uses every bin outside the guard.
const UNISON_CFAR_WINDOW_BINS: usize = 16;

/// Rank of the order statistic taken as the local noise, as a fraction of the
/// reference count: Rohling's `k/N`, at the median. His §V criterion holds with
/// margin, the note's other two strings occupying `2 × (2·GUARD + 1) = 10` cells
/// against the `N_ref − k = 16` it allows.
// Do not raise it to Rohling's worked q = 0.75: with three lines present the upper
// quantile is signal, and a three-string unison is lost (detection 100 % → 0 %).
// report 0012
const UNISON_CFAR_QUANTILE: f32 = 0.50;

/// Rayleigh criterion in bins: two lines closer than the Hann main-lobe half-width
/// (`2/T` Hz) are one line.
const UNISON_MERGE_BINS: f32 = 2.0;

/// Reference cells Rohling's §V criterion demands: the note's other strings,
/// [`MAX_UNISON_LINES`] − 1 main lobes of `2·GUARD + 1` cells, must affect fewer
/// than `N − k` of them.
const UNISON_MIN_REFS: usize = (((MAX_UNISON_LINES - 1) * (2 * UNISON_CFAR_GUARD_BINS + 1)) as f32
    / (1.0 - UNISON_CFAR_QUANTILE)) as usize;

/// Shortest record [`resolve_lines`] resolves: [`UNISON_MIN_REFS`] reference cells
/// besides the cell under test and its guard. Below it a second string makes the
/// window signal-dominated, so one line would mean a blind detector, not one string.
pub(crate) const UNISON_MIN_BINS: usize = UNISON_MIN_REFS + 2 * UNISON_CFAR_GUARD_BINS + 1;

/// Working buffers for [`resolve_lines`], so a hop allocates nothing.
/// [`Self::spectrum`] and [`Self::magnitudes`] hold at least the transform length,
/// [`Self::fft`] at least the plan's `get_inplace_scratch_len()`.
pub struct LineScratch<'a> {
    /// Windowed baseband, transformed in place.
    pub spectrum: &'a mut [Complex<f32>],
    /// `|Z[m]|`, the magnitudes of `spectrum`.
    pub magnitudes: &'a mut [f32],
    /// `rustfft`'s in-place scratch.
    pub fft: &'a mut [Complex<f32>],
}

/// Resolves the strings of one reference partial as separate spectral lines, each
/// a signed Hz offset from the reference. Returns how many were written to `out`,
/// strongest first and at most [`MAX_UNISON_LINES`], with `relative_amplitude`
/// normalised to the strongest.
///
/// A zoom FFT (Lyons 2010, ch. 13) whose mixing, anti-alias filtering and
/// decimation are done: `baseband` holds one Hann-windowed Goertzel output per hop
/// at the reference, oldest first, sampled at `hop_rate_hz` and so unambiguous over
/// ±`hop_rate_hz / 2`. Resolution is set by the record's duration `T`, not its bin
/// count: a pair resolves once its separation clears `2/T`.
///
/// The record is Hann-windowed and transformed at its own length `N`, unpadded:
/// padded bins are interpolated, and the CFAR null assumes independent bins. Local
/// maxima of the circular spectrum are taken strongest first, refined by Candan's
/// Eq. 1 evaluated circularly, dropped within `UNISON_MERGE_BINS` of a stronger
/// line, and admitted by an ordered-statistic CFAR gate against a window sliding
/// with each; the first rejection ends the list.
///
/// `fft` is a forward plan for exactly `N`, and `c_n` is [`spectral::candan_c_n`]
/// at `N`, passed in because it costs `O(N)` in trigonometry; the 2.0 asymptote
/// would scale every offset by 2.4 % at `N = 56`. A record shorter than
/// `UNISON_MIN_BINS` reports nothing, and there is no upper bound.
///
/// # Panics
/// In debug builds, if `fft`'s length disagrees with `baseband`'s or a scratch
/// buffer is too short. In release those return `0`.
///
/// # References
/// Rohling, H. (1983). "Radar CFAR Thresholding in Clutter and Multiple Target
/// Situations." IEEE Trans. AES-19(4) — the admission gate; §V sets the rank.
/// Candan, Ç. (2015). Signal Processing 114, Eq. 1 — the sub-bin refinement.
/// Lyons, R. (2010). *Understanding DSP*, ch. 13 — the zoom-FFT structure.
// asserted: tests/unison_resolution.rs
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
    // Symmetric Hann, over [0, N−1]: the window `c_n` is derived for.
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
    // The last candidate examined. A cursor through a strict total order skips what
    // was already examined without a visited set, so nothing bounds the record.
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

/// Strict total order on candidates: descending magnitude, then ascending bin.
/// Ties must order too, or the cursor scan would skip a candidate as strong as the
/// last one examined.
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

    // `coarse_read`'s three calibrated factors of two, with the whole record as the
    // searched band.
    // report 0012
    let m_eff = (n / 2).max(1) as f32;
    let threshold =
        noise * cfar_multiplier((n_ref / 2).max(2), (rank / 2).max(1), CFAR_P_FA / m_eff);
    threshold.is_finite() && threshold > 0.0 && mag >= threshold
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{BASS_WINDOW_SIZE, HOP_RATE_HZ, SAMPLE_RATE, WINDOW_SIZE};

    /// Pins the calibration: Rohling Eqs. 14 + 17 at the median rank against values
    /// computed independently from the closed form, and the asymptotic limit
    /// `T_lin → √(ln P_fa / ln(1−q))` = 3.157 at the median for P_fa = 0.001.
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
        // The finite-N form converges on the quantile form.
        let limit = cfar_multiplier(100_000, 50_000, 0.001);
        assert!(
            (limit - 3.1569).abs() < 5e-3,
            "finite-N must converge to the quantile form, got {limit}"
        );
        // A tighter budget demands a higher threshold, which the search-loss
        // correction relies on.
        assert!(cfar_multiplier(64, 16, 1e-5) > cfar_multiplier(64, 16, 1e-3));
        // Unusable rank admits nothing rather than defaulting to something.
        assert!(cfar_multiplier(0, 1, 0.001).is_infinite());
        assert!(cfar_multiplier(8, 9, 0.001).is_infinite());
    }

    /// Reproduces Rohling Table II: the square-law factor `T_q` at `P_fa = 10⁻⁶` for
    /// `N ∈ {8, 16, 24, 32}` is this function's output squared (Eq. 17 inverted).
    /// That checks Eq. 14 against the paper's numbers, not our derivation of it.
    ///
    /// `k = 1` is excluded: its `T_q = N/P_fa − N` reaches 3.2 × 10⁷, past the
    /// bisection's `1e6` bracket. The widest band in use keeps `T_q` under 2.5 × 10⁴.
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

        // A0 n = 4: the cap follows the spacing, not the centre. Capping at
        // centre/2 would give ±55 Hz and span two neighbouring partials.
        let a0_n4 = search_halfwidth_hz(110.0, 27.5, bin_8192);
        assert!((a0_n4 - 13.75).abs() < 1e-4);

        // At 2048 the four-bin floor is 86 Hz — wider than a bass fundamental.
        // The cap is what keeps that from returning the 2nd partial.
        assert!(COARSE_SPAN_MIN_BINS * bin_2048 > 80.0);
        assert!((search_halfwidth_hz(82.4, 82.4, bin_2048) - 41.2).abs() < 1e-3);
    }

    /// A clean sine ≈ 29 ¢ off the reference, read at 8192, is admitted and lands
    /// within 0.6 Hz.
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

    /// Broadband noise alone yields no reading.
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

    /// A strong 2nd partial is not returned in place of the weaker fundamental:
    /// without the neighbour cap, the read at 2048 returns it (+1200 ¢).
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

    /// A band the spectrum cannot hold, or a non-physical input, withholds the read.
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
        // An absurd spacing saturates the flank's cast to `usize::MAX`; the flank
        // must clamp rather than overflow. Only not panicking is asserted.
        let _ = read(440.0, 1.0e30);
        let _ = read(440.0, f32::MAX);
    }

    fn sine(f: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (2.0 * std::f32::consts::PI * f * i as f32 / SAMPLE_RATE as f32).sin())
            .collect()
    }

    /// Hann-windowed magnitude and complex spectra of the freshest `fft_size`
    /// samples.
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

    /// One string: a damped complex exponential at `offset_hz` from the
    /// reference, which is what the strobe's demodulated Goertzel produces.
    struct Source {
        offset_hz: f32,
        amplitude: f32,
        tau_secs: f32,
    }

    /// Builds a baseband record of `n` hops from the given strings, plus
    /// deterministic uniform noise spanning `noise` in each component.
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
                let t = h as f32 / HOP_RATE_HZ;
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

    /// Runs [`resolve_lines`] over a record, with a transform planned for its length.
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
            HOP_RATE_HZ,
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

    /// Two strings 2.0 Hz apart over a 56-hop record resolve as two lines, each in
    /// place.
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

    /// The null: one string reports one line, whatever its noise, decay or record
    /// length.
    // report 0012
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

    /// Two components inside the Rayleigh criterion report one line.
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

    /// A record shorter than [`UNISON_MIN_BINS`] reports nothing, and there is no
    /// matching ceiling: how long a record to keep is the caller's policy.
    #[test]
    fn resolve_lines_has_a_floor_and_no_ceiling() {
        // 5 Hz apart, clearing 2/T at the floor (3.45 Hz) too: this tests length
        // limits, not resolution.
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
        // Past the ring's cap, and past 64 bins, which a bitmask scan cannot reach.
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
                HOP_RATE_HZ,
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

    /// A silent baseband has no lines, and a second string 20 dB down is still
    /// found.
    // report 0012
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
