//! # Strobe replay — offline metrics for the Path-A strobe bank
//!
//! Runs the **shipped** [`Strobe`] over captured real audio
//! (`diagnostics/key_*/audio.raw`) and measures rotation fidelity with hard
//! numbers, so the strobe design note §15 "unlocked-regime" and R3 bass
//! questions can be answered without eyeballing a spinning disc. Every capture
//! we have is of a **detuned** piano, which is exactly the regime the strobe
//! is claimed to help in.
//!
//! It replays the pipeline cadence faithfully: an 8192-sample window advanced
//! one 1024-sample hop at a time, `Strobe::process` per hop, then the
//! per-hop `angle` (cycles, mod 1) is unwrapped in the harness and
//! least-squares-fit to recover the rotation rate and its residual.
//!
//! ## The experiments (they answer different questions)
//!
//! **E1 — detuning coherence (ET fundamental).** Reference = the pure ET
//! harmonic series `n·f_ET` (B = 0); we read the **fundamental** band (n = 1),
//! which is B-immune (R4), so its rotation is *pure pitch detuning*. This is
//! the "string is off, engine can't lock, does the band still show the offset
//! coherently" test — and the preview of a guitar/ET strobe mode. Only
//! meaningful where the fundamental is alive and the detuning is inside the
//! ±(fs/2·hop) ≈ 21.5 Hz phase-unwrap range (beyond that the rotation aliases;
//! D4's coarse cents indicator is what gets the user back into range).
//!
//! **E2 — steadiness + bass window (measured reference).** Reference = the
//! string's own measured stiff-string series `n·f0·√(1+Bn²)`, so a stable
//! string *should* hold each band ≈ stationary; residual rotation is then a
//! steadiness/noise metric. We read the **displayed** partial (the coarse
//! register table) and, for bass keys, A/B the 4096-sample window (auto) vs a
//! forced 1024 window to quantify the R3 payoff on real bass audio.
//!
//! **E3 — per-hop delta noise.** The quantity `BAND_READABLE_HZ`'s margin below
//! the alias boundary is made of (ADR 0011 §10).
//!
//! **E4 — fit-window length.** Slope jitter against window length, measured, vs
//! the exact `T/2` motion lag (ADR 0011 §11).
//!
//! **E5 — shipped rate vs an independent fit.** `StrobeResult::beat_hz` is the
//! bank's own least-squares slope; E5 refits the same retained points here and
//! reports the disagreement, so the DSP-side estimator is checked against real
//! audio rather than only against synthetic hops.
//!
//! ### Unison assist (ADR 0012)
//!
//! **E6 — the estimator against synthetic truth.** Resolution law, accuracy, and
//! the false-split null, on signals with known lines. The port must reproduce the
//! design note's own tables: 50 % of pairs resolved at `2/T` and 100 % at
//! ≈1.35·`2/T`; bias ≤ 0.02 Hz and σ ≤ 0.06 Hz where it resolves; **zero** false
//! second lines on a single string across SNR, decay and record length.
//!
//! **E7 — availability and repeat reproducibility (real).** How often the ring
//! reaches a usable record per register, and — the truth-free test — whether
//! independent strikes of the same key agree on the split they report.
//!
//! **E8 — unexplained lines.** Every reported line matched against a full-rate
//! DFT of the *identical* span with an **uncapped** peak picker, and the residue
//! classified: is the energy there at all, and does the residue concentrate in
//! the weakest line? This is the open item ADR 0012 §8 carries.
//!
//! **E9 — cost.** µs per hop in `--release` for the whole bank, against the
//! 23.2 ms callback the pipeline has to fit inside.
//!
//! ### The bass extra lines (ADR 0013)
//!
//! **E10 — the bass configuration, which every trial above skips.** The deep
//! bass runs a 4096-sample Goertzel against the same 1024-sample hop (R3), so
//! its baseband is 4× oversampled and its noise correlated — the independence
//! the CFAR null assumes. The null, the resolution law and the noise
//! correlation are measured there against the same audio through the 1024
//! window; then the folded interferer's *strength* (the open E-Q question) and
//! a sweep of how far up the compass a string's own neighbouring partials fold
//! into its baseband.
//!
//! **E11 — do the extra lines sit at fixed absolute frequencies?** Recurrence
//! across *different keys* is the signature of an instrument or room resonance
//! rather than a property of the struck string.
//!
//! **E12 — attribution.** Every extra line against the families that could have
//! produced it, each predicted from the instrument's own measured (f₀, B) and
//! scored against a permutation null; then which side of the partial they sit
//! on, the law their splits follow, and whether a third line is a symmetric
//! sideband.
//!
//! Run: `cargo run --release --example strobe_replay -- [diagnostics_dir]`

use std::path::{Path, PathBuf};
use std::time::Instant;

use realfft::RealFftPlanner;
use rustfft::num_complex::Complex;

use tuner_core::algorithms::curves::default_display_partials;
use tuner_core::algorithms::peaks::{LineScratch, MAX_UNISON_LINES, resolve_lines};
use tuner_core::algorithms::spectral::{self, goertzel, goertzel_bass};
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_RATE_HZ, HOP_SIZE, SAMPLE_RATE, WINDOW_SIZE};
use tuner_core::models::{NOTES, UnisonLine};
use tuner_core::strobe::band_slope::{
    BAND_SLOPE_MIN_POINTS, BAND_SLOPE_POINTS, BAND_SLOPE_WINDOW_SECS,
};
use tuner_core::strobe::unison::{UNISON_RING_HOPS, UnisonVerdict};
use tuner_core::strobe::{MAX_STROBE_REFS, Strobe, StrobeRefUpdate};

/// Phase-unwrap / rotation Nyquist: the largest offset (Hz) whose per-hop
/// phase advance stays inside ±0.5 cycle, hence readable as rotation.
const ALIAS_HZ: f32 = 0.5 * HOP_RATE_HZ;

/// Hops the bank least-squares-fits the band slope over. E3 detrends over the
/// same span, so its noise figure is the one that window actually sees.
const BAND_WIN_HOPS: usize = (BAND_SLOPE_WINDOW_SECS * HOP_RATE_HZ) as usize;

/// Fit-window lengths E4 sweeps, in seconds. 0.186 s is the 8192-sample analysis
/// window — the coarse read's own group delay, and so the length at which the two
/// readouts would lag the truth equally. 0.6 s is shipped.
const WINDOW_SECS: [f32; 5] = [0.186, 0.25, 0.4, 0.6, 1.0];

fn read_raw_f32(path: &Path) -> Option<Vec<f32>> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() % 4 != 0 {
        return None;
    }
    let n = bytes.len() / 4;
    let mut out = vec![0.0f32; n];
    // SAFETY: f32 has no invalid bit patterns; length is a multiple of 4.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.as_mut_ptr() as *mut u8, bytes.len());
    }
    Some(out)
}

/// One capture's measured facts, pulled from `analysis.json`.
struct Capture {
    key: u8,
    f0: f32,
    /// The capture's own `calculated_b`, which E12 uses only to *predict where
    /// another key's partials sit* — never to calibrate a synthetic generator.
    b: f32,
    noise_floor: f32,
    /// Measured partial frequency by number (1-indexed), `None` if absent.
    partial_hz: Vec<Option<f32>>,
    audio: Vec<f32>,
}

fn load(dir: &Path) -> Option<Capture> {
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("analysis.json")).ok()?).ok()?;
    let m = &j["metadata"];
    let key = m["key_index"].as_u64()? as u8;
    let f0 = m["mat_f0"].as_f64().or_else(|| m["measured_f0"].as_f64())? as f32;
    let noise_floor = m["noise_floor"].as_f64().unwrap_or(0.003) as f32;
    let b = m["calculated_b"].as_f64().unwrap_or(0.0) as f32;

    let mut partial_hz = vec![None; MAX_STROBE_REFS + 1];
    if let Some(arr) = m["partials"].as_array() {
        for p in arr {
            if let (Some(n), Some(f)) = (p["number"].as_u64(), p["frequency"].as_f64())
                && (n as usize) <= MAX_STROBE_REFS
            {
                partial_hz[n as usize] = Some(f as f32);
            }
        }
    }
    let audio = read_raw_f32(&dir.join("audio.raw"))?;
    if audio.len() < BASS_WINDOW_SIZE + 4 * HOP_SIZE {
        return None; // too short to fit a warmup + a few integrating hops
    }
    Some(Capture {
        key,
        f0,
        b,
        noise_floor,
        partial_hz,
        audio,
    })
}

/// Least-squares slope (per hop) and residual std of an unwrapped series.
fn fit(series: &[f32]) -> (f32, f32) {
    let n = series.len() as f32;
    let mean_x = (n - 1.0) / 2.0;
    let mean_y = series.iter().sum::<f32>() / n;
    let (mut sxy, mut sxx) = (0.0f32, 0.0f32);
    for (i, &y) in series.iter().enumerate() {
        let dx = i as f32 - mean_x;
        sxy += dx * (y - mean_y);
        sxx += dx * dx;
    }
    let slope = if sxx > 0.0 { sxy / sxx } else { 0.0 };
    let mut ss = 0.0f32;
    for (i, &y) in series.iter().enumerate() {
        let pred = mean_y + slope * (i as f32 - mean_x);
        ss += (y - pred) * (y - pred);
    }
    (slope, (ss / n).sqrt())
}

/// **Per-hop phase-delta noise σ_d, in cycles** — the quantity the readable-range
/// margin is made of.
///
/// The GUI unwraps one delta per DSP frame, so the branch folds as soon as a
/// *single* hop's delta leaves ±0.5 cycle. With true rate `Δf/f_hop` and delta
/// noise σ_d, staying inside the branch needs
/// `|Δf|/f_hop + z·σ_d < 0.5`, i.e. a readable limit of `f_hop·(0.5 − z·σ_d)`
/// and a margin below the alias boundary of **`f_hop·z·σ_d`**.
///
/// Deltas are detrended over the GUI's own fit window (`win_hops`), so a
/// constant rotation — the detuning itself — contributes nothing; only
/// hop-to-hop noise does. Windows whose mean delta is already near the branch
/// edge are dropped, since a folded delta would be measured as noise. Returns
/// `(σ_d, p99.9 |Δ|, max |Δ|)`: folding is a *tail* event, so the tail is what
/// must size the margin.
fn delta_noise(unwrapped: &[f32], usable: &[bool], win_hops: usize) -> Option<(f32, f32, f32)> {
    let d: Vec<(f32, bool)> = unwrapped
        .windows(2)
        .zip(usable.windows(2))
        .map(|(w, u)| (w[1] - w[0], u[0] && u[1]))
        .collect();
    if d.len() < win_hops + 1 {
        return None;
    }
    let mut resid: Vec<f32> = Vec::new();
    for w in d.windows(win_hops) {
        if w.iter().any(|(_, ok)| !ok) {
            continue; // the GUI drops its baseline across a gated hop
        }
        let mean = w.iter().map(|(x, _)| *x).sum::<f32>() / win_hops as f32;
        if mean.abs() > 0.35 {
            continue; // already near the branch edge: a fold would read as noise
        }
        resid.extend(w.iter().map(|(x, _)| x - mean));
    }
    if resid.len() < win_hops {
        return None;
    }
    let n = resid.len() as f32;
    // Sliding windows overlap, so this is a pooled within-window variance; one
    // Bessel correction for the mean removed from each window.
    let var =
        resid.iter().map(|x| x * x).sum::<f32>() / n * win_hops as f32 / (win_hops - 1) as f32;
    let mut mag: Vec<f32> = resid.iter().map(|x| x.abs()).collect();
    mag.sort_by(f32::total_cmp);
    let p999 = mag[((mag.len() as f32 * 0.999) as usize).min(mag.len() - 1)];
    Some((var.sqrt(), p999, *mag.last().unwrap()))
}

/// **Slope jitter vs fit-window length** — the lower bound on the band-slope
/// window, measured rather than assumed.
///
/// The unwrapped series telescopes (`y_h = Σd_i = φ_h − φ_0`), so its samples
/// carry the *phase* noise σ_η with `Var(d) = 2σ_η²`, and textbook OLS would give
/// `Var(slope) = σ_η² / Σ(x−x̄)²` with `Σ(x−x̄)² = n(n²−1)/12`. That assumes
/// independent samples, and the bank's windows overlap 75–87 %, so the prediction
/// is optimistic by an unknown factor. This measures the real thing: fit over
/// **non-overlapping** windows of `n` hops, then take successive differences of
/// the fitted rates (÷√2) so a slowly drifting true rate is not counted as noise.
///
/// Returns the successive rate *differences* (cycles/hop) for the caller to pool:
/// a 1.3 s capture yields only two non-overlapping 0.6 s windows, so a per-capture
/// variance would be meaningless at the lengths that matter.
///
/// The figure this produces is an **upper bound** on estimator noise, since a
/// genuine drift in the string's rate between windows also lands in the
/// difference.
fn slope_diffs(unwrapped: &[f32], usable: &[bool], n: usize) -> Vec<f32> {
    let mut rates: Vec<f32> = Vec::new();
    let mut i = 0usize;
    while i + n <= unwrapped.len() {
        if usable[i..i + n].iter().all(|u| *u) {
            rates.push(fit(&unwrapped[i..i + n]).0);
        }
        i += n;
    }
    rates.windows(2).map(|w| w[1] - w[0]).collect()
}

/// **Shipped rate vs an independent fit (E5).** For every hop the bank published
/// a rate on, refits the points it held — the current unbroken ungated run,
/// capped at the window — and returns the absolute disagreements in Hz.
///
/// The bank restarts its fit on the first live hop after a gate, so the run's
/// first point carries no delta; that only shifts the series by a constant,
/// which an OLS slope ignores.
fn rate_disagreement(
    unwrapped: &[f32],
    ungated: &[bool],
    shipped: &[Option<f32>],
    win_hops: usize,
    min_hops: usize,
) -> Vec<f32> {
    let mut out = Vec::new();
    let mut run_start = 0usize;
    for i in 0..unwrapped.len() {
        if !ungated[i] {
            run_start = i + 1;
            continue;
        }
        let Some(rate) = shipped[i] else { continue };
        let held = (i - run_start + 1).min(win_hops);
        if held < min_hops {
            continue;
        }
        let mine = fit(&unwrapped[i + 1 - held..=i]).0 * HOP_RATE_HZ;
        out.push((rate - mine).abs());
    }
    out
}

/// Drives the shipped [`Strobe`] over the capture with a given reference
/// set; returns the per-hop unwrapped angle (cycles), the per-hop **ungated**
/// mask, and the per-hop published beat rate (Hz) for the band at `read_idx`
/// (0-based partial index), aligned index-wise.
fn run_strobe(
    cap: &Capture,
    refs: &[f32; MAX_STROBE_REFS],
    count: usize,
    read_idx: usize,
) -> (Vec<f32>, Vec<bool>, Vec<Option<f32>>) {
    let mut strobe = Strobe::new(SAMPLE_RATE);
    strobe.retarget(StrobeRefUpdate {
        count,
        refs: *refs,
        // Phase-only replay: the coarse read is not exercised here.
        coarse_index: 0,
        spacing_hz: cap.f0,
    });

    let hops = (cap.audio.len() - BASS_WINDOW_SIZE) / HOP_SIZE;
    let mut unwrapped = Vec::with_capacity(hops);
    let mut ungated = Vec::with_capacity(hops);
    let mut shipped = Vec::with_capacity(hops);
    let (mut prev, mut acc) = (0.0f32, 0.0f32);
    let mut frame_buf = tuner_core::pipeline::ProcessingFrame::new();
    for h in 0..hops {
        let win = &cap.audio[h * HOP_SIZE..h * HOP_SIZE + BASS_WINDOW_SIZE];
        frame_buf.audio_buffer[..BASS_WINDOW_SIZE].copy_from_slice(win);
        let fr = strobe.process(&frame_buf, cap.noise_floor, false);
        if h == 0 {
            prev = fr.angle[read_idx];
            continue;
        }
        // Unwrap mod-1: bring the per-hop delta into [-0.5, 0.5).
        let mut d = fr.angle[read_idx] - prev;
        d -= d.round();
        acc += d;
        unwrapped.push(acc);
        ungated.push(!fr.gated[read_idx]);
        shipped.push(fr.beat_hz[read_idx]);
        prev = fr.angle[read_idx];
    }
    (unwrapped, ungated, shipped)
}

/// Direct phase-accumulation on a single reference with a chosen window,
/// for the bass A/B (bypasses the bank's auto window selection). Returns the
/// residual-std of the unwrapped angle — the steadiness metric.
fn steadiness(cap: &Capture, f_ref: f32, long: bool) -> f32 {
    let t_hop = HOP_SIZE as f32 / SAMPLE_RATE as f32;
    let expected = 2.0 * std::f32::consts::PI * f_ref * t_hop;
    let hops = (cap.audio.len() - BASS_WINDOW_SIZE) / HOP_SIZE;
    let (mut prev, mut acc) = (0.0f32, 0.0f32);
    let mut series = Vec::with_capacity(hops);
    for h in 0..hops {
        let win = &cap.audio[h * HOP_SIZE..h * HOP_SIZE + BASS_WINDOW_SIZE];
        let (_amp, phase) = if long {
            goertzel_bass(win, SAMPLE_RATE, f_ref)
        } else {
            goertzel(win, SAMPLE_RATE, f_ref)
        };
        if h == 0 {
            prev = phase;
            continue;
        }
        let mut d = phase - prev - expected;
        d = (d + std::f32::consts::PI).rem_euclid(2.0 * std::f32::consts::PI)
            - std::f32::consts::PI;
        acc += d / (2.0 * std::f32::consts::PI);
        series.push(acc);
        prev = phase;
    }
    fit(&series).1
}

// ─── Unison assist (E6–E9) ──────────────────────────────────────────────────

/// Median of a sample, `NaN` when empty.
fn median(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return f32::NAN;
    }
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

/// The reference the synthetic trials centre on — mid-treble, where the feature
/// lives and where the 1024-sample window is the one that runs.
const SYNTH_REF_HZ: f32 = 440.0;

/// One synthetic string: a partial at `SYNTH_REF_HZ + offset_hz`, decaying.
#[derive(Clone, Copy)]
struct Source {
    offset_hz: f32,
    amplitude: f32,
    tau_secs: f32,
}

/// Deterministic uniform noise in [−0.5, 0.5) — xorshift, no `rand` dependency.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 - 0.5
    }
}

/// Renders `sources` as **audio**, so the trials go through the shipped Goertzel
/// front end rather than a model of it: the analysis window, the decay and the
/// noise are all in the loop. Offsets are measured from `base_hz`.
///
/// `snr_db` is against the **total** source power, which is the probed line's own
/// SNR only when it is the only source; a multi-source trial that wants a stated
/// SNR at one line pre-compensates (see [`e10_bass_null`]).
fn synth_audio(base_hz: f32, sources: &[Source], hops: usize, snr_db: f32, seed: u32) -> Vec<f32> {
    let total = BASS_WINDOW_SIZE + hops * HOP_SIZE;
    let signal_rms = (sources
        .iter()
        .map(|s| s.amplitude * s.amplitude)
        .sum::<f32>()
        / 2.0)
        .sqrt();
    // Uniform noise has variance 1/12, so scale to the requested SNR in power.
    let noise_amp = signal_rms / 10f32.powf(snr_db / 20.0) * 12f32.sqrt();
    let mut noise = Noise(seed | 1);
    (0..total)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let mut x = noise_amp * noise.next();
            for (k, s) in sources.iter().enumerate() {
                let f = base_hz + s.offset_hz;
                let phase = 2.0 * std::f32::consts::PI * f * t + 1.1 * k as f32;
                x += s.amplitude * (-t / s.tau_secs).exp() * phase.sin();
            }
            x
        })
        .collect()
}

/// What one reference published at the end of a run.
#[derive(Clone, Copy, Default)]
struct Resolved {
    count: u8,
    resolution_hz: f32,
    lines: [UnisonLine; MAX_UNISON_LINES],
    /// Hop the record ended on, so E8 can rebuild the exact span it covered.
    hop: usize,
    /// Record length in hops at that point.
    record: usize,
}

/// Drives the shipped bank over `audio` and returns, per reference, the best
/// record it reached — the hop with the longest unbroken record, which is what
/// the panel would be showing when the tuner looks at it.
fn run_unison(
    audio: &[f32],
    refs: &[f32; MAX_STROBE_REFS],
    count: usize,
    spacing_hz: f32,
    noise_floor: f32,
) -> (Vec<Resolved>, UnisonVerdict) {
    let mut strobe = Strobe::new(SAMPLE_RATE);
    strobe.retarget(StrobeRefUpdate {
        count,
        refs: *refs,
        coarse_index: 0, // phase/baseband only; the coarse read is E1–E5's business
        spacing_hz,
    });

    let hops = (audio.len().saturating_sub(BASS_WINDOW_SIZE)) / HOP_SIZE;
    let mut best = vec![Resolved::default(); count];
    let mut verdict = UnisonVerdict::Undetermined;
    let mut frame = tuner_core::pipeline::ProcessingFrame::new();
    let mut record = vec![0usize; count];
    for h in 0..hops {
        frame.audio_buffer[..BASS_WINDOW_SIZE]
            .copy_from_slice(&audio[h * HOP_SIZE..h * HOP_SIZE + BASS_WINDOW_SIZE]);
        let out = strobe.process(&frame, noise_floor, false);
        for i in 0..count {
            // The published resolution is 2·f_hop/L, so it *is* the record length.
            record[i] = if out.line_resolution_hz[i] > 0.0 {
                (2.0 * HOP_RATE_HZ / out.line_resolution_hz[i]).round() as usize
            } else {
                0
            };
            if record[i] > best[i].record {
                best[i] = Resolved {
                    count: out.line_count[i],
                    resolution_hz: out.line_resolution_hz[i],
                    lines: out.lines[i],
                    hop: h,
                    record: record[i],
                };
                verdict = out.verdict;
            }
        }
    }
    (best, verdict)
}

/// One synthetic trial: how many lines the bank resolved, and where.
fn synth_trial(sources: &[Source], hops: usize, snr_db: f32, seed: u32) -> Resolved {
    let audio = synth_audio(SYNTH_REF_HZ, sources, hops, snr_db, seed);
    let mut refs = [0.0f32; MAX_STROBE_REFS];
    refs[0] = SYNTH_REF_HZ;
    let (best, _) = run_unison(&audio, &refs, 1, SYNTH_REF_HZ, 1e-6);
    best[0]
}

/// **E6** — the estimator against synthetic truth: resolution law, accuracy, null.
fn e6_synthetic() {
    const TRIALS: usize = 40;
    let equal = |offset: f32, tau: f32| Source {
        offset_hz: offset,
        amplitude: 1.0,
        tau_secs: tau,
    };

    println!("\n=== E6a: resolution law — P(two lines resolved), 40 trials/cell ===");
    let records = [20usize, 28, 40, 56];
    let splits = [0.7f32, 1.0, 1.5, 2.0, 3.0, 5.0];
    // Cell → (P(2 lines), median reported split). The second is the check the
    // first cannot make: "two lines" is not the same claim as "the right two".
    let mut reported = vec![vec![f32::NAN; records.len()]; splits.len()];
    print!("{:>9}", "split Hz");
    for r in records {
        print!("{:>9.2}s", r as f32 / HOP_RATE_HZ);
    }
    println!();
    for (si, &split) in splits.iter().enumerate() {
        print!("{split:>9.1}");
        for (ri, &hops) in records.iter().enumerate() {
            let mut hits = Vec::new();
            for t in 0..TRIALS {
                let r = synth_trial(
                    &[equal(-split / 2.0, 1.5), equal(split / 2.0, 1.5)],
                    hops,
                    40.0,
                    0x9e37_79b9u32.wrapping_mul(t as u32 + 1),
                );
                if r.count >= 2 {
                    let mut got: Vec<f32> = r.lines[..2].iter().map(|l| l.offset_hz).collect();
                    got.sort_by(f32::total_cmp);
                    hits.push(got[1] - got[0]);
                }
            }
            print!("{:>9.0}%", 100.0 * hits.len() as f32 / TRIALS as f32);
            reported[si][ri] = median(hits);
        }
        println!();
    }
    print!("{:>9}", "2/T floor");
    for r in records {
        print!("{:>9.2} ", 2.0 * HOP_RATE_HZ / r as f32);
    }
    println!("\n(design note E-A: 50 % at ≈2/T, 100 % at ≈1.3–1.4 × 2/T)");

    println!("\n--- and the split it reported where it did resolve (Hz) ---");
    print!("{:>9}", "split Hz");
    for r in records {
        print!("{:>9.2}s", r as f32 / HOP_RATE_HZ);
    }
    println!();
    for (&split, row) in splits.iter().zip(&reported) {
        print!("{split:>9.1}");
        for cell in row {
            if cell.is_nan() {
                print!("{:>10}", "—");
            } else {
                print!("{:>10.2}", cell);
            }
        }
        println!();
    }
    println!(
        "A reported pair is trustworthy only well above 2/T: at 1–2 × the limit the\n\
         separation is inflated (survivorship — only the wide realisations survive the\n\
         merge), and below it a pair can be reported at ≈2/T that is not there at all."
    );

    println!("\n=== E6b: accuracy where it resolves, 56-hop record ===");
    println!(
        "{:>28} {:>7} {:>11} {:>9} {:>9}",
        "case", "P(2)", "split bias", "split σ", "|pos| err"
    );
    let cases: [(&str, [Source; 2], f32); 6] = [
        (
            "equal, 2.0 Hz, SNR 40",
            [equal(-1.0, 1.5), equal(1.0, 1.5)],
            40.0,
        ),
        (
            "equal, 2.0 Hz, SNR 15",
            [equal(-1.0, 1.5), equal(1.0, 1.5)],
            15.0,
        ),
        (
            "equal, 2.0 Hz, SNR 6",
            [equal(-1.0, 1.5), equal(1.0, 1.5)],
            6.0,
        ),
        (
            "second string −20 dB",
            [
                equal(-1.0, 1.5),
                Source {
                    offset_hz: 1.0,
                    amplitude: 0.1,
                    tau_secs: 1.5,
                },
            ],
            40.0,
        ),
        (
            "fast decay τ 0.4 s",
            [equal(-1.0, 0.4), equal(1.0, 0.4)],
            40.0,
        ),
        (
            "split decay 1.5/0.4 s",
            [
                equal(-1.0, 1.5),
                Source {
                    offset_hz: 1.0,
                    amplitude: 1.0,
                    tau_secs: 0.4,
                },
            ],
            40.0,
        ),
    ];
    for (label, sources, snr) in cases {
        let mut splits = Vec::new();
        let mut pos_err = Vec::new();
        let mut hits = 0usize;
        for t in 0..TRIALS {
            let r = synth_trial(&sources, 56, snr, 0x85eb_ca6bu32.wrapping_mul(t as u32 + 1));
            if r.count < 2 {
                continue;
            }
            hits += 1;
            let mut got: Vec<f32> = r.lines[..2].iter().map(|l| l.offset_hz).collect();
            got.sort_by(f32::total_cmp);
            splits.push(got[1] - got[0]);
            let mut want = [sources[0].offset_hz, sources[1].offset_hz];
            want.sort_by(f32::total_cmp);
            pos_err.push(((got[0] - want[0]).abs() + (got[1] - want[1]).abs()) / 2.0);
        }
        let true_split = (sources[1].offset_hz - sources[0].offset_hz).abs();
        let (bias, sigma) = if splits.is_empty() {
            (f32::NAN, f32::NAN)
        } else {
            let mean = splits.iter().sum::<f32>() / splits.len() as f32;
            let var =
                splits.iter().map(|s| (s - mean).powi(2)).sum::<f32>() / splits.len().max(2) as f32;
            (mean - true_split, var.sqrt())
        };
        let pos = if pos_err.is_empty() {
            f32::NAN
        } else {
            pos_err.iter().sum::<f32>() / pos_err.len() as f32
        };
        println!(
            "{label:>28} {:>6.0}% {:>+11.3} {:>9.3} {:>9.3}",
            100.0 * hits as f32 / TRIALS as f32,
            bias,
            sigma,
            pos
        );
    }
    println!("(design note E-B: bias ≤ 0.02 Hz, σ ≤ 0.06 Hz, sensitivity to −26 dB)");

    println!("\n=== E6d: the level/separation surface — P(two lines), 56-hop record ===");
    println!(
        "Separation is in units of the record's own 2/T ({:.2} Hz at 56 hops), because that\n\
         is the axis the panel states. The second string is attenuated down each column.\n",
        2.0 * HOP_RATE_HZ / 56.0
    );
    let ratios = [1.0f32, 1.3, 1.6, 2.0, 2.6, 3.3];
    let levels_db = [0.0f32, -6.0, -12.0, -20.0, -26.0];
    print!("{:>10}", "2nd level");
    for r in ratios {
        print!("{r:>8.1}×");
    }
    println!();
    for db in levels_db {
        print!("{db:>8.0} dB");
        for r in ratios {
            let split = r * 2.0 * HOP_RATE_HZ / 56.0;
            let hits = (0..TRIALS)
                .filter(|t| {
                    synth_trial(
                        &[
                            equal(-split / 2.0, 1.5),
                            Source {
                                offset_hz: split / 2.0,
                                amplitude: 10f32.powf(db / 20.0),
                                tau_secs: 1.5,
                            },
                        ],
                        56,
                        40.0,
                        0x1656_67b1u32.wrapping_mul(*t as u32 + 1),
                    )
                    .count
                        >= 2
                })
                .count();
            print!("{:>8.0}%", 100.0 * hits as f32 / TRIALS as f32);
        }
        println!();
    }
    println!(
        "The resolution the panel states is a *geometric* limit. Where this surface is 0 %\n\
         above 1.0×, the limit is dynamic range instead, and the panel does not say so."
    );

    println!("\n=== E6c: the null — one string must never report two ===");
    println!("{:>26} {:>9} {:>9}", "case", "@30 hops", "@56 hops");
    for (label, tau, snr) in [
        ("τ 1.5 s, SNR 40", 1.5f32, 40.0f32),
        ("τ 0.4 s, SNR 40", 0.4, 40.0),
        ("τ 1.5 s, SNR 15", 1.5, 15.0),
        ("τ 1.5 s, SNR 6", 1.5, 6.0),
        ("τ 0.4 s, SNR 15", 0.4, 15.0),
    ] {
        print!("{label:>26}");
        for hops in [30usize, 56] {
            let false_splits = (0..TRIALS)
                .filter(|t| {
                    synth_trial(
                        &[equal(0.7, tau)],
                        hops,
                        snr,
                        0xc2b2_ae35u32.wrapping_mul(*t as u32 + 1),
                    )
                    .count
                        >= 2
                })
                .count();
            print!("{:>9.0}%", 100.0 * false_splits as f32 / TRIALS as f32);
        }
        println!();
    }
    println!("(design note E-U: 0 % at the 1024-sample window; the 4096 one broke at 45 %)");
}

// ─── The bass configuration (E10) ───────────────────────────────────────────

/// Where content `delta_hz` from a reference lands in the baseband. One Goertzel
/// output per hop *is* a decimation to `HOP_RATE_HZ`, so anything the analysis
/// window admits folds into ±f_hop/2 — it never leaves the band, it only moves
/// inside it (design note E-Q).
fn fold_hz(delta_hz: f32) -> f32 {
    let wrapped = delta_hz.rem_euclid(HOP_RATE_HZ);
    if wrapped > 0.5 * HOP_RATE_HZ {
        wrapped - HOP_RATE_HZ
    } else {
        wrapped
    }
}

/// A first reference low enough that the bank's R3 rule selects the 4096-sample
/// window (the boundary is `2·f_s/1024` ≈ 86 Hz), carrying no synthetic content
/// of its own. It makes the window an A/B at any probed frequency: the bank
/// keys the choice on `refs[0]`, and every reference is evaluated independently.
const R3_FORCING_HZ: f32 = 50.0;

/// One key's stiff-string series `f_n = n·f₀·√(1+Bn²)`, and the partial the
/// panel displays for it.
///
/// `B` is the Rigaud medium **prior** at the key, never a capture's own
/// `calculated_b`: the standing rule is that the synthetic generator is not
/// recalibrated to the engine's measurements.
struct KeyRefs {
    f1: f32,
    b: f32,
    partials: [f32; MAX_STROBE_REFS],
    probe_n: usize,
}

impl KeyRefs {
    fn for_key(key: usize) -> Self {
        let b = tuner_core::algorithms::rigaud::BXi::DEFAULT_MEDIUM.b_at_key(key) as f32;
        let f1 = NOTES[key].frequency;
        let f0 = f1 / (1.0 + b).sqrt();
        let mut partials = [0.0f32; MAX_STROBE_REFS];
        for (i, r) in partials.iter_mut().enumerate() {
            let n = (i + 1) as f32;
            *r = n * f0 * (1.0 + b * n * n).sqrt();
        }
        Self {
            f1,
            b,
            partials,
            probe_n: default_display_partials()[key] as usize,
        }
    }

    fn probe_hz(&self) -> f32 {
        self.partials[self.probe_n - 1]
    }

    /// Partial spacing at the probed partial — what has to clear the analysis
    /// window's main lobe for the neighbours not to leak in.
    fn spacing_hz(&self) -> f32 {
        let n = self.probe_n;
        if n >= MAX_STROBE_REFS {
            self.partials[n - 1] - self.partials[n - 2]
        } else {
            self.partials[n] - self.partials[n - 1]
        }
    }

    /// One string's whole partial series as sources, offsets measured from the
    /// probed partial. Equal amplitudes: the deep bass radiates its fundamental
    /// *worse* than its upper partials (ADR 0011 §"mechanism" — the n = 1
    /// competitor runs 1.3–1.8 × the target at B0–C#1), so a flat series is not
    /// a pessimistic choice there, and it is the leakage case the R3 window
    /// exists for.
    fn series(&self, tau: f32) -> Vec<Source> {
        self.partials
            .iter()
            .map(|f| Source {
                offset_hz: f - self.probe_hz(),
                amplitude: 1.0,
                tau_secs: tau,
            })
            .collect()
    }
}

/// What the probed frequency resolves under one window, with the reference set
/// shaped to select it: the probe alone (1024), or the probe behind
/// [`R3_FORCING_HZ`] (4096, the shipped deep-bass path).
fn probe_lines(audio: &[f32], probe_hz: f32, long: bool) -> Resolved {
    let mut refs = [0.0f32; MAX_STROBE_REFS];
    if long {
        refs[0] = R3_FORCING_HZ;
        refs[1] = probe_hz;
        run_unison(audio, &refs, 2, refs[0], 1e-6).0[1]
    } else {
        refs[0] = probe_hz;
        run_unison(audio, &refs, 1, probe_hz, 1e-6).0[0]
    }
}

/// SNR against the *total* source power that puts the probed line — source 0 —
/// at `want_db`. [`synth_audio`] scales its noise to every source together.
fn total_snr_for(sources: &[Source], want_db: f32) -> f32 {
    let total: f32 = sources.iter().map(|s| s.amplitude * s.amplitude).sum();
    want_db + 10.0 * (total / sources[0].amplitude.powi(2)).log10()
}

/// The demodulated baseband of a **noise-only** input at `f_ref`, formed exactly
/// as the bank forms it: one Goertzel per hop over the newest window, its
/// reference rotation removed.
fn baseband_noise(f_ref: f32, long: bool, hops: usize, seed: u32) -> Vec<Complex<f32>> {
    let mut noise = Noise(seed | 1);
    let audio: Vec<f32> = (0..BASS_WINDOW_SIZE + hops * HOP_SIZE)
        .map(|_| noise.next())
        .collect();
    let t_hop = HOP_SIZE as f32 / SAMPLE_RATE as f32;
    (0..hops)
        .map(|h| {
            let win = &audio[h * HOP_SIZE..h * HOP_SIZE + BASS_WINDOW_SIZE];
            let (amp, phase) = if long {
                goertzel_bass(win, SAMPLE_RATE, f_ref)
            } else {
                goertzel(win, SAMPLE_RATE, f_ref)
            };
            let turn = phase - 2.0 * std::f32::consts::PI * f_ref * t_hop * h as f32;
            Complex::from_polar(amp, turn)
        })
        .collect()
}

/// **E10** — the null, the resolution law and the noise correlation in the
/// **bass** configuration, which every synthetic trial before this one skipped.
///
/// The deep bass runs `goertzel_bass` (R3) against the same 1024-sample hop, so
/// its baseband is 4× oversampled and consecutive samples share three quarters
/// of their input. Correlated reference cells are exactly what an OS-CFAR
/// threshold assumes away, so the shipped null — measured only at the treble's
/// critically-sampled 1024 window — may not hold there at all. That is the first
/// candidate explanation for the bass second lines (ADR 0012 §5, Prompt T).
fn e10_bass_null() {
    const TRIALS: usize = 40;
    /// E1 — inside the 0–27 band whose second lines this is about, and well
    /// under the R3 boundary, so its shipped path is the 4096-sample window.
    const BASS_KEY: usize = 7;
    let cfg = KeyRefs::for_key(BASS_KEY);
    let probe = cfg.probe_hz();
    println!("\n=== E10: the bass (4096-sample window) configuration ===");
    println!(
        "reference set: key {BASS_KEY} ({}), f₁ = {:.2} Hz, B = {:.2e} (Rigaud medium prior),\n\
         probed at the displayed partial n = {} → {:.2} Hz, spacing there {:.1} Hz.  The A/B is\n\
         the *same audio* through two reference sets differing only in what selects the\n\
         window: the probe alone (1024) and the probe behind a {R3_FORCING_HZ:.0} Hz first reference (4096).",
        NOTES[BASS_KEY].name,
        cfg.f1,
        cfg.b,
        cfg.probe_n,
        probe,
        cfg.spacing_hz(),
    );

    println!("\n--- E10a: the null — one string must never report two ---");
    println!(
        "{:>34} {:>9} {:>9} {:>9} {:>9}",
        "case", "1024 @30", "1024 @56", "4096 @30", "4096 @56"
    );
    for (label, series, snr) in [
        ("isolated line, SNR 40", false, 40.0f32),
        ("isolated line, SNR 15", false, 15.0),
        ("isolated line, SNR 6", false, 6.0),
        ("whole partial series, SNR 40", true, 40.0),
        ("whole partial series, SNR 15", true, 15.0),
        ("whole partial series, SNR 6", true, 6.0),
    ] {
        let sources = if series {
            cfg.series(1.5)
        } else {
            vec![Source {
                offset_hz: 0.0,
                amplitude: 1.0,
                tau_secs: 1.5,
            }]
        };
        let snr_total = total_snr_for(&sources, snr);
        let mut hits = [[0usize; 2]; 2]; // [window][record]
        for (ri, &hops) in [30usize, 56].iter().enumerate() {
            for t in 0..TRIALS {
                let audio = synth_audio(
                    probe,
                    &sources,
                    hops,
                    snr_total,
                    0x27d4_eb2fu32.wrapping_mul(t as u32 + 1),
                );
                hits[0][ri] += usize::from(probe_lines(&audio, probe, false).count >= 2);
                hits[1][ri] += usize::from(probe_lines(&audio, probe, true).count >= 2);
            }
        }
        print!("{label:>34}");
        for window in &hits {
            for record in window {
                print!("{:>9.0}%", 100.0 * *record as f32 / TRIALS as f32);
            }
        }
        println!();
    }
    println!(
        "SNR is stated at the *probed* line in every row, so the two halves of the table\n\
         differ only in the presence of the other eleven partials — spacing {:.1} Hz, i.e.\n\
         inside the 1024-sample window's main lobe (±86 Hz) and outside the 4096 one's\n\
         (±21.5 Hz).  A partial that leaks in folds: the baseband is sampled at {:.2} Hz.",
        cfg.spacing_hz(),
        HOP_RATE_HZ
    );

    println!("\n--- E10b: does a genuine pair still resolve there? 56-hop record ---");
    println!(
        "{:>10} {:>10} {:>12} {:>10} {:>12}",
        "split Hz", "1024 P(2)", "1024 split", "4096 P(2)", "4096 split"
    );
    for split in [0.7f32, 1.0, 1.5, 2.0, 3.0, 5.0] {
        let sources = vec![
            Source {
                offset_hz: -split / 2.0,
                amplitude: 1.0,
                tau_secs: 1.5,
            },
            Source {
                offset_hz: split / 2.0,
                amplitude: 1.0,
                tau_secs: 1.5,
            },
        ];
        let mut got = [Vec::new(), Vec::new()];
        for t in 0..TRIALS {
            let audio = synth_audio(
                probe,
                &sources,
                56,
                40.0,
                0x1656_67b1u32.wrapping_mul(t as u32 + 1),
            );
            for (w, long) in [false, true].iter().enumerate() {
                let r = probe_lines(&audio, probe, *long);
                if r.count >= 2 {
                    let mut o: Vec<f32> = r.lines[..2].iter().map(|l| l.offset_hz).collect();
                    o.sort_by(f32::total_cmp);
                    got[w].push(o[1] - o[0]);
                }
            }
        }
        println!(
            "{split:>10.1} {:>9.0}% {:>12.2} {:>9.0}% {:>12.2}",
            100.0 * got[0].len() as f32 / TRIALS as f32,
            median(got[0].clone()),
            100.0 * got[1].len() as f32 / TRIALS as f32,
            median(got[1].clone()),
        );
    }

    println!("\n--- E10d: how far up the compass does neighbour leakage reach? ---");
    println!(
        "One string, whole partial series, 56-hop record, SNR 40 at the probed partial —\n\
         the E10a 'series' case swept across the compass. `R3` marks the window the shipped\n\
         rule picks for that key. 'rel amp' is the strongest false line's amplitude relative\n\
         to the true one, which is the strength E-Q measured only the *presence* of.\n"
    );
    println!(
        "{:>4} {:>5} {:>7} {:>3} {:>8} {:>7} {:>5} {:>9} {:>7} {:>9} {:>7} {:>8}",
        "key",
        "note",
        "f₁ Hz",
        "n*",
        "spacing",
        "folds to",
        "R3",
        "1024 @40",
        "@15",
        "4096 @40",
        "@15",
        "rel amp"
    );
    for key in [7usize, 12, 19, 24, 26, 27, 31, 36, 43, 48] {
        let k = KeyRefs::for_key(key);
        let p = k.probe_hz();
        let sources = k.series(1.5);
        let mut hits = [[0usize; 2]; 2]; // [window][snr]
        let mut rel = Vec::new();
        for (si, snr) in [40.0f32, 15.0].iter().enumerate() {
            let snr_total = total_snr_for(&sources, *snr);
            for t in 0..TRIALS {
                let audio = synth_audio(
                    p,
                    &sources,
                    56,
                    snr_total,
                    0x7feb_352du32.wrapping_mul(t as u32 + 1),
                );
                for (w, long) in [false, true].iter().enumerate() {
                    let r = probe_lines(&audio, p, *long);
                    if r.count >= 2 {
                        hits[w][si] += 1;
                        if !*long && si == 0 {
                            rel.push(r.lines[1].relative_amplitude);
                        }
                    }
                }
            }
        }
        let pct = |n: usize| 100.0 * n as f32 / TRIALS as f32;
        println!(
            "{key:>4} {:>5} {:>7.1} {:>3} {:>8.1} {:>+7.1} {:>5} {:>8.0}% {:>6.0}% {:>8.0}% {:>6.0}% {:>8.3}",
            NOTES[key].name,
            k.f1,
            k.probe_n,
            k.spacing_hz(),
            fold_hz(k.spacing_hz()),
            if k.f1 * 1024.0 < 2.0 * SAMPLE_RATE as f32 {
                "4096"
            } else {
                "1024"
            },
            pct(hits[0][0]),
            pct(hits[0][1]),
            pct(hits[1][0]),
            pct(hits[1][1]),
            median(rel.clone()),
        );
    }
    println!(
        "'folds to' is where the next partial up lands after decimation: an offset wraps\n\
         into ±{:.2} Hz, so a neighbour never leaves the baseband, it only moves inside it.",
        0.5 * HOP_RATE_HZ
    );

    println!("\n--- E10e: the folded interferer's *strength* (the open E-Q question) ---");
    println!(
        "One true line at the probe, one equal-amplitude interferer δ Hz above it, SNR 40,\n\
         56-hop record. E-Q confirmed a fold appears; what decides whether it out-ranks a\n\
         genuine string is its amplitude, which was never measured.  Note the 1024-sample\n\
         window's bin width is f_s/1024 = {:.2} Hz — the hop rate itself — so its Hann nulls\n\
         land exactly on the offsets that fold to zero, which is why δ = 43 and 86 are empty\n\
         rows.  Everywhere else there is no such protection.\n",
        SAMPLE_RATE as f32 / 1024.0
    );
    println!(
        "{:>8} {:>10} {:>11} {:>10} {:>11} {:>10}",
        "δ Hz", "folds to", "1024 spur", "1024 amp", "4096 spur", "4096 amp"
    );
    for delta in [
        5.0f32, 10.0, 15.0, 21.0, 30.0, 43.0, 55.0, 65.0, 86.0, 110.0, 150.0,
    ] {
        let sources = [
            Source {
                offset_hz: 0.0,
                amplitude: 1.0,
                tau_secs: 1.5,
            },
            Source {
                offset_hz: delta,
                amplitude: 1.0,
                tau_secs: 1.5,
            },
        ];
        let want = fold_hz(delta);
        let mut hits = [0usize; 2];
        let mut amps = [Vec::new(), Vec::new()];
        for t in 0..TRIALS {
            let audio = synth_audio(
                probe,
                &sources,
                56,
                40.0,
                0x2545_f491u32.wrapping_mul(t as u32 + 1),
            );
            for (w, long) in [false, true].iter().enumerate() {
                let r = probe_lines(&audio, probe, *long);
                // The spurious line is the one at the predicted fold, not merely
                // a second line: a fold within 2 bins of 0 is the target itself.
                if let Some(l) = r.lines[..r.count as usize]
                    .iter()
                    .find(|l| (l.offset_hz - want).abs() < 1.0 && want.abs() > r.resolution_hz)
                {
                    hits[w] += 1;
                    amps[w].push(l.relative_amplitude);
                }
            }
        }
        let pct = |n: usize| 100.0 * n as f32 / TRIALS as f32;
        let db = |v: Vec<f32>| {
            let m = median(v);
            if m.is_nan() {
                f32::NAN
            } else {
                20.0 * m.log10()
            }
        };
        println!(
            "{delta:>8.0} {:>+10.1} {:>10.0}% {:>9.0}dB {:>10.0}% {:>9.0}dB",
            want,
            pct(hits[0]),
            db(amps[0].clone()),
            pct(hits[1]),
            db(amps[1].clone()),
        );
    }

    println!("\n--- E10c: the mechanism — baseband noise correlation ---");
    println!(
        "A 4096-sample window advanced by a 1024-sample hop overlaps 75 %, so consecutive\n\
         baseband samples are correlated and the record carries fewer independent samples\n\
         than it has bins. ρ_k is the complex correlation coefficient at lag k, pooled over\n\
         64 noise-only runs of {UNISON_RING_HOPS} hops; N/N_eff is the variance inflation of an average\n\
         over the record, 1 + 2·Σ(1 − k/N)·|ρ_k|² (Welch 1967).  |ρ̂| of independent samples\n\
         does not estimate 0 but ≈1/√N = {:.3}, which is the floor to read the table against.",
        1.0 / (UNISON_RING_HOPS as f32).sqrt()
    );
    println!(
        "\n{:>10} {:>9} {:>9} {:>9} {:>9} {:>10}",
        "window", "ρ_1", "ρ_2", "ρ_3", "ρ_4", "N/N_eff"
    );
    for (label, long) in [("1024", false), ("4096", true)] {
        let mut rho = [0.0f64; 4];
        let runs = 64;
        for seed in 0..runs {
            let z = baseband_noise(probe, long, UNISON_RING_HOPS, 0x9e37_79b9 * (seed + 1));
            let power: f32 = z.iter().map(|x| x.norm_sqr()).sum();
            for (k, r) in rho.iter_mut().enumerate() {
                let lag = k + 1;
                let c: Complex<f32> = z[..z.len() - lag]
                    .iter()
                    .zip(&z[lag..])
                    .map(|(a, b)| a.conj() * b)
                    .sum();
                *r += (c.norm() / power) as f64;
            }
        }
        let rho: Vec<f32> = rho.iter().map(|r| (r / runs as f64) as f32).collect();
        let n = UNISON_RING_HOPS as f32;
        let inflation = 1.0
            + 2.0
                * rho
                    .iter()
                    .enumerate()
                    .map(|(k, r)| (1.0 - (k + 1) as f32 / n) * r * r)
                    .sum::<f32>();
        println!(
            "{label:>10} {:>9.3} {:>9.3} {:>9.3} {:>9.3} {:>10.2}",
            rho[0], rho[1], rho[2], rho[3], inflation
        );
    }
}

/// The measured stiff-string reference set for a capture, from `analysis.json`'s
/// own partial list. Returns the count written.
fn measured_refs(cap: &Capture, out: &mut [f32; MAX_STROBE_REFS]) -> usize {
    let mut count = 0;
    for n in 1..=MAX_STROBE_REFS {
        match cap.partial_hz[n] {
            Some(f) if f > 0.0 && f < SAMPLE_RATE as f32 / 2.0 => {
                out[n - 1] = f;
                count = n;
            }
            _ => break,
        }
    }
    count
}

/// Register label used by the unison summaries. The bass is reported separately
/// throughout: it produces a second line on essentially every capture of both
/// instruments, including on single-strung keys, and those lines are real
/// spectral content that is not a second string (ADR 0012 §5, ADR 0013).
fn register(key: u8) -> &'static str {
    match key {
        0..=27 => "bass",
        28..=51 => "tenor",
        52..=75 => "treble",
        _ => "high 76–87",
    }
}

/// **E7** — availability per register, and the truth-free reproducibility test.
fn e7_real(captures: &[(Capture, Vec<Resolved>, UnisonVerdict, usize)]) {
    let table = default_display_partials();

    println!("\n=== E7a: availability at the displayed partial n* (what the panel shows) ===");
    println!(
        "{:>20} {:>9} {:>7} {:>6} {:>6} {:>10} {:>9}",
        "register", "captures", "≥1 line", "≥2", "≥3", "median T", "med 2/T"
    );
    for band in ["bass", "tenor", "treble", "high 76–87"] {
        let rows: Vec<&Resolved> = captures
            .iter()
            .filter(|(c, ..)| register(c.key) == band)
            .filter_map(|(c, r, ..)| r.get(table[c.key as usize] as usize - 1))
            .collect();
        if rows.is_empty() {
            continue;
        }
        let n = rows.len() as f32;
        let frac = |k: u8| 100.0 * rows.iter().filter(|r| r.count >= k).count() as f32 / n;
        let published: Vec<&&Resolved> = rows.iter().filter(|r| r.record > 0).collect();
        println!(
            "{band:>20} {:>9} {:>6.0}% {:>5.0}% {:>5.0}% {:>9.2}s {:>8.2}Hz",
            rows.len(),
            frac(1),
            frac(2),
            frac(3),
            median(
                published
                    .iter()
                    .map(|r| r.record as f32 / HOP_RATE_HZ)
                    .collect()
            ),
            median(published.iter().map(|r| r.resolution_hz).collect()),
        );
    }

    println!("\n=== E7a′: the same over every reference in the bank ===");
    println!(
        "{:>20} {:>6} {:>7} {:>6} {:>6} {:>10} {:>9}",
        "register", "refs", "≥1 line", "≥2", "≥3", "median T", "med 2/T"
    );
    for band in ["bass", "tenor", "treble", "high 76–87"] {
        let rows: Vec<&Resolved> = captures
            .iter()
            .filter(|(c, ..)| register(c.key) == band)
            .flat_map(|(_, r, ..)| r.iter())
            .collect();
        if rows.is_empty() {
            continue;
        }
        let n = rows.len() as f32;
        let frac = |k: u8| 100.0 * rows.iter().filter(|r| r.count >= k).count() as f32 / n;
        let published: Vec<&&Resolved> = rows.iter().filter(|r| r.record > 0).collect();
        let secs = median(
            published
                .iter()
                .map(|r| r.record as f32 / HOP_RATE_HZ)
                .collect(),
        );
        let res = median(published.iter().map(|r| r.resolution_hz).collect());
        println!(
            "{band:>20} {:>6} {:>6.0}% {:>5.0}% {:>5.0}% {:>9.2}s {:>8.2}Hz",
            rows.len(),
            frac(1),
            frac(2),
            frac(3),
            secs,
            res
        );
    }

    println!("\n=== E7b: repeat reproducibility — independent strikes must agree ===");
    println!(
        "{:>20} {:>6} {:>13} {:>13} {:>10}",
        "register", "keys", "median split", "median MAD", "relative"
    );
    // Group by key, take each strike's split at the displayed partial.
    for band in ["bass", "tenor", "treble"] {
        let mut per_key: Vec<(u8, Vec<f32>)> = Vec::new();
        for (cap, resolved, _, _) in captures.iter().filter(|(c, ..)| register(c.key) == band) {
            let n_star = table[cap.key as usize] as usize;
            let Some(r) = resolved.get(n_star.saturating_sub(1)) else {
                continue;
            };
            let Some(f_ref) = cap.partial_hz[n_star] else {
                continue;
            };
            if r.count < 2 || f_ref <= 0.0 {
                continue;
            }
            let split = (r.lines[1].offset_hz - r.lines[0].offset_hz).abs();
            let cents = 1200.0 * (1.0 + split / f_ref).log2();
            match per_key.iter_mut().find(|(k, _)| *k == cap.key) {
                Some((_, v)) => v.push(cents),
                None => per_key.push((cap.key, vec![cents])),
            }
        }
        let repeats: Vec<(f32, f32)> = per_key
            .iter()
            .filter(|(_, v)| v.len() >= 3)
            .map(|(_, v)| {
                let m = median(v.clone());
                let mad = median(v.iter().map(|x| (x - m).abs()).collect());
                (m, mad)
            })
            .collect();
        if repeats.is_empty() {
            continue;
        }
        let med_split = median(repeats.iter().map(|r| r.0).collect());
        let med_mad = median(repeats.iter().map(|r| r.1).collect());
        println!(
            "{band:>20} {:>6} {:>11.2} ¢ {:>11.2} ¢ {:>9.0}%",
            repeats.len(),
            med_split,
            med_mad,
            100.0 * med_mad / med_split.max(1e-6)
        );
    }

    println!("\n=== E7c: the discriminator's verdict, per register ===");
    println!(
        "{:>20} {:>9} {:>9} {:>9} {:>14}",
        "register", "captures", "unison", "false beat", "undetermined"
    );
    for band in ["bass", "tenor", "treble", "high 76–87"] {
        let rows: Vec<UnisonVerdict> = captures
            .iter()
            .filter(|(c, ..)| register(c.key) == band)
            .map(|(_, _, v, _)| *v)
            .collect();
        if rows.is_empty() {
            continue;
        }
        let n = rows.len() as f32;
        let frac =
            |want: UnisonVerdict| 100.0 * rows.iter().filter(|v| **v == want).count() as f32 / n;
        println!(
            "{band:>20} {:>9} {:>8.0}% {:>8.0}% {:>13.0}%",
            rows.len(),
            frac(UnisonVerdict::Unison),
            frac(UnisonVerdict::FalseBeat),
            frac(UnisonVerdict::Undetermined)
        );
    }
}

/// The full-rate view of one baseband's span: every local maximum inside the
/// band, refined sub-bin, and the magnitude spectrum behind them.
///
/// The peak list is **uncapped** — the design note's own reference picker was
/// capped at three peaks, and lifting it to five alone halved its unmatched rate,
/// so a cap here would beg the question E8 asks.
struct Reference {
    /// (frequency Hz, magnitude) of every local maximum in the band.
    peaks: Vec<(f32, f32)>,
    magnitudes: Vec<f32>,
    hz_per_bin: f32,
    /// Median magnitude across the band — the scale "is there energy here" is
    /// judged against.
    band_median: f32,
}

impl Reference {
    fn of(span: &[f32], centre_hz: f32, half_hz: f32) -> Self {
        let n = span.len();
        let mut planner = RealFftPlanner::<f32>::new();
        let r2c = planner.plan_fft_forward(n);
        let mut time = vec![0.0f32; n];
        let mut spec = vec![Complex { re: 0.0, im: 0.0 }; n / 2 + 1];
        spectral::fft(span, &mut time, &mut spec, &r2c, n);
        let mut magnitudes = vec![0.0f32; n / 2];
        spectral::magnitude_spectrum(&spec, n, &mut magnitudes);

        let hz_per_bin = SAMPLE_RATE as f32 / n as f32;
        let lo = (((centre_hz - half_hz) / hz_per_bin).floor().max(1.0)) as usize;
        let hi = ((((centre_hz + half_hz) / hz_per_bin).ceil()) as usize).min(magnitudes.len() - 2);
        let mut peaks = Vec::new();
        if lo < hi {
            for bin in lo..=hi {
                if magnitudes[bin] > magnitudes[bin - 1] && magnitudes[bin] > magnitudes[bin + 1] {
                    peaks.push((
                        spectral::jacobsen(&spec, bin, n, SAMPLE_RATE),
                        magnitudes[bin],
                    ));
                }
            }
        }
        let band_median = if lo < hi {
            median(magnitudes[lo..=hi].to_vec())
        } else {
            f32::NAN
        };
        Self {
            peaks,
            magnitudes,
            hz_per_bin,
            band_median,
        }
    }

    /// Distance from `hz` to the nearest reference peak.
    fn nearest(&self, hz: f32) -> f32 {
        self.peaks
            .iter()
            .map(|(p, _)| (p - hz).abs())
            .fold(f32::INFINITY, f32::min)
    }

    /// The magnitude carried at `hz` over the band median — "is there energy
    /// there", asked without a peak picker.
    fn excess(&self, hz: f32) -> f32 {
        let at = ((hz / self.hz_per_bin).round() as usize).min(self.magnitudes.len() - 1);
        if self.band_median > 0.0 {
            self.magnitudes[at] / self.band_median
        } else {
            f32::NAN
        }
    }
}

/// **E8** — the unexplained-line investigation.
fn e8_unexplained(captures: &[(Capture, Vec<Resolved>, UnisonVerdict, usize)]) {
    println!("\n=== E8: reported lines vs a full-rate DFT of the identical span ===");
    println!(
        "the zoom and the reference see the same seconds, so this validates the\n\
         implementation, not the accuracy. Matched = a reference peak within 0.5 Hz."
    );
    /// One (register, rank) cell of the tally.
    struct Cell {
        band: &'static str,
        rank: usize,
        total: usize,
        unmatched: usize,
        /// Distance from each line to the nearest reference peak, Hz.
        distance: Vec<f32>,
        /// For the unmatched only: reference magnitude there over the band median.
        excess: Vec<f32>,
        /// Distance to this line's nearest sibling, in units of `2/T`.
        separation: Vec<f32>,
        /// The same, for the unmatched only.
        unmatched_separation: Vec<f32>,
    }
    let mut rows: Vec<Cell> = Vec::new();
    for (cap, resolved, _, window) in captures {
        let band = register(cap.key);
        for (i, r) in resolved.iter().enumerate() {
            if r.count == 0 || r.record < 2 {
                continue;
            }
            let Some(f_ref) = cap.partial_hz[i + 1] else {
                continue;
            };
            // The samples this record actually integrated: hop h's Goertzel reads
            // the newest `window` of the 8192 buffer ending at (h+1)·HOP + 8192.
            let end = r.hop * HOP_SIZE + BASS_WINDOW_SIZE;
            let start = (r.hop + 1 - r.record) * HOP_SIZE + BASS_WINDOW_SIZE - window;
            if end > cap.audio.len() || start >= end {
                continue;
            }
            let reference = Reference::of(&cap.audio[start..end], f_ref, ALIAS_HZ);
            for rank in 0..r.count as usize {
                let hz = f_ref + r.lines[rank].offset_hz;
                let nearest = reference.nearest(hz);
                // How close this line's own nearest neighbour is, in units of the
                // record's resolution: the hypothesis is that unmatched lines sit
                // where a pair is only just separated.
                let partner = (0..r.count as usize)
                    .filter(|k| *k != rank)
                    .map(|k| (r.lines[k].offset_hz - r.lines[rank].offset_hz).abs())
                    .fold(f32::INFINITY, f32::min)
                    / r.resolution_hz;
                let row = match rows.iter().position(|x| x.band == band && x.rank == rank) {
                    Some(k) => &mut rows[k],
                    None => {
                        rows.push(Cell {
                            band,
                            rank,
                            total: 0,
                            unmatched: 0,
                            distance: Vec::new(),
                            excess: Vec::new(),
                            separation: Vec::new(),
                            unmatched_separation: Vec::new(),
                        });
                        rows.last_mut().expect("just pushed")
                    }
                };
                row.total += 1;
                row.distance.push(nearest);
                if partner.is_finite() {
                    row.separation.push(partner);
                }
                if nearest > 0.5 {
                    row.unmatched += 1;
                    row.excess.push(reference.excess(hz));
                    if partner.is_finite() {
                        row.unmatched_separation.push(partner);
                    }
                }
            }
        }
    }
    rows.sort_by_key(|r| (r.band, r.rank));

    println!(
        "\n{:>20} {:>5} {:>7} {:>10} {:>10} {:>11} {:>11} {:>9} {:>13}",
        "register",
        "rank",
        "lines",
        "unmatched",
        "med |d| Hz",
        "med excess",
        "excess > 2",
        "sep /(2/T)",
        "unmatched sep"
    );
    for c in &rows {
        let ex = if c.excess.is_empty() {
            f32::NAN
        } else {
            median(c.excess.clone())
        };
        let strong = if c.excess.is_empty() {
            f32::NAN
        } else {
            100.0 * c.excess.iter().filter(|x| **x > 2.0).count() as f32 / c.excess.len() as f32
        };
        println!(
            "{:>20} {:>5} {:>7} {:>9.1}% {:>10.3} {:>11.1} {:>10.0}% {:>9.2} {:>13.2}",
            c.band,
            c.rank + 1,
            c.total,
            100.0 * c.unmatched as f32 / c.total as f32,
            median(c.distance.clone()),
            ex,
            strong,
            median(c.separation.clone()),
            median(c.unmatched_separation.clone())
        );
    }
    println!(
        "\n'excess' is the reference DFT's magnitude at the reported frequency over the\n\
         local median of its own band: > 2 means real spectral energy the reference's\n\
         peak picker did not list, i.e. the line is there and the *reference* missed it.\n\
         'sep' is each line's distance to its nearest sibling in units of the record's\n\
         own resolution 2/T — if the unmatched ones sit nearer the limit than the\n\
         matched ones, they are shoulders of a barely-separated pair, not phantoms."
    );
}

// ─── Bass attribution (E11–E12) ─────────────────────────────────────────────

/// A reported line that is not the strongest at its reference — what Prompt T is
/// about. The bass produces one on essentially every capture of both
/// instruments, including on single-strung keys, and what they are is
/// unestablished (ADR 0012 §5).
struct Extra {
    key: u8,
    /// Partial number of the reference it sits on.
    partial: usize,
    f_ref: f32,
    offset_hz: f32,
    /// Analysis window that produced it — what the front end could admit at all,
    /// its main lobe being ±2·f_s/N.
    window: usize,
    /// `2/T` of the record it came from — the floor its offset must clear
    /// before the *position* means anything (ADR 0012 §4).
    resolution_hz: f32,
}

impl Extra {
    fn abs_hz(&self) -> f32 {
        self.f_ref + self.offset_hz
    }

    /// Half the analysis window's Hann main lobe: content further out than this
    /// enters through sidelobes only, at −34 dB or worse (E10e).
    fn lobe_hz(&self) -> f32 {
        2.0 * SAMPLE_RATE as f32 / self.window as f32
    }
}

/// Every extra line the bank reported, over all captures.
fn extras(captures: &[(Capture, Vec<Resolved>, UnisonVerdict, usize)]) -> Vec<Extra> {
    let mut out = Vec::new();
    for (cap, resolved, _, window) in captures {
        for (i, r) in resolved.iter().enumerate() {
            let Some(f_ref) = cap.partial_hz[i + 1] else {
                continue;
            };
            for rank in 1..r.count as usize {
                out.push(Extra {
                    key: cap.key,
                    partial: i + 1,
                    f_ref,
                    offset_hz: r.lines[rank].offset_hz,
                    window: *window,
                    resolution_hz: r.resolution_hz,
                });
            }
        }
    }
    out
}

/// The instrument's own partial layout, one row per key — where a *neighbouring*
/// key's partials sit, which is what the sympathetic-resonance candidate
/// predicts. Built from the set's own captures and extended past the twelve
/// measured partials by the stiff-string law.
struct KeyTable {
    f0: [f32; 88],
    b: [f32; 88],
}

impl KeyTable {
    fn of(captures: &[(Capture, Vec<Resolved>, UnisonVerdict, usize)]) -> Self {
        let mut f0 = [0.0f32; 88];
        let mut b = [0.0f32; 88];
        for key in 0..88 {
            let rows: Vec<&Capture> = captures
                .iter()
                .map(|(c, ..)| c)
                .filter(|c| c.key as usize == key && c.f0 > 0.0)
                .collect();
            if rows.is_empty() {
                continue;
            }
            f0[key] = median(rows.iter().map(|c| c.f0).collect());
            b[key] = median(rows.iter().map(|c| c.b).collect());
        }
        Self { f0, b }
    }

    /// The **phantom partials** that land near this key's partial `n`: Conklin's
    /// nonlinear mixing products `fᵢ + fⱼ` with `i + j = n` and `2fᵢ − fⱼ` with
    /// `2i − j = n`. They sit *below* the transverse partial, because the
    /// transverse series is stretched by inharmonicity while a mixing product
    /// adds linearly — `f_n − (fᵢ + fⱼ) = (3/2)·B·f₀·i·j·n` to first order in B.
    ///
    /// The *free* longitudinal series is not predictable from `(f₀, B)` — it
    /// needs the string's length and `√(E/ρ)` — so this is the testable half of
    /// the longitudinal candidate.
    fn phantoms(&self, key: usize, n: usize, out: &mut Vec<f32>) {
        out.clear();
        let (f0, b) = (self.f0[key], self.b[key]);
        if f0 <= 0.0 || n < 2 {
            return;
        }
        let f = |m: usize| m as f32 * f0 * (1.0 + b * (m * m) as f32).sqrt();
        for i in 1..n {
            let j = n - i;
            if i <= j {
                out.push(f(i) + f(j));
            }
            // 2fᵢ − fⱼ with 2i − j = n, i.e. j = 2i − n.
            if 2 * i > n {
                out.push(2.0 * f(i) - f(2 * i - n));
            }
        }
    }

    /// This key's partials inside `centre ± half`, by the stiff-string law.
    fn near(&self, key: usize, centre: f32, half: f32, out: &mut Vec<f32>) {
        out.clear();
        let (f0, b) = (self.f0[key], self.b[key]);
        if f0 <= 0.0 {
            return;
        }
        for m in 1..=64u32 {
            let f = m as f32 * f0 * (1.0 + b * (m * m) as f32).sqrt();
            if f > centre + half {
                break;
            }
            if f >= centre - half {
                out.push(f);
            }
        }
    }
}

/// Does any candidate frequency in `set` fold onto the extra line?
///
/// A candidate is admitted only if the analysis window could pass it at all,
/// and it is then folded, because the baseband is decimated: a component
/// `Δ` from the reference is indistinguishable from one at `Δ ± k·f_hop`.
fn folds_onto(set: &[f32], e: &Extra, offset_hz: f32, tol: f32) -> bool {
    set.iter().any(|f| {
        let delta = f - e.f_ref;
        delta.abs() <= e.lobe_hz() && (fold_hz(delta) - offset_hz).abs() <= tol
    })
}

/// **E11** — do the extra lines sit at *fixed absolute frequencies*?
///
/// Prompt T's second discriminating experiment, and Prompt E's test: recurrence
/// of the same absolute frequency across *different keys* is the signature of an
/// instrument or room resonance rather than a property of the struck string.
fn e11_recurrence(captures: &[(Capture, Vec<Resolved>, UnisonVerdict, usize)]) {
    const TOL_HZ: f32 = 0.5;
    let all = extras(captures);
    println!("\n=== E11: do the extra lines recur at fixed absolute frequencies? ===");
    println!(
        "A soundboard or room resonance sits where it sits whatever is struck, so an extra\n\
         line it caused must reappear at the same absolute frequency under *other* keys.\n\
         'shared' = this line has a partner within {TOL_HZ} Hz reported under a different key;\n\
         'chance' is the same statistic with the offsets permuted between lines of the same\n\
         register, which keeps both the reference layout and the offsets' own distribution\n\
         and destroys only which line carries which. A resonance shows as shared ≫ chance."
    );
    println!(
        "{:>20} {:>7} {:>9} {:>9} {:>10} {:>10}",
        "register", "lines", "shared", "chance", "top bin", "null bin"
    );
    for band in ["bass", "tenor", "treble"] {
        let rows: Vec<&Extra> = all.iter().filter(|e| register(e.key) == band).collect();
        if rows.len() < 10 {
            continue;
        }
        let shared = |freqs: &[(u8, f32)]| -> f32 {
            let hit = freqs
                .iter()
                .filter(|(k, f)| {
                    freqs
                        .iter()
                        .any(|(k2, f2)| k2 != k && (f2 - f).abs() <= TOL_HZ)
                })
                .count();
            100.0 * hit as f32 / freqs.len() as f32
        };
        let observed: Vec<(u8, f32)> = rows.iter().map(|e| (e.key, e.abs_hz())).collect();
        let permuted = shuffle(rows.iter().map(|e| e.offset_hz).collect(), 0x8f51_2ab3);
        let drawn: Vec<(u8, f32)> = rows
            .iter()
            .zip(&permuted)
            .map(|(e, d)| (e.key, e.f_ref + d))
            .collect();
        // How many distinct keys the most-populated bin draws from — the same
        // question asked without a pairing rule, and asked of the null too.
        let crowd = |set: &[(u8, f32)]| -> usize {
            set.iter()
                .map(|(_, f)| {
                    let mut keys: Vec<u8> = set
                        .iter()
                        .filter(|(_, f2)| (f2 - f).abs() <= TOL_HZ)
                        .map(|(k, _)| *k)
                        .collect();
                    keys.sort_unstable();
                    keys.dedup();
                    keys.len()
                })
                .max()
                .unwrap_or(0)
        };
        let best = crowd(&observed);
        println!(
            "{band:>20} {:>7} {:>8.0}% {:>8.0}% {:>10} {:>10}",
            rows.len(),
            shared(&observed),
            shared(&drawn),
            best,
            crowd(&drawn)
        );
        if band.starts_with("bass") && best >= 3 {
            // The candidate fingerprint itself: where the most keys agree.
            let mut top: Vec<(f32, usize)> = observed
                .iter()
                .map(|(_, f)| {
                    let mut keys: Vec<u8> = observed
                        .iter()
                        .filter(|(_, f2)| (f2 - f).abs() <= TOL_HZ)
                        .map(|(k, _)| *k)
                        .collect();
                    keys.sort_unstable();
                    keys.dedup();
                    (*f, keys.len())
                })
                .collect();
            top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.total_cmp(&b.0)));
            top.dedup_by(|a, b| (a.0 - b.0).abs() <= TOL_HZ);
            print!("  most-shared bass frequencies (Hz × distinct keys × partials):");
            for (f, k) in top.iter().take(5) {
                let mut partials: Vec<usize> = rows
                    .iter()
                    .filter(|e| (e.abs_hz() - f).abs() <= TOL_HZ)
                    .map(|e| e.partial)
                    .collect();
                partials.sort_unstable();
                partials.dedup();
                print!("  {f:.1}×{k}×n{partials:?}");
            }
            println!();
        }
    }
}

/// **E12** — the attribution itself: every extra line against the families that
/// could have produced it.
///
/// Every transverse partial of every key is predictable from the measured
/// (f₀, B), so the struck key's own series, the neighbouring keys' series and
/// Conklin's nonlinear-mixing family can each be *predicted* and the leftovers
/// classified. Each family is scored against the same test run on redrawn
/// positions, because the piano's spectrum is dense enough that a family with
/// enough members explains everything by coincidence.
fn e12_attribution(captures: &[(Capture, Vec<Resolved>, UnisonVerdict, usize)]) {
    const TOL_HZ: f32 = 0.5;
    let table = KeyTable::of(captures);
    let all = extras(captures);
    println!("\n=== E12: what could have produced the extra lines ===");
    println!(
        "Each family is predicted from the instrument's own measured (f₀, B), admitted only\n\
         if the analysis window's main lobe reaches it (±{:.0} Hz at the 1024-sample window,\n\
         ±{:.0} Hz at 4096), then folded into the baseband. Tolerance {TOL_HZ} Hz.\n\
         'chance' repeats the identical test with the offsets permuted between lines.\n",
        2.0 * SAMPLE_RATE as f32 / 1024.0,
        2.0 * SAMPLE_RATE as f32 / 4096.0,
    );
    println!(
        "{:>20} {:>7} {:>26} {:>9} {:>9} {:>9}",
        "register", "lines", "family", "explains", "chance", "excess"
    );

    let families: [(&str, i32); 7] = [
        ("own partials (E-N)", -1),
        ("neighbour ±1 semitone", 1),
        ("neighbour ±2 semitones", 2),
        ("neighbour ±12 (octave)", 12),
        ("any key on the piano", 88),
        ("Conklin mixing, any fᵢ±fⱼ", -2),
        ("phantom at this partial", -3),
    ];

    let mut candidates: Vec<f32> = Vec::new();
    let mut scratch: Vec<f32> = Vec::new();
    // The bass is split by *window*, because that is what decides which
    // candidates the front end can admit at all: keys whose f₁ clears 86 Hz —
    // roughly 20 upwards — ship the 1024-sample window with the panel still
    // displaying partial 6, so their neighbouring partials are inside its main
    // lobe and fold (E10d).
    let bands: [(&str, usize); 3] = [
        ("bass, 4096 window", 4096),
        ("bass, 1024 window", 1024),
        ("tenor", 0),
    ];
    for (band, window) in bands {
        let rows: Vec<&Extra> = all
            .iter()
            .filter(|e| {
                if window == 0 {
                    register(e.key) == "tenor"
                } else {
                    register(e.key).starts_with("bass") && e.window == window
                }
            })
            .collect();
        if rows.len() < 10 {
            continue;
        }
        // The null shuffles the *observed* offsets between lines of the same
        // register rather than drawing uniformly: real offsets concentrate near
        // the reference, and a uniform draw would credit every family with the
        // difference between those two distributions.
        let shuffled = shuffle(rows.iter().map(|e| e.offset_hz).collect(), 0x1d2c_5f97);
        for (label, kind) in families {
            let (mut hit, mut chance) = (0usize, 0usize);
            for (idx, e) in rows.iter().enumerate() {
                candidates.clear();
                let key = e.key as usize;
                match kind {
                    -1 => {
                        table.near(key, e.f_ref, e.lobe_hz(), &mut scratch);
                        // The reference partial itself is not a candidate: it is
                        // the line the extra is measured against.
                        candidates.extend(scratch.iter().filter(|f| (*f - e.f_ref).abs() > TOL_HZ));
                    }
                    -2 => {
                        table.near(key, 0.0, 4.0 * e.f_ref, &mut scratch);
                        for (a, fi) in scratch.iter().enumerate() {
                            for fj in &scratch[a..] {
                                for f in [fi + fj, 2.0 * fi - fj] {
                                    if (f - e.f_ref).abs() <= e.lobe_hz() {
                                        candidates.push(f);
                                    }
                                }
                            }
                        }
                    }
                    -3 => table.phantoms(key, e.partial, &mut candidates),
                    span => {
                        let lo = key.saturating_sub(span as usize);
                        let hi = (key + span as usize).min(87);
                        for k2 in lo..=hi {
                            if k2 == key {
                                continue;
                            }
                            table.near(k2, e.f_ref, e.lobe_hz(), &mut scratch);
                            candidates.extend(scratch.iter());
                        }
                    }
                }
                hit += usize::from(folds_onto(&candidates, e, e.offset_hz, TOL_HZ));
                chance += usize::from(folds_onto(&candidates, e, shuffled[idx], TOL_HZ));
            }
            let n = rows.len() as f32;
            let observed = 100.0 * hit as f32 / n;
            let expected = 100.0 * chance as f32 / n;
            println!(
                "{band:>20} {:>7} {label:>26} {observed:>8.0}% {expected:>8.0}% {:>+8.0}%",
                rows.len(),
                observed - expected
            );
        }
    }

    println!("\n--- E12b: which side of the partial do they sit on? ---");
    println!(
        "A second string is above or below with equal prior probability, and so are the two\n\
         polarizations of one string. A *phantom* partial is not: the transverse series is\n\
         stretched by inharmonicity while the mixing product fᵢ + fⱼ adds linearly, so it\n\
         must land BELOW its partial, by f_n − (fᵢ + fⱼ) ≈ (3/2)·B·f₀·i·j·n.\n"
    );
    println!(
        "{:>20} {:>7} {:>9} {:>12} {:>12} {:>12} {:>11}",
        "register", "lines", "below", "median δ Hz", "median |δ|", "in cents", "|δ|/(2/T)"
    );
    for band in ["bass", "tenor", "treble"] {
        let rows: Vec<&Extra> = all.iter().filter(|e| register(e.key) == band).collect();
        if rows.len() < 10 {
            continue;
        }
        let below = rows.iter().filter(|e| e.offset_hz < 0.0).count();
        println!(
            "{band:>20} {:>7} {:>8.0}% {:>12.2} {:>12.2} {:>11.1}¢ {:>11.2}",
            rows.len(),
            100.0 * below as f32 / rows.len() as f32,
            median(rows.iter().map(|e| e.offset_hz).collect()),
            median(rows.iter().map(|e| e.offset_hz.abs()).collect()),
            median(
                rows.iter()
                    .map(|e| 1200.0 * (1.0 + e.offset_hz.abs() / e.f_ref).log2())
                    .collect()
            ),
            median(
                rows.iter()
                    .filter(|e| e.resolution_hz > 0.0)
                    .map(|e| e.offset_hz.abs() / e.resolution_hz)
                    .collect()
            ),
        );
    }

    println!("\n--- E12c: the phantom prediction, per partial ---");
    println!(
        "Predicted offsets are the mixing products fᵢ + fⱼ with i + j = n and 2fᵢ − fⱼ with\n\
         2i − j = n, evaluated on the key's own measured (f₀, B) — a sparse set of at most\n\
         n/2 values, all negative. 'ratio' is the observed offset over the nearest predicted\n\
         one; a family that is right predicts 1.0.\n"
    );
    println!(
        "{:>20} {:>4} {:>7} {:>12} {:>12} {:>9} {:>9}",
        "register", "n", "lines", "predicted Hz", "observed Hz", "ratio", "within .5"
    );
    for band in ["bass", "tenor"] {
        for n in [2usize, 4, 6, 8] {
            let rows: Vec<&Extra> = all
                .iter()
                .filter(|e| register(e.key) == band && e.partial == n)
                .collect();
            if rows.len() < 10 {
                continue;
            }
            let mut predicted = Vec::new();
            let mut ratios = Vec::new();
            let mut within = 0usize;
            for e in &rows {
                table.phantoms(e.key as usize, n, &mut candidates);
                let Some(best) = candidates
                    .iter()
                    .map(|f| f - e.f_ref)
                    .min_by(|a, b| (a - e.offset_hz).abs().total_cmp(&(b - e.offset_hz).abs()))
                else {
                    continue;
                };
                predicted.push(best);
                ratios.push(e.offset_hz / best);
                within += usize::from((best - e.offset_hz).abs() <= TOL_HZ);
            }
            if predicted.is_empty() {
                continue;
            }
            println!(
                "{band:>20} {n:>4} {:>7} {:>12.2} {:>12.2} {:>9.2} {:>8.0}%",
                rows.len(),
                median(predicted),
                median(rows.iter().map(|e| e.offset_hz).collect()),
                median(ratios),
                100.0 * within as f32 / rows.len() as f32,
            );
        }
    }

    println!("\n--- E12e: are the flanking lines a symmetric pair? ---");
    println!(
        "Where three lines resolve, the two weaker ones are measured against the strongest.\n\
         A modulation — anything that varies the string's amplitude or frequency at a fixed\n\
         rate — puts a *symmetric* pair at ±ν around every partial, so `|d₁+d₂|/(|d₁|+|d₂|)`\n\
         is 0. Three strings, or a string plus an unrelated line, have no reason to be\n\
         symmetric. 'chance' pairs each d₁ with another triple's d₂.\n"
    );
    println!(
        "{:>20} {:>8} {:>12} {:>12} {:>10} {:>10}",
        "register", "triples", "median |d₁|", "median |d₂|", "symmetry", "chance"
    );
    for band in ["bass", "tenor", "treble"] {
        let mut d1 = Vec::new();
        let mut d2 = Vec::new();
        for (cap, resolved, ..) in captures.iter().filter(|(c, ..)| register(c.key) == band) {
            for (i, r) in resolved.iter().enumerate() {
                if r.count == 3 && cap.partial_hz[i + 1].is_some() {
                    d1.push(r.lines[1].offset_hz - r.lines[0].offset_hz);
                    d2.push(r.lines[2].offset_hz - r.lines[0].offset_hz);
                }
            }
        }
        if d1.len() < 10 {
            continue;
        }
        let asymmetry = |a: &[f32], b: &[f32]| -> Vec<f32> {
            a.iter()
                .zip(b)
                .map(|(x, y)| (x + y).abs() / (x.abs() + y.abs()).max(1e-6))
                .collect()
        };
        let observed = asymmetry(&d1, &d2);
        let permuted = asymmetry(&d1, &shuffle(d2.clone(), 0x94d0_49bb));
        println!(
            "{band:>20} {:>8} {:>12.2} {:>12.2} {:>10.2} {:>10.2}",
            d1.len(),
            median(d1.iter().map(|x| x.abs()).collect()),
            median(d2.iter().map(|x| x.abs()).collect()),
            median(observed),
            median(permuted),
        );
    }

    println!("\n--- E12d: the law the splits follow, fitted per capture ---");
    println!(
        "The discriminator fits ln Δ = ln a + p·ln f and asks whether p is 1 (a unison, both\n\
         strings' partials scaling together) or 0 (a separation fixed in Hz). A phantom\n\
         partial obeys neither: Δ ≈ (3/2)·B·f₀·i·j·n with i·j ≈ n²/4 gives Δ ∝ n³, i.e.\n\
         p ≈ 3. This is the same fit the shipped test runs, reported as a distribution\n\
         instead of a verdict.\n"
    );
    println!(
        "A split at the record's own limit is reported *at* the limit whatever the truth was\n\
         (ADR 0012 §4), and the limit is the same for every partial — which would itself\n\
         manufacture p̂ = 0. The second row per register admits only splits wider than\n\
         2 × 2/T, where §4 measures the reported separation to be exact.\n"
    );
    println!(
        "{:>20} {:>13} {:>9} {:>9} {:>9} {:>9} {:>10} {:>9}",
        "register", "splits used", "captures", "median p̂", "p10", "p90", "|p̂−1|<3σ", "|p̂|<3σ"
    );
    for band in ["bass", "tenor", "treble"] {
        for (label, floor) in [("all", 0.0f32), ("> 2 × 2/T", 2.0)] {
            let mut slopes = Vec::new();
            let (mut near_unison, mut near_fixed) = (0usize, 0usize);
            for (cap, resolved, ..) in captures.iter().filter(|(c, ..)| register(c.key) == band) {
                let mut points = Vec::new();
                for (i, r) in resolved.iter().enumerate() {
                    let Some(f_ref) = cap.partial_hz[i + 1] else {
                        continue;
                    };
                    if r.count < 2 {
                        continue;
                    }
                    let lines = &r.lines[..r.count as usize];
                    let lo = lines
                        .iter()
                        .map(|l| l.offset_hz)
                        .fold(f32::INFINITY, f32::min);
                    let hi = lines
                        .iter()
                        .map(|l| l.offset_hz)
                        .fold(f32::NEG_INFINITY, f32::max);
                    if hi - lo > floor * r.resolution_hz {
                        points.push((f_ref.ln(), (hi - lo).ln()));
                    }
                }
                let Some((slope, se)) = log_slope(&points) else {
                    continue;
                };
                slopes.push(slope);
                near_unison += usize::from((slope - 1.0).abs() <= 3.0 * se);
                near_fixed += usize::from(slope.abs() <= 3.0 * se);
            }
            if slopes.len() < 5 {
                continue;
            }
            let mut sorted = slopes.clone();
            sorted.sort_by(f32::total_cmp);
            let n = slopes.len();
            println!(
                "{band:>20} {label:>13} {n:>9} {:>9.2} {:>9.2} {:>9.2} {:>9.0}% {:>8.0}%",
                median(slopes),
                sorted[n / 10],
                sorted[(n * 9 / 10).min(n - 1)],
                100.0 * near_unison as f32 / n as f32,
                100.0 * near_fixed as f32 / n as f32,
            );
        }
    }
}

/// A deterministic permutation of `v` — the null that keeps a sample's own
/// distribution and destroys only which line it belongs to.
fn shuffle(mut v: Vec<f32>, seed: u32) -> Vec<f32> {
    let mut noise = Noise(seed);
    for i in (1..v.len()).rev() {
        let j = ((noise.next() + 0.5) * (i + 1) as f32) as usize;
        v.swap(i, j.min(i));
    }
    v
}

/// Least-squares slope of `(x, y)` and its standard error, the way
/// `Unison::discriminate` computes them. `None` when the fit is degenerate.
fn log_slope(points: &[(f32, f32)]) -> Option<(f32, f32)> {
    if points.len() < 3 {
        return None;
    }
    let n = points.len() as f32;
    let mean_x = points.iter().map(|p| p.0).sum::<f32>() / n;
    let mean_y = points.iter().map(|p| p.1).sum::<f32>() / n;
    let s_xx: f32 = points.iter().map(|p| (p.0 - mean_x).powi(2)).sum();
    let s_xy: f32 = points.iter().map(|p| (p.0 - mean_x) * (p.1 - mean_y)).sum();
    if s_xx <= 0.0 {
        return None;
    }
    let slope = s_xy / s_xx;
    let rss: f32 = points
        .iter()
        .map(|p| (p.1 - mean_y - slope * (p.0 - mean_x)).powi(2))
        .sum();
    let se = (rss / (n - 2.0) / s_xx).sqrt();
    (slope.is_finite() && se.is_finite() && se > 0.0).then_some((slope, se))
}

/// **E9** — per-hop cost of the whole bank in `--release`, against the 23.2 ms
/// callback.
fn e9_cost() {
    // Every reference live and every ring at the cap — the worst case, which a
    // real capture does not reach (its upper partials gate out and stop
    // transforming).
    let mut refs = [0.0f32; MAX_STROBE_REFS];
    let count = MAX_STROBE_REFS;
    for (i, r) in refs.iter_mut().enumerate() {
        *r = 220.0 * (i + 1) as f32;
    }
    let hops = UNISON_RING_HOPS * 2;
    let sources: Vec<Source> = (0..MAX_STROBE_REFS)
        .map(|i| Source {
            offset_hz: 220.0 * i as f32 + 0.9,
            amplitude: 1.0,
            tau_secs: 60.0,
        })
        .collect();
    let audio = synth_audio(SYNTH_REF_HZ, &sources, hops, 40.0, 0x5bf0_3635);

    let mut frame = tuner_core::pipeline::ProcessingFrame::new();
    let mut run = |coarse: u8| -> Vec<f32> {
        let mut strobe = Strobe::new(SAMPLE_RATE);
        strobe.retarget(StrobeRefUpdate {
            count,
            refs,
            coarse_index: coarse,
            spacing_hz: 440.0,
        });
        let mut us = Vec::with_capacity(hops);
        for h in 0..hops {
            frame.audio_buffer[..BASS_WINDOW_SIZE]
                .copy_from_slice(&audio[h * HOP_SIZE..h * HOP_SIZE + BASS_WINDOW_SIZE]);
            let t0 = Instant::now();
            std::hint::black_box(strobe.process(&frame, 1e-6, false));
            us.push(t0.elapsed().as_secs_f32() * 1e6);
        }
        // Only hops with every ring at the cap, i.e. past the fill.
        us.split_off(UNISON_RING_HOPS)
    };
    let full = run(0);

    // The unison share alone: 12 records at the cap, transformed once each.
    let record: Vec<Complex<f32>> = (0..UNISON_RING_HOPS)
        .map(|h| {
            let p = 0.31 * h as f32;
            Complex::new(p.cos(), p.sin())
        })
        .collect();
    let fft = rustfft::FftPlanner::<f32>::new().plan_fft_forward(UNISON_RING_HOPS);
    let mut spectrum = vec![Complex { re: 0.0, im: 0.0 }; UNISON_RING_HOPS];
    let mut magnitudes = vec![0.0f32; UNISON_RING_HOPS];
    let mut fft_scratch = vec![Complex { re: 0.0, im: 0.0 }; fft.get_inplace_scratch_len()];
    let mut out = [UnisonLine::default(); MAX_UNISON_LINES];
    let reps = 2000;
    let t0 = Instant::now();
    for _ in 0..reps {
        for _ in 0..MAX_STROBE_REFS {
            std::hint::black_box(resolve_lines(
                &record,
                fft.as_ref(),
                spectral::candan_c_n(UNISON_RING_HOPS),
                HOP_RATE_HZ,
                &mut LineScratch {
                    spectrum: &mut spectrum,
                    magnitudes: &mut magnitudes,
                    fft: &mut fft_scratch,
                },
                &mut out,
            ));
        }
    }
    let per_hop_us = t0.elapsed().as_secs_f32() * 1e6 / reps as f32;

    let mut sorted = full.clone();
    sorted.sort_by(f32::total_cmp);
    println!("\n=== E9: per-hop cost, --release ===");
    println!(
        "callback budget {:.1} ms ({} samples at {} Hz)\n\
         whole bank, 12 references, {} hops:  median {:.1} µs, p90 {:.1} µs, max {:.1} µs\n\
         of which resolve_lines × 12 at the cap: {:.1} µs ({:.2} % of the budget)",
        1000.0 * HOP_SIZE as f32 / SAMPLE_RATE as f32,
        HOP_SIZE,
        SAMPLE_RATE,
        hops,
        median(full.clone()),
        sorted[(sorted.len() as f32 * 0.9) as usize],
        sorted.last().copied().unwrap_or(f32::NAN),
        per_hop_us,
        100.0 * per_hop_us / (1e6 * HOP_SIZE as f32 / SAMPLE_RATE as f32),
    );
    println!(
        "ring memory: {} references × {} hops × 8 B = {:.1} KB touched per hop",
        MAX_STROBE_REFS,
        UNISON_RING_HOPS,
        (MAX_STROBE_REFS * UNISON_RING_HOPS * 8) as f32 / 1024.0
    );
}

fn main() {
    let root = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "diagnostics".into());
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .expect("read diagnostics dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("key_"))
                    .unwrap_or(false)
        })
        .collect();
    dirs.sort();

    let table = default_display_partials();
    let hop_to_hz = HOP_RATE_HZ; // slope cyc/hop → Hz

    // E1 accumulators (fundamental-alive, in-range captures).
    let mut e1: Vec<(u8, f32, f32, f32)> = Vec::new(); // key, offset, rate_err, resid
    // E2 bass A/B accumulators.
    let mut e2_bass: Vec<(u8, f32, f32)> = Vec::new(); // key, resid_1024, resid_4096
    // E3: key, σ_d, p99.9 |Δ|, max |Δ| — all in cycles per hop.
    let mut e3: Vec<(u8, f32, f32, f32)> = Vec::new();
    // E4: key, displayed reference Hz, σ_d, observed slope jitter per window.
    let mut e4: Vec<(u8, f32, f32, Vec<Vec<f32>>)> = Vec::new();
    // E5: |shipped beat rate − an independent refit of the same points|, Hz.
    let mut e5: Vec<f32> = Vec::new();
    // E7/E8: per capture, what each reference resolved, the bank verdict, and
    // the analysis window that produced it (1024, or 4096 in the deep bass).
    let mut unison: Vec<(Capture, Vec<Resolved>, UnisonVerdict, usize)> = Vec::new();

    println!(
        "{:>4} {:>4} {:>3} {:>8} {:>8} {:>8} {:>7} {:>6}",
        "key", "note", "n*", "offHz", "rateHz", "errHz", "resid", "gated"
    );

    for dir in &dirs {
        let Some(cap) = load(dir) else { continue };
        let key = cap.key as usize;
        let f_et = NOTES[key].frequency;
        let n_star = table[key] as usize;

        // ── E7/E8: the unison rings, on the key's own measured partials ──
        // The measured series centres each baseband, which is what the design
        // note's real-capture runs did; the app's reference is the curve target
        // and an out-of-tune string sits off-centre (ADR 0012, Limitations).
        //
        // Piano #2's cached deep-bass partials predate the MAT seed fix, so any
        // capture whose fundamental is absurd against ET is dropped rather than
        // consumed (`06-capture-sets.md`). v1 is tenor and treble regardless.
        let mut measured = [0.0f32; MAX_STROBE_REFS];
        let n_measured = measured_refs(&cap, &mut measured);
        let sane = cap.partial_hz[1]
            .is_some_and(|f1| f1 > 0.0 && (1200.0 * (f1 / f_et).log2()).abs() < 200.0);
        if n_measured > 0 && sane {
            let window = if measured[0] * 1024.0 < 2.0 * SAMPLE_RATE as f32 {
                BASS_WINDOW_SIZE / 2 // the R3 long-window path
            } else {
                WINDOW_SIZE / 2
            };
            let (resolved, verdict) =
                run_unison(&cap.audio, &measured, n_measured, cap.f0, cap.noise_floor);
            unison.push((
                Capture {
                    key: cap.key,
                    f0: cap.f0,
                    b: cap.b,
                    noise_floor: cap.noise_floor,
                    partial_hz: cap.partial_hz.clone(),
                    audio: cap.audio.clone(),
                },
                resolved,
                verdict,
                window,
            ));
        }

        // ── E1: ET fundamental reference, read the n=1 band ──
        // refs = pure ET harmonic series (B=0); refs[0]=f_ET drives the window.
        let mut refs_et = [0.0f32; MAX_STROBE_REFS];
        let mut count = 0;
        for (i, r) in refs_et.iter_mut().enumerate() {
            let f = (i + 1) as f32 * f_et;
            if f >= SAMPLE_RATE as f32 / 2.0 {
                break;
            }
            *r = f;
            count += 1;
        }
        // ── E3: per-hop delta noise at the *displayed* partial ──
        // The band the user watches, on the shipped ET reference set, so the
        // margin below the alias boundary can be derived instead of chosen.
        if n_star >= 1 && n_star <= count {
            let (unwrapped, ungated, shipped) = run_strobe(&cap, &refs_et, count, n_star - 1);
            e5.extend(rate_disagreement(
                &unwrapped,
                &ungated,
                &shipped,
                BAND_SLOPE_POINTS,
                BAND_SLOPE_MIN_POINTS,
            ));
            if let Some((sd, p999, dmax)) = delta_noise(&unwrapped, &ungated, BAND_WIN_HOPS) {
                e3.push((cap.key, sd, p999, dmax));
                let jit = WINDOW_SECS
                    .iter()
                    .map(|t| {
                        let n = (t * HOP_RATE_HZ).round() as usize;
                        slope_diffs(&unwrapped, &ungated, n.max(3))
                    })
                    .collect();
                let f_disp = refs_et[n_star - 1];
                e4.push((cap.key, f_disp, sd, jit));
            }
        }

        if let Some(f_live1) = cap.partial_hz[1] {
            let offset = f_live1 - f_et;
            if offset.abs() < ALIAS_HZ {
                let (unwrapped, ungated, _) = run_strobe(&cap, &refs_et, count, 0);
                let gated = 1.0
                    - ungated.iter().filter(|u| **u).count() as f32 / ungated.len().max(1) as f32;
                if gated < 0.5 && unwrapped.len() > 8 {
                    let (slope, resid) = fit(&unwrapped);
                    let rate = slope * hop_to_hz;
                    let err = rate - offset;
                    e1.push((cap.key, offset, err, resid));
                    println!(
                        "{:>4} {:>4} {:>3} {:>8.2} {:>8.2} {:>8.2} {:>7.4} {:>5.0}%",
                        cap.key,
                        NOTES[key].name,
                        1,
                        offset,
                        rate,
                        err,
                        resid,
                        gated * 100.0
                    );
                }
            }
        }

        // ── E2: measured stiff-string reference, bass window A/B ──
        // For bass keys only (where the long window is selected), compare the
        // displayed partial's steadiness under 4096 vs 1024.
        if refs_et[0] * 1024.0 < 2.0 * SAMPLE_RATE as f32 {
            // bass
            if let Some(f_live) = cap.partial_hz[n_star] {
                // Reference at the measured partial ⇒ ideally stationary.
                let r1024 = steadiness(&cap, f_live, false);
                let r4096 = steadiness(&cap, f_live, true);
                e2_bass.push((cap.key, r1024, r4096));
            }
        }
    }

    // ── Summaries ──

    println!("\n=== E3: per-hop delta noise → the derived readable-range margin ===");
    println!("captures used: {}", e3.len());
    if !e3.is_empty() {
        let f_hop = HOP_RATE_HZ;
        println!(
            "alias boundary f_hop/2 = {:.2} Hz;  fit window = {} hops\n\n\
             {:>10} {:>8} {:>9} {:>9} {:>10} {:>10}",
            ALIAS_HZ,
            BAND_WIN_HOPS,
            "register",
            "caps",
            "σ_d cyc",
            "max |Δ|",
            "3σ_d Hz",
            "max Δ Hz"
        );
        for (label, lo, hi) in [
            ("bass 0–23", 0u8, 23u8),
            ("mid 24–59", 24, 59),
            ("treble 60–87", 60, 87),
            ("ALL", 0, 87),
        ] {
            let rows: Vec<&(u8, f32, f32, f32)> =
                e3.iter().filter(|r| r.0 >= lo && r.0 <= hi).collect();
            if rows.is_empty() {
                continue;
            }
            let sd = median(rows.iter().map(|r| r.1).collect());
            let dmax = median(rows.iter().map(|r| r.3).collect());
            println!(
                "{:>10} {:>8} {:>9.4} {:>9.4} {:>10.2} {:>10.2}",
                label,
                rows.len(),
                sd,
                dmax,
                3.0 * sd * f_hop,
                dmax * f_hop
            );
        }
        let worst = e3
            .iter()
            .max_by(|a, b| a.3.total_cmp(&b.3))
            .expect("non-empty");
        println!(
            "\nworst single capture: key {} max |Δ| {:.4} cyc = {:.2} Hz of margin;\n\
             p99.9 pooled {:.4} cyc = {:.2} Hz.  Shipped margin is 3.50 Hz \
             (BAND_READABLE_HZ 18.0).",
            worst.0,
            worst.3,
            worst.3 * f_hop,
            median(e3.iter().map(|r| r.2).collect()),
            median(e3.iter().map(|r| r.2).collect()) * f_hop
        );
    }

    println!("\n=== E4: fit-window length — jitter (measured) vs motion lag (exact) ===");
    if !e4.is_empty() {
        let f_hop = HOP_RATE_HZ;
        println!(
            "captures: {}.  OLS over a window estimates the rate at the window's MIDPOINT, so the\n\
             readout lags a turning peg by exactly T/2 — a group delay, not a budget.  Jitter is\n\
             pooled window-to-window scatter, in cents at each capture's own displayed reference;\n\
             it is an UPPER bound (real rate drift lands in it too).  OLS× is the ratio to the\n\
             independent-sample prediction σ_η·√(12/(n(n²−1))), i.e. the window-overlap penalty.\n\n\
             {:>7} {:>6} {:>8} {:>9} {:>9} {:>9} {:>8} {:>7}",
            e4.len(),
            "T (s)",
            "hops",
            "lag T/2",
            "bass ¢",
            "mid ¢",
            "treble ¢",
            "vs .186",
            "OLS×"
        );
        let mut first: Option<f32> = None;
        for (wi, t) in WINDOW_SECS.iter().enumerate() {
            let n = (t * f_hop).round() as usize;
            let band = |lo: u8, hi: u8| -> (f32, f32, usize) {
                // Pool the differences, convert once at the median reference.
                let mut d: Vec<f32> = Vec::new();
                let mut refs: Vec<f32> = Vec::new();
                let mut ratios: Vec<f32> = Vec::new();
                for (key, f_disp, sd, diffs) in &e4 {
                    if *key < lo || *key > hi {
                        continue;
                    }
                    d.extend(diffs[wi].iter().copied());
                    refs.push(*f_disp);
                    let pred = (sd / 2f32.sqrt())
                        * (12.0 / (n as f32 * (n as f32 * n as f32 - 1.0))).sqrt();
                    if pred > 0.0 && !diffs[wi].is_empty() {
                        let rms = (diffs[wi].iter().map(|x| x * x).sum::<f32>()
                            / diffs[wi].len() as f32
                            / 2.0)
                            .sqrt();
                        ratios.push(rms / pred);
                    }
                }
                if d.is_empty() {
                    return (f32::NAN, f32::NAN, 0);
                }
                let sigma = (d.iter().map(|x| x * x).sum::<f32>() / d.len() as f32 / 2.0).sqrt();
                let r = median(refs);
                (
                    1200.0 * (1.0 + sigma * f_hop / r).log2(),
                    median(ratios),
                    d.len(),
                )
            };
            let (b, rb, _) = band(0, 23);
            let (m, rm, _) = band(24, 59);
            let (tr, rt, _) = band(60, 87);
            let all = [b, m, tr];
            let mean_c = all.iter().filter(|x| x.is_finite()).sum::<f32>()
                / all.iter().filter(|x| x.is_finite()).count().max(1) as f32;
            if first.is_none() {
                first = Some(mean_c);
            }
            let rr = [rb, rm, rt];
            let ols = rr.iter().filter(|x| x.is_finite()).sum::<f32>()
                / rr.iter().filter(|x| x.is_finite()).count().max(1) as f32;
            println!(
                "{:>7.3} {:>6} {:>7.0}ms {:>9.3} {:>9.3} {:>9.3} {:>8.2} {:>6.1}×",
                t,
                n,
                t * 500.0,
                b,
                m,
                tr,
                mean_c / first.unwrap_or(mean_c),
                ols
            );
        }
        println!(
            "\nIf jitter fell as the independent-sample law says, 'vs .186' would read\n\
             {:.2} at 0.4 s and {:.2} at 1.0 s.",
            (0.186f32 / 0.4).powf(1.5),
            (0.186f32 / 1.0).powf(1.5)
        );
    }

    println!("\n=== E5: shipped beat rate vs an independent refit of the same points ===");
    if e5.is_empty() {
        println!("no published rates (every run shorter than the minimum span)");
    } else {
        let mut sorted = e5.clone();
        sorted.sort_by(f32::total_cmp);
        println!(
            "published rates: {}  (window {} points, minimum {})\n\
             |Δ| median {:.2e} Hz, p99 {:.2e} Hz, max {:.2e} Hz",
            e5.len(),
            BAND_SLOPE_POINTS,
            BAND_SLOPE_MIN_POINTS,
            median(e5.clone()),
            sorted[((sorted.len() as f32 * 0.99) as usize).min(sorted.len() - 1)],
            sorted.last().copied().unwrap_or(f32::NAN),
        );
    }

    println!("\n=== E1: detuning coherence (ET fundamental, in-range, alive) ===");
    println!("captures used: {}", e1.len());
    if !e1.is_empty() {
        println!(
            "median |rate error| vs true detuning: {:.3} Hz",
            median(e1.iter().map(|x| x.2.abs()).collect())
        );
        println!(
            "median rotation residual (cycles): {:.4}   [<~0.05 = clean beat]",
            median(e1.iter().map(|x| x.3).collect())
        );
        println!(
            "offset range covered: {:.1} .. {:.1} Hz",
            e1.iter().map(|x| x.1).fold(f32::MAX, f32::min),
            e1.iter().map(|x| x.1).fold(f32::MIN, f32::max),
        );
    }

    println!("\n=== E2: bass steadiness, 1024 vs 4096 window (measured ref) ===");
    println!("bass captures: {}", e2_bass.len());
    if !e2_bass.is_empty() {
        println!(
            "median residual  1024-window: {:.4} cycles",
            median(e2_bass.iter().map(|x| x.1).collect())
        );
        println!(
            "median residual  4096-window: {:.4} cycles",
            median(e2_bass.iter().map(|x| x.2).collect())
        );
        let improved = e2_bass.iter().filter(|x| x.2 < x.1).count();
        println!(
            "captures steadier with 4096: {}/{}",
            improved,
            e2_bass.len()
        );
    }

    // ── Unison assist ──
    e6_synthetic();
    e10_bass_null();
    if !unison.is_empty() {
        e7_real(&unison);
        e8_unexplained(&unison);
        e11_recurrence(&unison);
        e12_attribution(&unison);
    }
    e9_cost();
}
