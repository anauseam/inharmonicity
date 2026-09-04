//! # The coarse read's admission gate
//!
//! The bounded single-partial search publishes a reading only where the peak
//! clears a threshold. This module measures that threshold four ways against
//! [`crate::truth`]: the per-key × per-partial profile it admits, its realized
//! false-alarm rate on signal-free input, the anatomy of the reference set it
//! thresholds against (Rohling §V), and a head-to-head between the shipped
//! ambient-σ gate and the OS-CFAR variants that could replace it.
//!
//! Reproduces faithfulness audit 13.

use std::path::{Path, PathBuf};

use realfft::RealFftPlanner;
use rustfft::num_complex::Complex;

use crate::truth::*;
use tuner_core::algorithms::curves;
use tuner_core::algorithms::peaks;
use tuner_core::algorithms::spectral::{fft, magnitude_spectrum};
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE, WINDOW_SIZE};
use tuner_core::gatekeeper::{Gatekeeper, SignalState};
use tuner_core::models::{NOTES, get_expected_beta};
use tuner_core::pipeline::ProcessingFrame;
use tuner_core::strobe::MAX_STROBE_REFS;

/// **T1 — the per-key × per-partial CFAR profile.** The objective input to the
/// coarse-`n*` decision, and the closure of the coverage gap that let round 1
/// crown n = 3 from an *ambient*-gate aggregate while the real gate behaves
/// entirely differently (A0: n = 5 perfect, n = 6 starved).
///
/// One row per (key, partial), aggregated over that key's repeat captures:
/// availability, |median − partial-truth|, jitter, and median CFAR margin (the
/// headroom over threshold — a partial at margin 0.9 is one strike-strength
/// away from admission, which a bare availability figure hides).
fn cfar_profile(
    caps: &[PathBuf],
    planner: &mut RealFftPlanner<f32>,
    span_cents: f32,
    min_bins: f32,
    fft_size: usize,
    max_n: usize,
) {
    let gate = shipping_gate();
    // key → per-capture rows, each row one (avail, |err|, jitter, margin) per partial
    type PartialRow = Vec<(f32, f32, f32, f32)>;
    let mut by_key: std::collections::BTreeMap<u8, Vec<PartialRow>> =
        std::collections::BTreeMap::new();

    for dir in caps {
        let Some(key) = dir
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(crate::capture::key_from_dirname)
        else {
            continue;
        };
        let Some(signal) = crate::raw::read(&dir.join("audio.raw")) else {
            continue;
        };
        if signal.len() < BASS_WINDOW_SIZE {
            continue;
        }
        let f_et = NOTES[key as usize].frequency;
        let b = get_expected_beta(key);
        let mut refs = [0.0f32; MAX_STROBE_REFS];
        let count = strobe_refs(f_et, b, max_n, &mut refs);
        let series = multi_partial_series(
            &signal, &refs, count, f_et, fft_size, 0.001, span_cents, min_bins, gate, planner,
        );

        let mut per_partial = Vec::new();
        for n in 0..count {
            let truth = dtft_truth(&signal, refs[n], planner).map(|f| cents(f, refs[n]));
            let hits: Vec<f32> = series
                .iter()
                .filter_map(|hop| hop[n].0)
                .map(|f| cents(f, refs[n]))
                .filter(|c| c.is_finite())
                .collect();
            let mut margins: Vec<f32> = series.iter().map(|hop| hop[n].1).collect();
            margins.sort_by(f32::total_cmp);
            let med_margin = margins.get(margins.len() / 2).copied().unwrap_or(0.0);
            let avail = hits.len() as f32 / series.len().max(1) as f32;
            if hits.is_empty() {
                per_partial.push((avail, f32::NAN, f32::NAN, med_margin));
                continue;
            }
            let mut sorted = hits.clone();
            sorted.sort_by(f32::total_cmp);
            let med = sorted[sorted.len() / 2];
            let mean = hits.iter().sum::<f32>() / hits.len() as f32;
            let jit =
                (hits.iter().map(|c| (c - mean).powi(2)).sum::<f32>() / hits.len() as f32).sqrt();
            let err = truth.map(|t| (med - t).abs()).unwrap_or(f32::NAN);
            per_partial.push((avail, err, jit, med_margin));
        }
        by_key.entry(key).or_default().push(per_partial);
    }

    println!(
        "{:>4} {:<5} {:>2} {:>8} {:>8} {:>9} {:>8}  caps",
        "key", "note", "n", "avail%", "|e| ¢", "jitter ¢", "margin"
    );
    for (key, caps_rows) in &by_key {
        let n_partials = caps_rows.iter().map(|r| r.len()).max().unwrap_or(0);
        for n in 0..n_partials {
            let rows: Vec<(f32, f32, f32, f32)> =
                caps_rows.iter().filter_map(|r| r.get(n).copied()).collect();
            if rows.is_empty() {
                continue;
            }
            let med = |mut v: Vec<f32>| -> f32 {
                v.retain(|x| x.is_finite());
                if v.is_empty() {
                    return f32::NAN;
                }
                v.sort_by(f32::total_cmp);
                v[v.len() / 2]
            };
            let av = med(rows.iter().map(|r| r.0).collect());
            let er = med(rows.iter().map(|r| r.1).collect());
            let ji = med(rows.iter().map(|r| r.2).collect());
            let mg = med(rows.iter().map(|r| r.3).collect());
            let flag = if av >= 0.9 && er <= 2.0 && ji <= 10.0 {
                " ✓"
            } else {
                ""
            };
            println!(
                "{:>4} {:<5} {:>2} {:>8.0} {:>8} {:>9} {:>8.2}  {:>4}{}",
                key,
                NOTES[*key as usize].name,
                n + 1,
                av * 100.0,
                if er.is_finite() {
                    format!("{er:.1}")
                } else {
                    "--".into()
                },
                if ji.is_finite() {
                    format!("{ji:.1}")
                } else {
                    "--".into()
                },
                mg,
                rows.len(),
                flag
            );
        }
    }
}

/// **T3 — realized false-alarm rate.** Converts the finite-N and
/// bin-correlation corrections from "conservative reasoning" into a measured
/// number, the same empirical-closure standard the NP gate audit set.
///
/// Runs the gated read over noise with no tone present and counts admissions.
/// Two populations: synthetic AWGN at the calibrated floor (exact H₀ — the
/// clean calibration), and the pre-onset segments of real captures
/// (`audio_full_event.raw` carries ~348 ms of pre-roll before the strike — real
/// room noise, the honest H₀). Nominal is P_fa = 0.001.
fn pfa_calibration(
    caps: &[PathBuf],
    planner: &mut RealFftPlanner<f32>,
    span_cents: f32,
    min_bins: f32,
    fft_size: usize,
) {
    let noise_floor = 0.001f32;
    let sweep: Vec<Gate> = [0.25f32, 0.5, 0.75, 0.9]
        .iter()
        .map(|&q| {
            Gate::Cfar(CfarCfg {
                quantile: q,
                guard_bins: 0,
                flanking: true,
                flank_min_hz: 172.0,
                finite_n: true,
            })
        })
        .collect();

    // ── Synthetic AWGN: exact H₀ ──────────────────────────────────────────
    // Deterministic xorshift so the run is reproducible; Box–Muller for normality.
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next_u = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f32 / (1u64 << 53) as f32
    };
    let len = SAMPLE_RATE as usize * 4;
    let noise: Vec<f32> = (0..len)
        .map(|_| {
            let (u1, u2) = (next_u().max(1e-9), next_u());
            noise_floor * (-2.0 * u1.ln()).sqrt() * (TAU * u2).cos()
        })
        .collect();

    let preroll = 15 * HOP_SIZE;
    println!(
        "{:<22} {:>12} {:>12}   (nominal 0.001)",
        "gate", "AWGN P_fa", "room P_fa"
    );

    // Band width buckets (bins): the search-loss correction scales with band
    // width, so a single pooled rate could hide a systematic error at one end.
    let bucket = |w: usize| -> usize {
        match w {
            0..=4 => 0,
            5..=8 => 1,
            9..=16 => 2,
            17..=32 => 3,
            _ => 4,
        }
    };
    let bucket_name = ["≤4", "5-8", "9-16", "17-32", ">32"];

    for gate in &sweep {
        let (mut trials, mut admits) = (0usize, 0usize);
        let mut bk_t = [0usize; 5];
        let mut bk_a = [0usize; 5];
        for key in (0u8..88).step_by(4) {
            let f_et = NOTES[key as usize].frequency;
            let b = get_expected_beta(key);
            let mut refs = [0.0f32; MAX_STROBE_REFS];
            let count = strobe_refs(f_et, b, MAX_BASS_PARTIAL, &mut refs);
            let series = multi_partial_series(
                &noise,
                &refs,
                count,
                f_et,
                fft_size,
                noise_floor,
                span_cents,
                min_bins,
                *gate,
                planner,
            );
            // Band width for this key's references (same for every partial:
            // the cap is spacing-driven, the span is per-partial but the bin
            // count is dominated by the cap in the bass and the span up top).
            let hz_per_bin = SAMPLE_RATE as f32 / fft_size as f32;
            for (i, hop) in series.iter().enumerate() {
                let _ = i;
                for (n, (v, _)) in hop.iter().enumerate() {
                    let half = search_halfwidth_hz(refs[n], f_et, span_cents, min_bins, hz_per_bin);
                    let w = ((2.0 * half / hz_per_bin).ceil() as usize).max(1);
                    let b = bucket(w);
                    bk_t[b] += 1;
                    bk_a[b] += usize::from(v.is_some());
                    trials += 1;
                    admits += usize::from(v.is_some());
                }
            }
        }

        let (mut r_trials, mut r_admits) = (0usize, 0usize);
        for dir in caps {
            let Some(key) = dir
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(crate::capture::key_from_dirname)
            else {
                continue;
            };
            let Some(full) = crate::raw::read(&dir.join("audio_full_event.raw")) else {
                continue;
            };
            if full.len() < preroll {
                continue;
            }
            let f_et = NOTES[key as usize].frequency;
            let b = get_expected_beta(key);
            let mut refs = [0.0f32; MAX_STROBE_REFS];
            let count = strobe_refs(f_et, b, MAX_BASS_PARTIAL, &mut refs);
            let series = multi_partial_series(
                &full[..preroll],
                &refs,
                count,
                f_et,
                fft_size,
                noise_floor,
                span_cents,
                min_bins,
                *gate,
                planner,
            );
            for hop in &series {
                for (v, _) in hop {
                    r_trials += 1;
                    r_admits += usize::from(v.is_some());
                }
            }
        }

        print!(
            "{:<26} {:>12.5} {:>12.5}   ",
            gate.label(),
            admits as f32 / trials.max(1) as f32,
            r_admits as f32 / r_trials.max(1) as f32,
        );
        // Per-band-width AWGN rates — a pooled figure can hide a systematic
        // error confined to wide or narrow bands.
        for b in 0..5 {
            if bk_t[b] > 0 {
                print!("{}:{:.4} ", bucket_name[b], bk_a[b] as f32 / bk_t[b] as f32);
            }
        }
        println!();
    }

    // ── F8: is the pre-onset region actually Gatekeeper-Silence? ──────────
    // If it is, the room-noise false-alarm rate is moot for the shipped design:
    // the coarse read would never be computed there. If it is NOT, the gate has
    // to carry that load itself and 0.34 is a real problem.
    let mut silent_frames = 0usize;
    let mut total_frames = 0usize;
    let mut planner2 = RealFftPlanner::<f32>::new();
    let fft_t = planner2.plan_fft_forward(WINDOW_SIZE);
    let fft_b = planner2.plan_fft_forward(BASS_WINDOW_SIZE);
    for dir in caps.iter().take(20) {
        let Some(full) = crate::raw::read(&dir.join("audio_full_event.raw")) else {
            continue;
        };
        if full.len() < preroll {
            continue;
        }
        let quiet = &full[..preroll];
        let mut frame = ProcessingFrame::new();
        let mut gate = Gatekeeper::new();
        // The LIVE default, not the harness's analysis floor: `PipelineAtomics`
        // ships silence_threshold = 0.005, and the calibration flow exists to
        // raise it above the room. Measuring Silence at 0.001 answers a
        // question the shipped app never asks.
        gate.config.silence_threshold = 0.005;
        let mut cursor = 0usize;
        while cursor + BASS_WINDOW_SIZE <= quiet.len() {
            frame.audio_buffer[..BASS_WINDOW_SIZE]
                .copy_from_slice(&quiet[cursor..cursor + BASS_WINDOW_SIZE]);
            let newest = BASS_WINDOW_SIZE - WINDOW_SIZE;
            fft(
                &frame.audio_buffer[newest..BASS_WINDOW_SIZE],
                &mut frame.time_buffer[..WINDOW_SIZE],
                &mut frame.frequency_buffer[..],
                &fft_t,
                WINDOW_SIZE,
            );
            fft(
                &frame.audio_buffer[..BASS_WINDOW_SIZE],
                &mut frame.time_buffer[..BASS_WINDOW_SIZE],
                &mut frame.bass_frequency_buffer[..],
                &fft_b,
                BASS_WINDOW_SIZE,
            );
            let gr = gate.process_frame(&frame);
            total_frames += 1;
            silent_frames += usize::from(gr.state == SignalState::Silence);
            cursor += HOP_SIZE;
        }
    }
    if total_frames > 0 {
        println!(
            "\nF8 — Gatekeeper on the same pre-onset audio: {:.1}% of frames are Silence \
             ({silent_frames}/{total_frames}).",
            100.0 * silent_frames as f32 / total_frames as f32
        );
        println!(
            "     The coarse read is only computed outside Silence, so the room-noise column \
             above is\n     load-bearing only for the non-Silence remainder."
        );
    }
}

/// Per-hop reads of **every** reference partial from a single FFT — the
/// structure the pipeline would actually use (one spectrum, several bounded
/// searches). Returns `[hop][partial] = (reading, CFAR margin)`.
#[allow(clippy::too_many_arguments)]
/// **T5 — reference-set anatomy.** The two questions Rohling's §V raises about
/// our reference window, measured rather than argued.
///
/// 1. **Composition.** His interference criterion (§V, journal p. 620) is that
///    an inhomogeneity is "minor" only if it "affects less than (N − k)
///    resolution cells". Our interferer is a harmonic comb, so with partial
///    spacing `s` bins and a Hann main lobe `W_lobe = 4` bins wide null-to-null
///    the criterion reads `k/N ≤ 1 − W_lobe/s`. This run classifies every
///    reference cell as **lobe** (within a main-lobe half-width of a partial) or
///    **valley**, reports the lobe fraction against that bound, and says which
///    class the selected order statistic actually landed in — the standing claim
///    that the wide flank "lets a low order statistic find the valleys between
///    partials" has never been checked.
/// 2. **Guard cells.** §V states they "become unnecessary" for OS CFAR, since a
///    small number of target amplitudes in the reference window "have almost no
///    influence on the clutter level estimation by quantiles". We keep ±2 on the
///    CA-CFAR rationale, so this sweeps `guard_bins` and reports what the guard
///    actually buys.
///
/// Both parts read the **coarse partial** (`curves::coarse_read_partial`), the
/// one the shipped readout centres on.
fn ref_anatomy(
    caps: &[PathBuf],
    planner: &mut RealFftPlanner<f32>,
    span_cents: f32,
    min_bins: f32,
    fft_size: usize,
) {
    let hz_per_bin = SAMPLE_RATE as f32 / fft_size as f32;
    let fftp = planner.plan_fft_forward(fft_size);
    // Hann main-lobe half-width, in bins: the lobe is 4 bins null-to-null.
    const LOBE_HALF_BINS: f32 = 2.0;

    println!(
        "── Part A: reference-cell composition ({} bins, {:.2} Hz/bin) ──\n\
         {:>4} {:<5} {:>2} {:>8} {:>7} {:>6} {:>7} {:>7} {:>8} {:>7} {:>13}  criterion",
        fft_size,
        hz_per_bin,
        "key",
        "note",
        "n*",
        "center",
        "s bins",
        "N_ref",
        "lobe %",
        "q bound",
        "sel=lobe%",
        "sel dB",
        "sel partial",
    );

    for dir in caps {
        let Some(key) = dir
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(crate::capture::key_from_dirname)
        else {
            continue;
        };
        let Some(signal) = crate::raw::read(&dir.join("audio.raw")) else {
            continue;
        };
        if signal.len() < BASS_WINDOW_SIZE {
            continue;
        }
        let f_et = NOTES[key as usize].frequency;
        let b = get_expected_beta(key);
        let n_star = curves::coarse_read_partial(key) as usize;
        let mut series = [0.0f32; MAX_STROBE_REFS];
        let count = strobe_refs(f_et, b, MAX_STROBE_REFS, &mut series);
        if n_star == 0 || n_star > count {
            continue;
        }
        let center = series[n_star - 1];

        // Predicted partial bins for the lobe test: the whole stiff-string
        // series up to Nyquist, not just the strobe's few references.
        let nyq = SAMPLE_RATE as f32 / 2.0;
        let partial_bins: Vec<f32> = (1..)
            .map(|m| {
                let mf = m as f32;
                mf * f_et * (1.0 + b * mf * mf).sqrt()
            })
            .take_while(|f| *f < nyq)
            .map(|f| f / hz_per_bin)
            .collect();

        let cfg = match shipping_gate() {
            Gate::Cfar(c) => c,
            Gate::Ambient => continue,
        };

        let mut time = vec![0.0f32; fft_size];
        let mut spec = vec![Complex { re: 0.0, im: 0.0 }; fft_size / 2 + 1];
        let mut mag = vec![0.0f32; fft_size / 2];
        let mut lobe_fracs: Vec<f32> = Vec::new();
        let mut sel_db: Vec<f32> = Vec::new();
        let mut sel_partial: Vec<f32> = Vec::new();
        let (mut sel_lobe, mut hops) = (0usize, 0usize);
        let mut cursor = 0usize;

        while cursor + BASS_WINDOW_SIZE <= signal.len() {
            let end = cursor + BASS_WINDOW_SIZE;
            cursor += HOP_SIZE;
            fft(
                &signal[end - fft_size..end],
                &mut time,
                &mut spec,
                &fftp,
                fft_size,
            );
            magnitude_spectrum(&spec, fft_size, &mut mag);

            let half = search_halfwidth_hz(center, f_et, span_cents, min_bins, hz_per_bin);
            let n_bins = mag.len();
            let lo = (((center - half) / hz_per_bin).floor().max(1.0)) as usize;
            let hi = ((((center + half) / hz_per_bin).ceil()) as usize).min(n_bins.max(2) - 2);
            if lo >= hi {
                continue;
            }
            let (mut best, mut best_mag) = (lo, 0.0f32);
            for (i, &m) in mag[lo..=hi].iter().enumerate() {
                if m > best_mag {
                    best_mag = m;
                    best = lo + i;
                }
            }

            let (outer_lo, outer_hi) = ref_window(lo, hi, f_et, &cfg, hz_per_bin, n_bins);
            let cells: Vec<(usize, f32)> = (outer_lo..lo)
                .chain((hi + 1)..=outer_hi)
                .filter(|bin| bin.abs_diff(best) > cfg.guard_bins)
                .map(|bin| (bin, mag[bin]))
                .collect();
            if cells.len() < 4 {
                continue;
            }

            // Which partial a reference cell belongs to, if any — 1-based, so the
            // "reach partial m" hypothesis for the flank floor can be tested.
            let lobe_of = |bin: usize| -> Option<usize> {
                let x = bin as f32;
                partial_bins
                    .iter()
                    .position(|pb| (x - pb).abs() <= LOBE_HALF_BINS)
                    .map(|i| i + 1)
            };
            let is_lobe = |bin: usize| -> bool { lobe_of(bin).is_some() };
            let n_lobe = cells.iter().filter(|(bin, _)| is_lobe(*bin)).count();
            lobe_fracs.push(n_lobe as f32 / cells.len() as f32);

            // The selected order statistic, by the shipped rank rule.
            let mut sorted = cells.clone();
            sorted.sort_by(|a, b| a.1.total_cmp(&b.1));
            let k = (((sorted.len() as f32 - 1.0) * cfg.quantile).round() as usize).max(1);
            let (sel_bin, sel_mag) = sorted[k.min(sorted.len() - 1)];
            if let Some(m) = lobe_of(sel_bin) {
                sel_lobe += 1;
                sel_partial.push(m as f32);
            }
            if sel_mag > 0.0 && best_mag > 0.0 {
                sel_db.push(20.0 * (sel_mag / best_mag).log10());
            }
            hops += 1;
        }

        if hops == 0 {
            continue;
        }
        let median = |v: &mut Vec<f32>| -> f32 {
            v.sort_by(f32::total_cmp);
            v.get(v.len() / 2).copied().unwrap_or(f32::NAN)
        };
        let lobe_pct = median(&mut lobe_fracs) * 100.0;
        let bound = 1.0 - lobe_pct / 100.0;
        let sel_pct = 100.0 * sel_lobe as f32 / hops as f32;
        println!(
            "{:>4} {:<5} {:>2} {:>8.1} {:>7.2} {:>6} {:>7.1} {:>7.3} {:>8.0} {:>7.1} {:>13}  {}",
            key,
            NOTES[key as usize].name,
            n_star,
            center,
            f_et / hz_per_bin,
            hops,
            lobe_pct,
            bound,
            sel_pct,
            median(&mut sel_db),
            if sel_partial.is_empty() {
                "—".to_string()
            } else {
                let lo = sel_partial.iter().cloned().fold(f32::MAX, f32::min);
                let hi = sel_partial.iter().cloned().fold(0.0f32, f32::max);
                format!(
                    "n{}–{} med {}",
                    lo as u32,
                    hi as u32,
                    median(&mut sel_partial) as u32
                )
            },
            if cfg.quantile <= bound {
                "q ≤ bound"
            } else {
                "**q > bound**"
            }
        );
    }

    // ── Part B: does the guard buy anything? ──────────────────────────────
    println!(
        "\n── Part B: guard-cell sweep at the coarse partial (Rohling §V: \
         \"unnecessary\" for OS CFAR) ──\n\
         {:>5} {:>8} {:>8} {:>9} {:>8} {:>6}",
        "guard", "avail%", "|e| ¢", "jitter ¢", "margin", "caps"
    );
    for guard in 0..=4usize {
        let gate = match shipping_gate() {
            Gate::Cfar(c) => Gate::Cfar(CfarCfg {
                guard_bins: guard,
                ..c
            }),
            Gate::Ambient => continue,
        };
        let (mut avails, mut errs, mut jits, mut margins) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for dir in caps {
            let Some(key) = dir
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(crate::capture::key_from_dirname)
            else {
                continue;
            };
            let Some(signal) = crate::raw::read(&dir.join("audio.raw")) else {
                continue;
            };
            if signal.len() < BASS_WINDOW_SIZE {
                continue;
            }
            let f_et = NOTES[key as usize].frequency;
            let b = get_expected_beta(key);
            let n_star = curves::coarse_read_partial(key) as usize;
            let mut series = [0.0f32; MAX_STROBE_REFS];
            let count = strobe_refs(f_et, b, MAX_STROBE_REFS, &mut series);
            if n_star == 0 || n_star > count {
                continue;
            }
            let mut one = [0.0f32; MAX_STROBE_REFS];
            one[0] = series[n_star - 1];
            let hops = multi_partial_series(
                &signal, &one, 1, f_et, fft_size, 0.001, span_cents, min_bins, gate, planner,
            );
            if hops.is_empty() {
                continue;
            }
            let truth = dtft_truth(&signal, one[0], planner).map(|f| cents(f, one[0]));
            let hits: Vec<f32> = hops
                .iter()
                .filter_map(|h| h[0].0)
                .map(|f| cents(f, one[0]))
                .filter(|c| c.is_finite())
                .collect();
            let mut ms: Vec<f32> = hops.iter().map(|h| h[0].1).collect();
            ms.sort_by(f32::total_cmp);
            margins.push(ms[ms.len() / 2]);
            avails.push(100.0 * hits.len() as f32 / hops.len() as f32);
            if hits.is_empty() {
                continue;
            }
            let mut sorted = hits.clone();
            sorted.sort_by(f32::total_cmp);
            let med = sorted[sorted.len() / 2];
            let mean = hits.iter().sum::<f32>() / hits.len() as f32;
            jits.push(
                (hits.iter().map(|c| (c - mean).powi(2)).sum::<f32>() / hits.len() as f32).sqrt(),
            );
            if let Some(t) = truth {
                errs.push((med - t).abs());
            }
        }
        let mean_of = |v: &[f32]| -> f32 {
            if v.is_empty() {
                f32::NAN
            } else {
                v.iter().sum::<f32>() / v.len() as f32
            }
        };
        println!(
            "{:>5} {:>8.1} {:>8.2} {:>9.2} {:>8.2} {:>6}",
            guard,
            mean_of(&avails),
            mean_of(&errs),
            mean_of(&jits),
            mean_of(&margins),
            avails.len()
        );
    }
}

/// **Port verification — this harness's read vs the shipped one.**
///
/// The measurement rounds settled the coarse read here, in [`spectral_read`]
/// under [`shipping_gate`]; the hot path now carries its own copy in
/// `peaks::coarse_read`. Every number on record was produced by *this* code, so
/// the shipped one has to reproduce it bit-for-bit or the record does not
/// transfer. Run over real captures at both analysis sizes, on the partial the
/// shipped rule (`curves::coarse_read_partial`) actually selects.
///
/// Reports per FFT size: hops compared, admission agreement, and the largest
/// frequency disagreement among hops both admitted.
fn verify_shipped(caps: &[PathBuf], planner: &mut RealFftPlanner<f32>) {
    println!(
        "Port verification — harness `spectral_read` vs shipped `peaks::coarse_read`.\n\
         Both at the shipping gate, on partial `curves::coarse_read_partial(key)`.\n"
    );
    println!("  fft  |    hops | admit-agree | max Δf (Hz) | worst case");
    println!("  -----|---------|-------------|-------------|-----------");

    for &fft_size in &[BASS_WINDOW_SIZE, WINDOW_SIZE] {
        let fftp = planner.plan_fft_forward(fft_size);
        let mut time = vec![0.0f32; fft_size];
        let mut spec = vec![Complex { re: 0.0, im: 0.0 }; fft_size / 2 + 1];
        let mut mag = vec![0.0f32; fft_size / 2];
        let mut harness_scratch: Vec<f32> = Vec::new();
        let mut shipped_scratch = vec![0.0f32; fft_size / 2];

        let (mut hops, mut agree, mut max_df, mut worst) = (0usize, 0usize, 0.0f32, String::new());

        for dir in caps {
            let Some(key) = dir
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(crate::capture::key_from_dirname)
            else {
                continue;
            };
            let Some(signal) = crate::raw::read(&dir.join("audio.raw")) else {
                continue;
            };
            if signal.len() < BASS_WINDOW_SIZE {
                continue;
            }

            // The shipping reference set, exactly as the GUI builds it: prior B
            // (unmeasured key), f₀ spacing, coarse partial from the derived rule.
            let f_et = NOTES[key as usize].frequency;
            let b = get_expected_beta(key);
            let n_star = curves::coarse_read_partial(key) as usize;
            let mut refs = [0.0f32; MAX_STROBE_REFS];
            let count = strobe_refs(f_et, b, MAX_STROBE_REFS, &mut refs);
            if n_star > count {
                continue;
            }
            let center = refs[n_star - 1];
            let spacing = f_et / (1.0 + b).sqrt();

            let mut cursor = 0usize;
            while cursor + BASS_WINDOW_SIZE <= signal.len() {
                let end = cursor + BASS_WINDOW_SIZE;
                fft(
                    &signal[end - fft_size..end],
                    &mut time,
                    &mut spec,
                    &fftp,
                    fft_size,
                );
                magnitude_spectrum(&spec, fft_size, &mut mag);
                cursor += HOP_SIZE;

                let theirs = spectral_read(
                    &mag,
                    &spec,
                    fft_size,
                    center,
                    spacing,
                    0.005,
                    100.0,
                    4.0,
                    shipping_gate(),
                    &mut harness_scratch,
                );
                let theirs = match theirs.read {
                    Read::Hit(f) => Some(f),
                    _ => None,
                };
                let ours = peaks::coarse_read(
                    &mag,
                    &spec,
                    fft_size,
                    SAMPLE_RATE,
                    center,
                    spacing,
                    &mut shipped_scratch,
                );

                hops += 1;
                match (theirs, ours) {
                    (Some(a), Some(b)) => {
                        agree += 1;
                        let d = (a - b).abs();
                        if d > max_df {
                            max_df = d;
                            worst = format!("key {key} {a:.4} vs {b:.4}");
                        }
                    }
                    (None, None) => agree += 1,
                    (t, o) => {
                        if worst.is_empty() {
                            worst = format!("key {key} admit {t:?} vs {o:?}");
                        }
                    }
                }
            }
        }
        let pct = if hops > 0 {
            100.0 * agree as f32 / hops as f32
        } else {
            0.0
        };
        println!(
            "  {fft_size:<4} | {hops:>7} | {pct:>10.4}% | {max_df:>11.2e} | {}",
            if worst.is_empty() { "—" } else { &worst }
        );
    }
    println!(
        "\nAgreement must be 100.0000% with Δf = 0: the shipped read is a port, \
         not a reimplementation."
    );
}

/// **Gate A/B.** The same bounded spectral read under every gate in
/// [`gate_variants`], on one capture, at the partial given by `partial`.
///
/// Two opposite tests share this table. On the **deep bass** it is a rejection
/// test: the n = 1 readings there are known junk, and the ambient gate admits
/// them at ~98 % — a better gate should reject. On **A7/C8** it is an
/// admission test: the readings are accurate but scarce, and a better gate
/// should recover availability without losing accuracy.
#[allow(clippy::too_many_arguments)]
fn gate_ab(
    dir: &Path,
    planner: &mut RealFftPlanner<f32>,
    span_cents: f32,
    min_bins: f32,
    partial: usize,
    fft_size: usize,
) -> Option<()> {
    let key = dir
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(crate::capture::key_from_dirname)?;
    let signal = crate::raw::read(&dir.join("audio.raw"))?;
    if signal.len() < BASS_WINDOW_SIZE {
        return None;
    }
    let f_et = NOTES[key as usize].frequency;
    let b = get_expected_beta(key);
    let mut refs = [0.0f32; MAX_STROBE_REFS];
    let count = strobe_refs(f_et, b, partial.max(1), &mut refs);
    if count < partial {
        return None;
    }
    let r_n = refs[partial - 1];
    let truth_n = dtft_truth(&signal, r_n, planner).map(|f| cents(f, r_n));

    print!(
        "{:<4} key {:>2}  n={partial} r={r_n:>9.2}  ",
        NOTES[key as usize].name, key
    );
    match truth_n {
        Some(t) => println!("truth {t:>+7.1}¢"),
        None => println!("truth      --"),
    }

    for gate in gate_variants() {
        let (series, no_ref, med_ref) = spectral_series(
            &signal, r_n, f_et, fft_size, 0.001, span_cents, min_bins, gate, planner,
        );
        let s = score(&series, r_n);
        let err = truth_n.map(|t| s.median_cents - t);
        print!(
            "     {}  av{:>4.0}%  no-ref{:>4.0}%  ref{:>4} bins  ",
            gate.label(),
            s.avail * 100.0,
            100.0 * no_ref as f32 / series.len().max(1) as f32,
            med_ref
        );
        match err {
            Some(e) if s.median_cents.is_finite() => println!("e{e:>+8.1}¢  j{:>7.1}¢", s.jitter),
            _ => println!("e      --    j     --"),
        }
    }
    println!();
    Some(())
}

// ── Mode entry points ────────────────────────────────────────────────────────

use anyhow::Result;

use crate::capture;
use crate::gates::GateOpts;

/// The captures a gate study runs over, honouring `--keys`.
fn population(o: &GateOpts) -> Result<Vec<PathBuf>> {
    let mut caps = capture::find_or_single(&o.root);
    if let Some(keys) = &o.keys {
        capture::retain_keys(&mut caps, keys);
    }
    if caps.is_empty() {
        anyhow::bail!("No captures found under {}", o.root.display());
    }
    Ok(caps)
}

pub(super) fn profile(o: &GateOpts, max_n: Option<usize>) -> Result<()> {
    let fft_size = o.fft;
    println!(
        "T1 — per-key × per-partial profile under the settled gate \
         (os25 / ±2 guard / flank floor {FLANK_MIN_HZ:.0} Hz / finite-N, search-loss corrected) \
         at FFT {fft_size}.\n\
         Rows are medians over each key's repeat captures. margin = peak ÷ threshold \
         (> 1 admits; near 1 = one strike away from flipping).\n\
         ✓ marks a (key, partial) meeting the criterion: avail ≥ 90 %, |e| ≤ 2 ¢, jitter ≤ 10 ¢.\n"
    );
    cfar_profile(
        &population(o)?,
        &mut RealFftPlanner::<f32>::new(),
        o.span,
        o.min_bins,
        fft_size,
        max_n.unwrap_or(MAX_BASS_PARTIAL),
    );
    Ok(())
}

pub(super) fn pfa(o: &GateOpts) -> Result<()> {
    println!(
        "T3 — realized false-alarm rate of the settled gate, measured on signal-free input.\n"
    );
    pfa_calibration(
        &population(o)?,
        &mut RealFftPlanner::<f32>::new(),
        o.span,
        o.min_bins,
        o.fft,
    );
    Ok(())
}

pub(super) fn refset(o: &GateOpts) -> Result<()> {
    println!(
        "T5 — reference-set anatomy: is the selected order statistic a valley cell or a \
         weak partial's lobe, and does the guard buy anything (Rohling §V)?\n"
    );
    ref_anatomy(
        &population(o)?,
        &mut RealFftPlanner::<f32>::new(),
        o.span,
        o.min_bins,
        o.fft,
    );
    Ok(())
}

pub(super) fn verify(o: &GateOpts) -> Result<()> {
    verify_shipped(&population(o)?, &mut RealFftPlanner::<f32>::new());
    Ok(())
}

pub(super) fn ab(o: &GateOpts, partial: usize) -> Result<()> {
    println!(
        "Gate A/B — the same 8192 bounded read under the shipped ambient-σ gate and four \
         OS-CFAR variants.\n\
         Deep bass = rejection test (ambient admits known junk); A7/C8 = admission test \
         (ambient rejects good signal).\n\
         os50/os25 = order statistic; g2 = ±2 guard bins; band/flank = reference cells inside \
         the search band or outside it.\n\
         Multipliers (asymptotic, P_fa = 0.001): median ×{:.3}, 25th pct ×{:.3}.\n",
        cfar_multiplier(0.5),
        cfar_multiplier(0.25)
    );
    let mut planner = RealFftPlanner::<f32>::new();
    for cap in &population(o)? {
        gate_ab(cap, &mut planner, o.span, o.min_bins, partial, o.fft);
    }
    Ok(())
}
