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
//! Run: `cargo run --release --example strobe_replay -- [diagnostics_dir]`

use std::path::{Path, PathBuf};

use tuner_core::algorithms::curves::default_display_partials;
use tuner_core::algorithms::spectral::{goertzel, goertzel_bass};
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_RATE_HZ, HOP_SIZE, SAMPLE_RATE};
use tuner_core::models::NOTES;
use tuner_core::strobe::{
    BAND_SLOPE_MIN_POINTS, BAND_SLOPE_POINTS, BAND_SLOPE_WINDOW_SECS, MAX_STROBE_REFS, Strobe,
    StrobeRefUpdate,
};

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

    println!(
        "{:>4} {:>4} {:>3} {:>8} {:>8} {:>8} {:>7} {:>6}",
        "key", "note", "n*", "offHz", "rateHz", "errHz", "resid", "gated"
    );

    for dir in &dirs {
        let Some(cap) = load(dir) else { continue };
        let key = cap.key as usize;
        let f_et = NOTES[key].frequency;
        let n_star = table[key] as usize;

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
    let median = |mut v: Vec<f32>| -> f32 {
        if v.is_empty() {
            return f32::NAN;
        }
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() / 2]
    };

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
}
