//! # Pitch ground-truth audit — who is actually right?
//!
//! The strobe-panel cents readout is `1200·log₂(f_inst / f_ET)`, where `f_inst`
//! is the engine's phase-vocoder estimate for the fundamental. The `--selftest`
//! mode below shows that estimator is accurate to < 0.6 ¢ on clean synthetic
//! tones — so a several-cent reading disagreement must come from **real audio**
//! through the **full** pipeline, not the isolated math. This harness settles it
//! on captured audio by measuring the same note three independent ways:
//!
//! 1. **`app`** — our shipped hot path: Gatekeeper + Engine driven at the real
//!    hop cadence in manual mode (`target = key`), reading the n = 1 `f_inst`.
//! 2. **`truth`** — a high-resolution Hann-windowed, heavily zero-padded DFT
//!    magnitude peak around f_ET. Method-independent from the phase vocoder
//!    (magnitude, not phase differencing); the arbiter.
//! 3. **`yin`** — a textbook YIN / autocorrelation estimate, the family the
//!    field's phone/web tuners use — to see whether *they* are the biased ones.
//!
//! If `app` disagrees with **both** `truth` and `yin`, the bias is ours. If
//! `app` and `yin` agree but both differ from `truth`, the autocorrelation
//! family (and the phone) is the biased reference. Decisive either way.
//!
//! Run: `cargo run --release --example pitch_ground_truth -- <dir>`
//! `<dir>` is a single capture (contains `audio.raw`) or a parent of `key_*`
//! capture dirs (e.g. `diagnostics/`). Optional `--keys 19,24,29` filters.

use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossbeam_queue::ArrayQueue;
use realfft::RealFftPlanner;
use rustfft::num_complex::Complex;

use tuner_core::algorithms::curves;
use tuner_core::algorithms::peaks;
use tuner_core::algorithms::spectral::{
    self, fft, goertzel_windowed, magnitude_spectrum, neyman_pearson_k,
};
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE, WINDOW_SIZE};
use tuner_core::engine::Engine;
use tuner_core::gatekeeper::{Gatekeeper, SignalState};
use tuner_core::models::{KeyProfile, NOTES, get_expected_beta};
use tuner_core::pipeline::ProcessingFrame;
use tuner_core::strobe::{MAX_STROBE_REFS, Strobe, StrobeRefUpdate};

const SAMPLE_RATE: u32 = 44_100;

const TAU: f32 = 2.0 * std::f32::consts::PI;

/// Sliding window for the band-slope readout (#4): ~0.5 s at the 43 Hz hop.
const BAND_WIN_HOPS: usize = 21;

fn read_raw_f32(path: &Path) -> Option<Vec<f32>> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() || bytes.len() % 4 != 0 {
        return None;
    }
    let mut out = vec![0.0f32; bytes.len() / 4];
    // SAFETY: f32 has no invalid bit patterns; length is a multiple of 4.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.as_mut_ptr() as *mut u8, bytes.len());
    }
    Some(out)
}

/// `key_034_G3_...` → 34.
fn key_from_dirname(name: &str) -> Option<u8> {
    let rest = name.strip_prefix("key_")?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Builds the 88 prior-B templates the live pipeline seeds when nothing is
/// measured (the guitar/ET case — no captured B for the key).
fn prior_profiles() -> [KeyProfile; 88] {
    let v: Vec<KeyProfile> = (0..88)
        .map(|i| KeyProfile::new(NOTES[i].frequency, get_expected_beta(i as u8)))
        .collect();
    v.try_into().expect("88 profiles")
}

/// **Truth.** Highest-resolution frequency estimate we can make offline,
/// independent of the hot-path phase vocoder. Hann-windows the freshest `win`
/// samples, zero-pads ×4, takes the magnitude-spectrum peak within ±80 ¢ of
/// `center_hz`, and parabolically interpolates. The magnitude peak of a
/// (possibly decaying) sinusoid is unbiased.
///
/// `center_hz` is any partial's predicted frequency, not just the fundamental:
/// the ±80 ¢ search is narrow enough to isolate one partial anywhere the
/// neighbours are further away than that, which holds for every partial of
/// every key (spacing ≈ f₀ ≫ 80 ¢ of f_n for n below the treble limit).
fn dtft_truth(signal: &[f32], center_hz: f32, planner: &mut RealFftPlanner<f32>) -> Option<f32> {
    let win = signal.len().min(32_768);
    if win < 8_192 {
        return None;
    }
    let n = (win.next_power_of_two()) * 4; // heavy zero-pad → fine bin grid
    let start = signal.len() - win;
    let mut buf = vec![0.0f32; n];
    for (i, b) in buf.iter_mut().take(win).enumerate() {
        let w = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (win as f32 - 1.0)).cos());
        *b = signal[start + i] * w;
    }
    let fftp = planner.plan_fft_forward(n);
    let mut spec = fftp.make_output_vec();
    fftp.process(&mut buf, &mut spec).ok()?;

    let hz_per_bin = SAMPLE_RATE as f32 / n as f32;
    let lo = (center_hz * 2f32.powf(-80.0 / 1200.0) / hz_per_bin) as usize;
    let hi = ((center_hz * 2f32.powf(80.0 / 1200.0) / hz_per_bin) as usize).min(spec.len() - 2);
    let lo = lo.max(1);
    let (mut best, mut best_mag) = (lo, 0.0f32);
    for (k, bin) in spec[lo..=hi].iter().enumerate() {
        let m = bin.norm();
        if m > best_mag {
            best_mag = m;
            best = lo + k;
        }
    }
    // Parabolic interpolation on log-magnitude of the three bins around the peak.
    let a = spec[best - 1].norm().max(1e-20).ln();
    let b = spec[best].norm().max(1e-20).ln();
    let c = spec[best + 1].norm().max(1e-20).ln();
    let denom = a - 2.0 * b + c;
    let delta = if denom.abs() > 1e-12 {
        0.5 * (a - c) / denom
    } else {
        0.0
    };
    Some((best as f32 + delta) * hz_per_bin)
}

/// **YIN** (de Cheveigné & Kawahara 2002) over the freshest `win` samples:
/// difference function → cumulative-mean normalization → absolute threshold
/// 0.1 → parabolic interpolation. Represents the autocorrelation family.
fn yin(signal: &[f32], f_min: f32, f_max: f32) -> Option<f32> {
    let win = signal.len().min(16_384);
    if win < 4_096 {
        return None;
    }
    let start = signal.len() - win;
    let s = &signal[start..start + win];
    let tau_max = ((SAMPLE_RATE as f32 / f_min) as usize + 1).min(win / 2);
    let tau_min = (SAMPLE_RATE as f32 / f_max) as usize;
    let w = win / 2;

    let mut d = vec![0.0f32; tau_max + 1];
    for (tau, dt) in d.iter_mut().enumerate().take(tau_max + 1).skip(1) {
        let mut sum = 0.0f32;
        for i in 0..w {
            let diff = s[i] - s[i + tau];
            sum += diff * diff;
        }
        *dt = sum;
    }
    // Cumulative mean normalized difference.
    let mut dp = vec![1.0f32; tau_max + 1];
    let mut running = 0.0f32;
    for tau in 1..=tau_max {
        running += d[tau];
        dp[tau] = if running > 0.0 {
            d[tau] * tau as f32 / running
        } else {
            1.0
        };
    }
    // First dip below threshold, else global min.
    let mut tau_est = None;
    let mut tau = tau_min.max(2);
    while tau < tau_max {
        if dp[tau] < 0.1 {
            while tau + 1 < tau_max && dp[tau + 1] < dp[tau] {
                tau += 1;
            }
            tau_est = Some(tau);
            break;
        }
        tau += 1;
    }
    let tau =
        tau_est.or_else(|| (tau_min.max(2)..tau_max).min_by(|&a, &b| dp[a].total_cmp(&dp[b])))?;
    // Parabolic interpolation around tau.
    let (x0, x1, x2) = (dp[tau - 1], dp[tau], dp[tau + 1]);
    let denom = x0 + x2 - 2.0 * x1;
    let delta = if denom.abs() > 1e-12 {
        0.5 * (x0 - x2) / denom
    } else {
        0.0
    };
    Some(SAMPLE_RATE as f32 / (tau as f32 + delta))
}

struct AppResult {
    f0: f32,
    locked_key: Option<u8>,
    gated_frac: f32,
    /// Hop-to-hop std of the *instantaneous* cents readout over the settled
    /// tail — the "strobe jitter" the user sees on the number (the band, being
    /// integrated, does not carry it).
    cents_jitter: f32,
}

/// **App.** Drives the real Gatekeeper + Engine at the live hop cadence
/// (`pipeline.rs` step order) in manual mode. Returns the settled-tail mean of
/// the n = 1 `f_inst` and the engine's own `cents_deviation`.
fn run_engine(
    signal: &[f32],
    key: u8,
    noise_floor: f32,
    planner: &mut RealFftPlanner<f32>,
) -> Option<AppResult> {
    let fft_treble = planner.plan_fft_forward(WINDOW_SIZE);
    let fft_bass = planner.plan_fft_forward(BASS_WINDOW_SIZE);
    let profiles = prior_profiles();

    let mut frame = ProcessingFrame::new();
    let mut gate = Gatekeeper::new(Arc::new(ArrayQueue::new(1)));
    gate.config.silence_threshold = noise_floor;
    let mut engine = Engine::new(SAMPLE_RATE);
    engine.noise_floor = noise_floor;

    let hops_total = (signal.len() - BASS_WINDOW_SIZE) / HOP_SIZE + 1;
    let settle_from = hops_total * 3 / 5;

    let f_et = NOTES[key as usize].frequency;
    let mut f0_acc = 0.0f32;
    let mut alive = 0u32;
    let mut settled_hops = 0u32;
    let mut gated_hops = 0u32;
    let mut locked_key = None;
    let mut hop_cents: Vec<f32> = Vec::new();

    let mut cursor = 0usize;
    let mut h = 0usize;
    while cursor + BASS_WINDOW_SIZE <= signal.len() {
        frame.audio_buffer[..BASS_WINDOW_SIZE]
            .copy_from_slice(&signal[cursor..cursor + BASS_WINDOW_SIZE]);
        let newest = BASS_WINDOW_SIZE - WINDOW_SIZE;
        fft(
            &frame.audio_buffer[newest..BASS_WINDOW_SIZE],
            &mut frame.time_buffer[..WINDOW_SIZE],
            &mut frame.frequency_buffer[..],
            &fft_treble,
            WINDOW_SIZE,
        );
        fft(
            &frame.audio_buffer[..BASS_WINDOW_SIZE],
            &mut frame.time_buffer[..BASS_WINDOW_SIZE],
            &mut frame.bass_frequency_buffer[..],
            &fft_bass,
            BASS_WINDOW_SIZE,
        );
        let gr = gate.process_frame(&frame);
        magnitude_spectrum(
            &frame.frequency_buffer[..],
            WINDOW_SIZE,
            &mut frame.treble_magnitude_buffer[..WINDOW_SIZE / 2],
        );
        magnitude_spectrum(
            &frame.bass_frequency_buffer[..],
            BASS_WINDOW_SIZE,
            &mut frame.bass_magnitude_buffer[..BASS_WINDOW_SIZE / 2],
        );

        let res = engine.process(
            &frame,
            &profiles,
            gr.state == SignalState::Silence,
            gr.state == SignalState::Stable,
            gr.is_new_onset,
            gr.is_transient_bypass,
            Some(key),
        );

        if let Some(r) = res {
            locked_key = Some(r.key_index);
            let p1 = (0..r.partial_count).find(|&i| r.partial_ns[i] == 1);
            if h >= settle_from {
                settled_hops += 1;
                match p1 {
                    Some(i) if r.partial_freqs[i].is_finite() && r.partial_freqs[i] > 0.0 => {
                        f0_acc += r.partial_freqs[i];
                        hop_cents.push(cents(r.partial_freqs[i], f_et));
                        alive += 1;
                    }
                    _ => gated_hops += 1,
                }
            }
        } else if h >= settle_from {
            settled_hops += 1;
            gated_hops += 1;
        }

        cursor += HOP_SIZE;
        h += 1;
    }

    if alive == 0 {
        return Some(AppResult {
            f0: f32::NAN,
            locked_key,
            gated_frac: 1.0,
            cents_jitter: f32::NAN,
        });
    }
    let mean_c = hop_cents.iter().sum::<f32>() / hop_cents.len() as f32;
    let var =
        hop_cents.iter().map(|c| (c - mean_c).powi(2)).sum::<f32>() / hop_cents.len().max(1) as f32;
    Some(AppResult {
        f0: f0_acc / alive as f32,
        locked_key,
        gated_frac: gated_hops as f32 / settled_hops.max(1) as f32,
        cents_jitter: var.sqrt(),
    })
}

fn cents(f: f32, f_ref: f32) -> f32 {
    1200.0 * (f / f_ref).log2()
}

fn process_capture(dir: &Path, planner: &mut RealFftPlanner<f32>) -> Option<u8> {
    let key = key_from_dirname(dir.file_name()?.to_str()?)?;
    let signal = read_raw_f32(&dir.join("audio.raw"))?;
    if signal.len() < BASS_WINDOW_SIZE {
        return None;
    }
    let f_et = NOTES[key as usize].frequency;
    let name = &NOTES[key as usize].name;

    let truth = dtft_truth(&signal, f_et, planner);
    let yin_f0 = yin(&signal, f_et * 0.7, f_et * 1.5);
    let app = run_engine(&signal, key, 0.001, planner);

    let fmt = |o: Option<f32>| match o {
        Some(f) if f.is_finite() && f > 0.0 => format!("{:>8.3}Hz {:>+6.1}¢", f, cents(f, f_et)),
        _ => "     --        ".to_string(),
    };
    let app_f0 = app.as_ref().map(|a| a.f0);
    print!(
        "{:<3} {:>4} f_ET={:>8.3}  truth {}  yin {}  app {}",
        key,
        name,
        f_et,
        fmt(truth),
        fmt(yin_f0),
        fmt(app_f0),
    );
    if let (Some(a), Some(t)) = (app_f0, truth)
        && a.is_finite()
    {
        print!("   app−truth {:>+6.1}¢", cents(a, f_et) - cents(t, f_et));
    }
    if let Some(a) = &app {
        print!(
            "   lock={:?} gate={:.0}% jitter=±{:.1}¢",
            a.locked_key,
            a.gated_frac * 100.0,
            a.cents_jitter
        );
    }
    if let Some((mean, jit)) = band_slope_cents(&signal, f_et) {
        print!("   BAND-slope {mean:+.1}¢ jitter=±{jit:.1}¢");
    }
    println!();
    Some(key)
}

/// Additive guitar-ish tone: partial n at `n·f0·√(1+B·n²)`, per-partial decay
/// `exp(−t·n^0.6/tau0)` (higher partials die faster — the string physics).
fn synth_tone(f0: f32, b: f32, amps: &[f32], len: usize, tau0: f32) -> Vec<f32> {
    let fs = SAMPLE_RATE as f32;
    (0..len)
        .map(|i| {
            let t = i as f32 / fs;
            let mut s = 0.0;
            for (k, &a) in amps.iter().enumerate() {
                if a == 0.0 {
                    continue;
                }
                let n = (k + 1) as f32;
                let f_n = n * f0 * (1.0 + b * n * n).sqrt();
                let env = if tau0 > 0.0 {
                    (-t * n.powf(0.6) / tau0).exp()
                } else {
                    1.0
                };
                s += a * env * (2.0 * std::f32::consts::PI * f_n * t).sin();
            }
            0.1 * s
        })
        .collect()
}

/// Validates the arbiter (`truth`) and `yin` against *known* detunings before
/// we trust either on real audio. A biased estimator here disqualifies its
/// column on the real captures.
fn selftest(planner: &mut RealFftPlanner<f32>) {
    let amps = [0.30f32, 1.0, 0.7, 0.45, 0.25]; // weak-fundamental (wound-string) worst case
    let keys: [(u8, &str); 6] = [
        (19, "E2"),
        (24, "A2"),
        (29, "D3"),
        (34, "G3"),
        (38, "B3"),
        (43, "E4"),
    ];
    println!("SELF-TEST — recovered − true cents on synthetic tones (decay, weak fundamental).");
    println!("Any nonzero here is estimator bias, not a real reading.\n");
    for (b_name, b) in [("B=0", 0.0f32), ("B=3e-4", 3e-4)] {
        println!("── {b_name} ──   (columns: true¢ → truth bias / yin bias)");
        for (key, name) in keys {
            let f_et = NOTES[key as usize].frequency;
            let mut row = String::new();
            for &c in &[-5.0f32, 0.0, 5.0] {
                let f_true = f_et * 2f32.powf(c / 1200.0);
                let sig = synth_tone(f_true, b, &amps, SAMPLE_RATE as usize * 3 / 2, 0.6);
                let tb = dtft_truth(&sig, f_et, planner)
                    .map(|f| cents(f, f_et) - c)
                    .unwrap_or(f32::NAN);
                let yb = yin(&sig, f_et * 0.7, f_et * 1.5)
                    .map(|f| cents(f, f_et) - c)
                    .unwrap_or(f32::NAN);
                row.push_str(&format!("{c:+.0}→[{tb:+5.2}/{yb:+5.2}] "));
            }
            println!("  {name:<3} {f_et:>7.2}Hz  {row}");
        }
        println!();
    }
}

/// **#2 test.** Quantifies YIN (whole-signal) sharpness vs inharmonicity `B`
/// and partial richness. Mechanism claim: a partial-weighted estimate reads
/// sharp by ≈ `866·B·⟨n²⟩`, where `⟨n²⟩` is the power-weighted mean-square
/// partial index — because partial n implies a fundamental `f₀·√(1+Bn²)`,
/// sharp by `866·B·n²` cents. A *harmonic* signal (B=0) has an exact period
/// ⇒ zero drift at any partial count (missing-fundamental recovered). Our
/// strobe reads n=1 ⇒ ⟨n²⟩=1 ⇒ B-immune.
fn inharm_sweep() {
    let f0 = 110.0f32; // A2
    println!("YIN sharpness vs B and partial richness (f0=110 Hz, aₙ=1/n, decay).");
    println!("drift = 1200·log₂(yin/f0) cents; pred ≈ 866·B·⟨n²⟩.");
    println!("B=0 must give ≈0 at every K (the 'independent of inharmonicity' check).\n");
    let bs = [0.0f32, 5e-5, 1e-4, 3e-4, 1e-3];
    print!("{:<20}", "");
    for b in bs {
        print!("  B={b:>7.0e}    ");
    }
    println!();
    for k in [5usize, 10, 15, 20] {
        let mut amps = [0.0f32; 24];
        for (i, a) in amps.iter_mut().enumerate().take(k) {
            *a = 1.0 / (i + 1) as f32;
        }
        let num: f32 = (0..k)
            .map(|i| amps[i] * amps[i] * ((i + 1) * (i + 1)) as f32)
            .sum();
        let den: f32 = (0..k).map(|i| amps[i] * amps[i]).sum();
        let mn2 = num / den;
        print!("K={k:<2} ⟨n²⟩={mn2:5.1}      ");
        for b in bs {
            let sig = synth_tone(f0, b, &amps[..k], SAMPLE_RATE as usize * 3 / 2, 0.6);
            let drift = yin(&sig, f0 * 0.7, f0 * 1.5)
                .map(|y| 1200.0 * (y / f0).log2())
                .unwrap_or(f32::NAN);
            let pred = 866.0 * b * mn2;
            print!("{drift:+5.2}(p{pred:+5.2}) ");
        }
        println!();
    }
}

/// Gate-aware unwrap → longest contiguous ungated run → sliding
/// least-squares slope → beat Hz → cents. The post-processing half of the
/// band-slope readout, shared by the single-reference and full-set drivers.
///
/// Gate-awareness matters: a gated hop *holds* the bank's angle, so counting
/// it would contribute zero drift and drag the fit toward 0 ¢ — a decayed note
/// would read "in tune". Gated hops therefore break the run, and the fit takes
/// the longest contiguous ungated stretch, which for a fast-decaying treble
/// note is its early, still-ringing life.
fn slope_from_angles(
    angles: &[(f32, bool)],
    f_ref: f32,
    win_hops: usize,
) -> Option<(f32, f32, usize)> {
    let t_hop = HOP_SIZE as f32 / SAMPLE_RATE as f32;
    let mut runs: Vec<Vec<f32>> = Vec::new();
    let mut current: Vec<f32> = Vec::new();
    let mut prev = 0.0f32;
    let mut acc = 0.0f32;
    let mut have_prev = false;

    for &(a, gated) in angles {
        if gated {
            if current.len() > 1 {
                runs.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
            have_prev = false;
            continue;
        }
        if !have_prev {
            prev = a;
            acc = 0.0;
            have_prev = true;
        } else {
            let mut d = a - prev;
            if d > 0.5 {
                d -= 1.0;
            } else if d < -0.5 {
                d += 1.0;
            }
            acc += d;
            prev = a;
        }
        current.push(acc);
    }
    if current.len() > 1 {
        runs.push(current);
    }
    let unwrapped = runs.into_iter().max_by_key(|r| r.len()).unwrap_or_default();
    let run_hops = unwrapped.len();
    if run_hops < win_hops + 2 {
        return None;
    }

    let mut vals: Vec<f32> = Vec::new();
    for h in win_hops..unwrapped.len() {
        let seg = &unwrapped[h - win_hops..h];
        let n = seg.len() as f32;
        let sx: f32 = (0..seg.len()).map(|i| i as f32).sum();
        let sy: f32 = seg.iter().sum();
        let sxx: f32 = (0..seg.len()).map(|i| (i * i) as f32).sum();
        let sxy: f32 = seg.iter().enumerate().map(|(i, &y)| i as f32 * y).sum();
        let slope = (n * sxy - sx * sy) / (n * sxx - sx * sx); // cycles/hop
        let beat_hz = slope / t_hop;
        vals.push(1200.0 * ((f_ref + beat_hz) / f_ref).log2());
    }
    if vals.is_empty() {
        return None;
    }
    let mean = vals.iter().sum::<f32>() / vals.len() as f32;
    let var = vals.iter().map(|c| (c - mean).powi(2)).sum::<f32>() / vals.len() as f32;
    Some((mean, var.sqrt(), run_hops))
}

/// Drives the `Strobe` over a whole capture and returns the per-hop
/// `(angle, gated)` series for every live reference.
///
/// The reference set is installed in **one** `StrobeRefUpdate`, exactly as the
/// live app does. This is load-bearing, not cosmetic: the bank's long-window
/// rule keys off `refs[0]`, so retargeting with a lone higher-partial
/// reference would select the short window and silently diverge from the
/// shipped path.
fn strobe_angles(
    signal: &[f32],
    refs: &[f32; MAX_STROBE_REFS],
    count: usize,
) -> Vec<Vec<(f32, bool)>> {
    let mut strobe = Strobe::new(SAMPLE_RATE);
    strobe.retarget(StrobeRefUpdate {
        count,
        refs: *refs,
        // Strobe angles only — this harness drives the coarse read directly.
        coarse_index: 0,
        spacing_hz: refs[0],
    });
    let mut out: Vec<Vec<(f32, bool)>> = vec![Vec::new(); count];
    let mut cursor = 0usize;
    let mut frame_buf = tuner_core::pipeline::ProcessingFrame::new();
    while cursor + BASS_WINDOW_SIZE <= signal.len() {
        frame_buf.audio_buffer[..BASS_WINDOW_SIZE]
            .copy_from_slice(&signal[cursor..cursor + BASS_WINDOW_SIZE]);
        let fr = strobe.process(&frame_buf, 0.0005, false);
        for (series, (&angle, &gated)) in out.iter_mut().zip(fr.angle.iter().zip(fr.gated.iter())) {
            series.push((angle, gated));
        }
        cursor += HOP_SIZE;
    }
    out
}

/// **#4 test.** The band-slope readout at a single reference (`f_ET` — the
/// guitar/ET case): accumulate the beat phase through the `Strobe`, then
/// take the least-squares slope of the unwrapped angle over a sliding
/// `win_hops` window. Returns `(mean_cents, jitter_std, run_hops)`.
fn band_slope_cents_win(signal: &[f32], f_et: f32, win_hops: usize) -> Option<(f32, f32, usize)> {
    let mut refs = [0.0f32; MAX_STROBE_REFS];
    refs[0] = f_et;
    let angles = strobe_angles(signal, &refs, 1);
    slope_from_angles(&angles[0], f_et, win_hops)
}

/// [`band_slope_cents_win`] at the shipped window length.
fn band_slope_cents(signal: &[f32], f_et: f32) -> Option<(f32, f32)> {
    band_slope_cents_win(signal, f_et, BAND_WIN_HOPS).map(|(m, j, _)| (m, j))
}

/// **Fit-window sweep.** Does a shorter baseline rescue the fast-decaying
/// treble, where the ~0.5 s window finds no long-enough ungated run? Reports
/// the longest ungated run (the note's usable life at `f_ref`) and the reading
/// at several window lengths, with its jitter — the accuracy/responsiveness
/// trade the CRLB predicts (variance ~1/T³).
fn window_sweep(dir: &Path, planner: &mut RealFftPlanner<f32>) {
    let Some(key) = dir
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(key_from_dirname)
    else {
        return;
    };
    let Some(signal) = read_raw_f32(&dir.join("audio.raw")) else {
        return;
    };
    let f_et = NOTES[key as usize].frequency;
    let Some(f_true) = dtft_truth(&signal, f_et, planner) else {
        return;
    };
    let hop_ms = 1000.0 * HOP_SIZE as f32 / SAMPLE_RATE as f32;
    let true_cents = 1200.0 * (f_true / f_et).log2();
    print!(
        "{:<4} f_true={f_true:>9.2} true={true_cents:>+7.1}¢ |",
        NOTES[key as usize].name
    );
    for win in [21usize, 12, 6, 3] {
        match band_slope_cents_win(&signal, f_et, win) {
            Some((m, j, run)) => print!(
                "  w{:>2}({:>3.0}ms,run{:>3}): {:>+6.1}±{:<4.1}",
                win,
                win as f32 * hop_ms,
                run,
                m,
                j
            ),
            None => print!("  w{win:>2}: {:>18}", "--"),
        }
    }
    println!();
}

/// The fixed-reference readable range (Hz): the largest |f_live − f_ref| whose
/// per-hop phase advance stays under ½ cycle, hence unwraps correctly. Beyond
/// it the band-slope aliases. Not a Goertzel limit — a hop/unwrap one.
const ALIAS_HZ: f32 = 0.5 * SAMPLE_RATE as f32 / HOP_SIZE as f32;

/// **Out-of-range test.** For each capture, place the strobe reference a known
/// `Δ` Hz below the string's true pitch and read the band-slope back. It should
/// track the true detuning up to ≈ [`ALIAS_HZ`], then break (alias) — the case
/// the regime-aware D4 routing must guard against.
fn alias_sweep(dir: &Path, planner: &mut RealFftPlanner<f32>) {
    println!("Band-slope vs reference offset — 'what happens out of tune'.");
    println!("readable range = ±{ALIAS_HZ:.1} Hz (hop/unwrap limit). Δ = string − reference.\n");
    let key = dir
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(key_from_dirname);
    let Some(key) = key else { return };
    let Some(signal) = read_raw_f32(&dir.join("audio.raw")) else {
        return;
    };
    let f_et = NOTES[key as usize].frequency;
    let Some(f_true) = dtft_truth(&signal, f_et, planner) else {
        return;
    };
    println!("{} f_true={f_true:.2} Hz", NOTES[key as usize].name);
    println!(
        "{:>7}  {:>10}  {:>12}  {:>8}",
        "Δ(Hz)", "true¢", "band-read¢", "error¢"
    );
    for d_hz in [0.0f32, 5.0, 10.0, 15.0, 18.0, 21.0, 24.0, 30.0, 40.0] {
        let f_ref = f_true - d_hz;
        let true_cents = 1200.0 * (f_true / f_ref).log2();
        match band_slope_cents(&signal, f_ref) {
            Some((read, _)) => {
                let err = read - true_cents;
                let flag = if d_hz > ALIAS_HZ {
                    " ← past limit"
                } else {
                    ""
                };
                println!("{d_hz:>7.0}  {true_cents:>+10.1}  {read:>+12.1}  {err:>+8.1}{flag}");
            }
            None => println!(
                "{d_hz:>7.0}  {true_cents:>+10.1}  {:>12}  {:>8}",
                "--", "--"
            ),
        }
    }
    println!();
}

// ─── Three-way readout comparison (Prompt N) ─────────────────────────────────
//
// The prompt's decision experiment: **tracker as-is** vs **tracker with the
// Defect-1 register window** vs **bounded spectral peak + jacobsen**, scored on
// availability (fraction of hops yielding any value — the treble's real
// limit), accuracy vs `truth`, and jitter. Availability is measured over the
// note's WHOLE life, not the settled tail: a fast-decaying treble note is
// already dead by the tail, so a tail-only measurement scores its availability
// as 0 for reasons that have nothing to do with the estimator.

/// Periodic-form Hann coefficients — the same window
/// [`goertzel_windowed`]'s callers use, at a length chosen at runtime.
fn hann_vec(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 * (1.0 - (TAU * i as f32 / (n as f32 - 1.0)).cos()))
        .collect()
}

/// The engine's `long_window` rule (`engine.rs`): the 1024-sample Hann
/// main-lobe half-width is `2·fs/1024`; take the long window whenever the
/// partial spacing (≈ f₀, proxied by the f₁ seed) falls inside it.
fn register_window(seed_hz: f32) -> usize {
    if seed_hz * 1024.0 < 2.0 * SAMPLE_RATE as f32 {
        4096
    } else {
        1024
    }
}

/// **Methods 1–2 — the adaptive phase-vocoder tracker** (`engine.rs`'s tracking
/// state, n = 1 only) at an arbitrary analysis window. A faithful replica of
/// the engine's recurrence: Goertzel at the adaptive center over the freshest
/// `win` samples of the COLA buffer, wrapped phase difference against the
/// expected advance at the target, NP amplitude gate at `K(win)`, then the
/// 0.95/0.05 EMA re-centering. `--readout` prints the shipped engine's own
/// reading alongside so the replica stays honest.
///
/// Returns one entry per hop: `None` where the method yields nothing (gated or
/// non-physical) — that is the availability signal.
fn tracker_series(signal: &[f32], seed_hz: f32, win: usize, noise_floor: f32) -> Vec<Option<f32>> {
    let window = hann_vec(win);
    let t_amp = noise_floor * neyman_pearson_k(win);
    let t_hop = HOP_SIZE as f32 / SAMPLE_RATE as f32;
    let mut target = seed_hz;
    let mut prev_phase = 0.0f32;
    let mut warm = false;
    let mut out = Vec::new();
    let mut cursor = 0usize;

    while cursor + BASS_WINDOW_SIZE <= signal.len() {
        let buf = &signal[cursor..cursor + BASS_WINDOW_SIZE];
        let (amp, phase) = goertzel_windowed(buf, SAMPLE_RATE, target, &window);
        cursor += HOP_SIZE;

        if !warm {
            prev_phase = phase;
            warm = true;
            out.push(None);
            continue;
        }
        let expected = TAU * target * t_hop;
        let delta = (phase - prev_phase - expected + std::f32::consts::PI).rem_euclid(TAU)
            - std::f32::consts::PI;
        prev_phase = phase;
        let f_inst = target + delta / (TAU * t_hop);

        if amp < t_amp || !(f_inst.is_finite() && f_inst > 0.0) {
            out.push(None);
        } else {
            target = 0.95 * target + 0.05 * f_inst;
            out.push(Some(f_inst));
        }
    }
    out
}

/// Search half-width for the bounded spectral read, in Hz. Three terms, in
/// order of precedence:
///
/// 1. a **cents** span (register-proportional — ±100 ¢ is a fixed musical
///    distance, unlike a fixed Hz span);
/// 2. a **bin floor** so the band stays resolvable where that span is sub-bin
///    (at A0, ±100 ¢ is ±1.6 Hz — under a third of one 8192 bin);
/// 3. a **neighbour cap at half the partial spacing**, which overrides both.
///
/// The cap is what `mat.rs`'s constant cannot be copied without: its 4-bin
/// floor lives in the Worker's 2¹⁶ FFT where a bin is 0.67 Hz, so the floor is
/// 2.7 Hz. At the pipeline's 2048 a bin is 21.5 Hz, so the same 4 bins is an
/// 86 Hz half-width — wider than a bass fundamental, and measurably fatal:
/// uncapped, the 2048 read at E2/A2/A0 returns the **2nd partial** (+1200 ¢).
///
/// `spacing_hz` is the distance to the neighbouring partial (≈ f₀), and is
/// **not** interchangeable with `center_hz`: the two coincide only for n = 1.
/// A read centered on A0's 4th partial has `center_hz ≈ 110` but
/// `spacing_hz ≈ 27.5`, and capping at `center_hz/2` there would admit a
/// ±55 Hz band spanning two neighbours. A band left under one bin by the cap
/// means that FFT size cannot serve that register at all — the selection rule,
/// not a tuning knob.
fn search_halfwidth_hz(
    center_hz: f32,
    spacing_hz: f32,
    span_cents: f32,
    min_bins: f32,
    hz_per_bin: f32,
) -> f32 {
    let span = center_hz * (2f32.powf(span_cents / 1200.0) - 1.0);
    span.max(min_bins * hz_per_bin).min(spacing_hz / 2.0)
}

/// False-alarm probability shared by every gate here — the same 0.001 the
/// shipped Neyman–Pearson gates commit to, so the variants differ only in
/// which noise they measure, never in how permissive they are.
const P_FA: f32 = 0.001;

/// Ordered-statistic CFAR configuration (Rohling 1983).
#[derive(Clone, Copy, PartialEq, Debug)]
struct CfarCfg {
    /// Order statistic as a fraction of the reference count (0.5 = median).
    quantile: f32,
    /// Cells excluded either side of the peak — its own main lobe. **The shipped
    /// read uses 0** (Rohling §V: unnecessary for an OS detector, and measured
    /// inert — audit 13). Retained here to sweep it and for the in-band control.
    guard_bins: usize,
    /// Take reference cells from outside the search band rather than inside.
    flanking: bool,
    /// Floor on the reference half-width, **in Hz**. In the deep bass partials
    /// are ≈ 5 bins apart at 8192, so 75 % of cells lie inside some partial's
    /// main lobe: a flank of `1.5 × spacing` samples only the strong low
    /// partials and the order statistic reads *signal* as noise. Widening spans
    /// partials ≈ 1–11 at A0 and imports the **weak upper** ones, 19–36 dB below
    /// the band peak, which is what the low quantile lands on (`--refset`).
    ///
    /// **Hz, not bins** — a bin is a different physical width at each FFT size
    /// (5.4 Hz at 8192, 21.5 Hz at 2048), so a bin-specified floor silently
    /// quadruples when the coarse read switches size. 172 Hz is the 8192-tuned
    /// value the deep-bass profile was measured at (32 bins there).
    flank_min_hz: f32,
    /// Use Rohling's exact finite-N scaling factor instead of the asymptotic
    /// quantile one.
    finite_n: bool,
}

/// Which detection threshold the bounded spectral read applies.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Gate {
    /// **The shipped gate.** Kay 1998 Neyman–Pearson against the *ambient*
    /// silence RMS (`engine.rs`, `strobe.rs`, discovery's peak threshold).
    /// Its H₀ is a quiet room, which is the wrong null during a sustain —
    /// see `docs/internals/suspected-issues.md`. Present here as the control.
    Ambient,
    /// **Ordered-statistic CFAR** against a *local* noise estimate taken from
    /// reference cells around the peak (Rohling 1983).
    Cfar(CfarCfg),
}

impl Gate {
    fn label(self) -> String {
        match self {
            Gate::Ambient => "ambient           ".to_string(),
            Gate::Cfar(c) => format!(
                "os{:.0}/g{}/{}/{:.0}Hz{}",
                c.quantile * 100.0,
                c.guard_bins,
                if c.flanking { "flank" } else { "band " },
                c.flank_min_hz,
                if c.finite_n { "/fN" } else { "/asy" }
            ),
        }
    }
}

/// **Asymptotic** threshold multiplier on an ordered-statistic noise estimate,
/// for Rayleigh magnitude bins — ours, derived.
///
/// Magnitude bins of locally-flat complex Gaussian noise are Rayleigh, so
/// `P(X > T) = exp(−T²/2σ²)` and the `q`-quantile is `σ·√(−2·ln(1−q))`.
/// Estimating σ from that quantile and solving for the P_fa threshold gives
/// `T = x_q · √( ln(P_fa) / ln(1−q) )` — 3.157 at the median, 4.900 at the
/// 25th percentile, both for P_fa = 0.001. Exact only as the reference count
/// → ∞; [`cfar_multiplier_finite`] is the finite-N form, and the two agree in
/// the limit (pinned by `test_cfar_multiplier_limit`).
fn cfar_multiplier(quantile: f32) -> f32 {
    (P_FA.ln() / (1.0 - quantile).ln()).sqrt()
}

/// **Exact finite-N** scaling factor — a faithful port of Rohling (1983).
///
/// His Eq. 14 gives the false-alarm probability of an OS-CFAR detector with
/// `N` reference cells selecting rank `k`, for an **exponentially** distributed
/// (square-law detector) parent population:
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
/// which is exact, monotone in `T`, and needs no gamma function.
///
/// Our cells are Rayleigh **magnitudes**, not exponential powers, so his
/// Table II does not apply directly — but the paper anticipates exactly this:
/// its closing section derives the linear-detector conversion **`T_lin = √T_sq`**
/// for the case where the receiver uses the absolute value and the cells "obey
/// a Rayleigh distribution". That conversion is applied here, making this a
/// port of Eqs 14 + 17 rather than a bespoke calibration.
///
/// # Reference
/// Rohling, H. (1983). "Radar CFAR Thresholding in Clutter and Multiple Target
/// Situations." IEEE Trans. Aerospace and Electronic Systems, AES-19(4),
/// pp. 608–621. DOI: 10.1109/TAES.1983.309350. (Eqs. 9–10, 12, 14, 17.)
/// Lineage: Finn, H. M. & Johnson, R. S. (1968), "Adaptive Detection Mode with
/// Threshold Control as a Function of Spatially Sampled Clutter-Level
/// Estimates", RCA Review 29(3), pp. 414–464 — the cell-averaging predecessor.
fn cfar_multiplier_finite(n_ref: usize, k: usize, p_fa: f32) -> f32 {
    if n_ref == 0 || k == 0 || k > n_ref {
        return cfar_multiplier(0.5);
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
    // P_fa is strictly decreasing in T; bisect for the square-law factor.
    let (mut lo, mut hi) = (0.0f64, 1.0e6f64);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if pfa(mid) > p_fa as f64 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    // Rohling Eq. 17: linear (magnitude) detector takes the square root.
    (0.5 * (lo + hi)).sqrt() as f32
}

/// Kay 1998 Neyman–Pearson AWGN magnitude threshold for an unnormalized
/// Hann-windowed FFT (`Σw² = 0.375·N`) — discovery's own peak gate.
fn ambient_threshold(noise_floor: f32, fft_size: usize) -> f32 {
    let p_bin = noise_floor * noise_floor * 0.375 * fft_size as f32;
    (-p_bin * P_FA.ln()).sqrt()
}

/// Outcome of one bounded spectral read, distinguishing *why* a hop produced
/// nothing — a gate that rejects and a gate that cannot be calibrated are very
/// different failures, and in the deep bass both occur.
enum Read {
    Hit(f32),
    /// The peak was below threshold.
    Rejected,
    /// Too few reference cells to estimate the local noise (CFAR only): the
    /// band minus its guard cells is empty, or the flanks ran off the
    /// spectrum. Deep bass at 8192 hits this — partial spacing is ≈ 5 bins,
    /// the capped band is ≈ 5 bins, and ±2 guard cells consume all of it.
    NoReference,
}

/// One bounded spectral read's full outcome.
struct ReadOut {
    read: Read,
    /// Reference cells used to set the threshold (0 for [`Gate::Ambient`]).
    n_ref: usize,
    /// Peak magnitude ÷ threshold — the CFAR margin. `> 1` iff admitted, and
    /// the quantity a strongest-partial policy ranks partials by (comparable
    /// across partials because each is normalized by its own local noise).
    margin: f32,
}

/// Outer bounds of the two flanking reference bands, before guard exclusion.
///
/// Shared by [`spectral_read`] and [`ref_anatomy`] so the anatomy report cannot
/// drift from the gate it describes. Both terms are in Hz and converted once:
/// the floor and the spacing rule must be compared in physical units, not bins,
/// because a bin is 5.4 Hz at 8192 and 21.5 Hz at 2048.
fn ref_window(
    lo: usize,
    hi: usize,
    spacing_hz: f32,
    c: &CfarCfg,
    hz_per_bin: f32,
    n_bins: usize,
) -> (usize, usize) {
    let flank_hz = (1.5 * spacing_hz).max(c.flank_min_hz);
    let flank = (flank_hz / hz_per_bin).ceil() as usize;
    (
        lo.saturating_sub(flank).max(1),
        (hi + flank).min(n_bins.max(2) - 2),
    )
}

/// **Method 3 — bounded spectral peak + `jacobsen`.** The candidate coarse
/// read: argmax of the already-computed magnitude spectrum within
/// [`search_halfwidth_hz`] of `center_hz`, refined sub-bin by the audited
/// Candan estimator, admitted by `gate`.
#[allow(clippy::too_many_arguments)]
fn spectral_read(
    magnitudes: &[f32],
    complex_spectrum: &[Complex<f32>],
    fft_size: usize,
    center_hz: f32,
    spacing_hz: f32,
    noise_floor: f32,
    span_cents: f32,
    min_bins: f32,
    gate: Gate,
    refs: &mut Vec<f32>,
) -> ReadOut {
    let hz_per_bin = SAMPLE_RATE as f32 / fft_size as f32;
    let half = search_halfwidth_hz(center_hz, spacing_hz, span_cents, min_bins, hz_per_bin);
    let n_bins = magnitudes.len();
    let lo = (((center_hz - half) / hz_per_bin).floor().max(1.0)) as usize;
    let hi = ((((center_hz + half) / hz_per_bin).ceil()) as usize).min(n_bins.max(2) - 2);
    if lo >= hi {
        return ReadOut {
            read: Read::NoReference,
            n_ref: 0,
            margin: 0.0,
        };
    }

    let (mut best, mut best_mag) = (lo, 0.0f32);
    for (k, &m) in magnitudes[lo..=hi].iter().enumerate() {
        if m > best_mag {
            best_mag = m;
            best = lo + k;
        }
    }

    let (threshold, n_ref) = match gate {
        Gate::Ambient => (ambient_threshold(noise_floor, fft_size), 0),
        Gate::Cfar(c) => {
            refs.clear();
            if c.flanking {
                // Reference cells from outside the search band; the order
                // statistic is what tolerates the partials the flank spans.
                let (outer_lo, outer_hi) = ref_window(lo, hi, spacing_hz, &c, hz_per_bin, n_bins);
                let cells = (outer_lo..lo).chain((hi + 1)..=outer_hi);
                refs.extend(
                    cells
                        .filter(|b| b.abs_diff(best) > c.guard_bins)
                        .map(|b| magnitudes[b]),
                );
            } else {
                refs.extend(
                    (lo..=hi)
                        .filter(|b| b.abs_diff(best) > c.guard_bins)
                        .map(|b| magnitudes[b]),
                );
            }
            if refs.len() < 4 {
                return ReadOut {
                    read: Read::NoReference,
                    n_ref: refs.len(),
                    margin: 0.0,
                };
            }
            refs.sort_by(f32::total_cmp);
            let k = (((refs.len() as f32 - 1.0) * c.quantile).round() as usize).max(1);
            let mult = if c.finite_n {
                // ── Search loss (measured, then derived) ──────────────────
                // Rohling's P_fa is for ONE cell under test, but this detector
                // takes the argmax over the whole search band, so it gets M
                // independent chances to false-alarm and the realized rate is
                // ≈ M·P_fa. Measured directly: collapsing the band to a single
                // bin brought the realized AWGN rate to 0.0012 against a
                // nominal 0.001 (exactly right), while the full band gave
                // 0.0386 — a 32× search loss. The correction is the standard
                // multiple-comparisons one: budget P_fa/M per cell. Hann
                // correlation makes adjacent bins non-independent, so M is the
                // band width halved.
                let m_eff = ((hi - lo).div_ceil(2).max(1)) as f32;
                cfar_multiplier_finite((refs.len() / 2).max(2), (k / 2).max(1), P_FA / m_eff)
            } else {
                cfar_multiplier(c.quantile)
            };
            (refs[k.min(refs.len() - 1)] * mult, refs.len())
        }
    };

    let margin = if threshold > 0.0 {
        best_mag / threshold
    } else {
        0.0
    };
    if best_mag < threshold {
        return ReadOut {
            read: Read::Rejected,
            n_ref,
            margin,
        };
    }
    let f = spectral::jacobsen(complex_spectrum, best, fft_size, SAMPLE_RATE);
    ReadOut {
        read: if f.is_finite() && f > 0.0 {
            Read::Hit(f)
        } else {
            Read::Rejected
        },
        n_ref,
        margin,
    }
}

/// Per-hop spectral reads at one FFT size, over the whole capture. Returns the
/// readings plus how many hops failed for lack of a calibratable reference set
/// and the median reference-set size (0 for [`Gate::Ambient`]).
#[allow(clippy::too_many_arguments)]
fn spectral_series(
    signal: &[f32],
    center_hz: f32,
    spacing_hz: f32,
    fft_size: usize,
    noise_floor: f32,
    span_cents: f32,
    min_bins: f32,
    gate: Gate,
    planner: &mut RealFftPlanner<f32>,
) -> (Vec<Option<f32>>, usize, usize) {
    let fftp = planner.plan_fft_forward(fft_size);
    let mut time = vec![0.0f32; fft_size];
    let mut spec = vec![Complex { re: 0.0, im: 0.0 }; fft_size / 2 + 1];
    let mut mag = vec![0.0f32; fft_size / 2];
    let mut refs: Vec<f32> = Vec::new();
    let mut out = Vec::new();
    let mut no_ref = 0usize;
    let mut ref_sizes: Vec<usize> = Vec::new();
    let mut cursor = 0usize;

    while cursor + BASS_WINDOW_SIZE <= signal.len() {
        // Newest `fft_size` samples of the same COLA window the pipeline holds.
        let end = cursor + BASS_WINDOW_SIZE;
        fft(
            &signal[end - fft_size..end],
            &mut time,
            &mut spec,
            &fftp,
            fft_size,
        );
        magnitude_spectrum(&spec, fft_size, &mut mag);
        let r = spectral_read(
            &mag,
            &spec,
            fft_size,
            center_hz,
            spacing_hz,
            noise_floor,
            span_cents,
            min_bins,
            gate,
            &mut refs,
        );
        ref_sizes.push(r.n_ref);
        out.push(match r.read {
            Read::Hit(f) => Some(f),
            Read::Rejected => None,
            Read::NoReference => {
                no_ref += 1;
                None
            }
        });
        cursor += HOP_SIZE;
    }
    ref_sizes.sort_unstable();
    let median_ref = ref_sizes.get(ref_sizes.len() / 2).copied().unwrap_or(0);
    (out, no_ref, median_ref)
}

/// Availability / accuracy / jitter of one method's hop series.
struct Score {
    avail: f32,
    /// Availability over the note's first third — the treble's usable life.
    avail_early: f32,
    median_cents: f32,
    jitter: f32,
}

fn score(series: &[Option<f32>], f_ref: f32) -> Score {
    let n = series.len().max(1);
    let early = (series.len() / 3).max(1);
    let hits = series.iter().filter(|v| v.is_some()).count();
    let hits_early = series[..early].iter().filter(|v| v.is_some()).count();

    let mut c: Vec<f32> = series
        .iter()
        .filter_map(|v| *v)
        .map(|f| cents(f, f_ref))
        .filter(|x| x.is_finite())
        .collect();
    if c.is_empty() {
        return Score {
            avail: hits as f32 / n as f32,
            avail_early: hits_early as f32 / early as f32,
            median_cents: f32::NAN,
            jitter: f32::NAN,
        };
    }
    c.sort_by(f32::total_cmp);
    let median = c[c.len() / 2];
    let mean = c.iter().sum::<f32>() / c.len() as f32;
    let var = c.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / c.len() as f32;
    Score {
        avail: hits as f32 / n as f32,
        avail_early: hits_early as f32 / early as f32,
        median_cents: median,
        jitter: var.sqrt(),
    }
}

/// **The three-way comparison.** Per capture, every candidate readout on the
/// same audio and the same reference, so the columns are directly comparable.
fn readout_compare(dir: &Path, planner: &mut RealFftPlanner<f32>, span_cents: f32, min_bins: f32) {
    let Some(key) = dir
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(key_from_dirname)
    else {
        return;
    };
    let Some(signal) = read_raw_f32(&dir.join("audio.raw")) else {
        return;
    };
    if signal.len() < BASS_WINDOW_SIZE {
        return;
    }
    let f_et = NOTES[key as usize].frequency;
    let noise_floor = 0.001;
    // Every method is seeded/referenced at the SAME place the live app would
    // have it: the ET target of the key the user selected.
    let seed = f_et;
    let truth_c = dtft_truth(&signal, f_et, planner).map(|f| cents(f, f_et));

    // The tracker seeded at the string's ACTUAL pitch. The ET-seeded rows can
    // fail two different ways — the seed is too far off to unwrap (aliasing,
    // an accuracy failure) or the partial is too weak/short to gate through (an
    // availability failure). Only a perfectly-seeded tracker separates them:
    // whatever it still cannot do is not a seeding problem. (The live engine
    // seeds from Stage B's refined scale, which lands between these two rows.)
    let ideal_seed = dtft_truth(&signal, f_et, planner).unwrap_or(seed);

    let win = register_window(seed);
    let rows: [(&str, Vec<Option<f32>>); 6] = [
        ("trk1024", tracker_series(&signal, seed, 1024, noise_floor)),
        ("trk4096", tracker_series(&signal, seed, 4096, noise_floor)),
        (
            if win == 4096 { "trkFIX*" } else { "trkFIX " },
            tracker_series(&signal, seed, win, noise_floor),
        ),
        (
            "trkTRU",
            tracker_series(&signal, ideal_seed, win, noise_floor),
        ),
        (
            "pk2048",
            spectral_series(
                &signal,
                seed,
                seed,
                2048,
                noise_floor,
                span_cents,
                min_bins,
                Gate::Ambient,
                planner,
            )
            .0,
        ),
        (
            "pk8192",
            spectral_series(
                &signal,
                seed,
                seed,
                8192,
                noise_floor,
                span_cents,
                min_bins,
                Gate::Ambient,
                planner,
            )
            .0,
        ),
    ];

    print!("{:<4} ", NOTES[key as usize].name);
    match truth_c {
        Some(t) => print!("true {t:>+7.1}¢ |"),
        None => print!("true      --  |"),
    }
    for (name, series) in &rows {
        let s = score(series, f_et);
        let err = truth_c.map(|t| s.median_cents - t).unwrap_or(f32::NAN);
        if s.median_cents.is_finite() {
            print!(
                "  {name} av{:>3.0}/{:>3.0}% e{:>+6.1} j{:>5.1}",
                s.avail * 100.0,
                s.avail_early * 100.0,
                err,
                s.jitter
            );
        } else {
            print!("  {name} av{:>3.0}/{:>3.0}%       --      ", 0.0, 0.0);
        }
    }
    println!();
}

// ─── Partial-centered bass read (the deep-bass question) ─────────────────────
//
// Every n = 1 method is junk below ≈ E1: the fundamental is not acoustically
// present, and even a 32k-sample DFT disagrees with itself across repeat
// captures of the same key. But the string's mistuning is observable on ANY
// partial, exactly: f_n = n·f₀·√(1+Bn²) is linear in f₀, so scaling the string
// by x cents scales every partial by x cents. Partial-relative cents IS
// string-relative cents, with no B correction at display time. This mode asks
// whether a bounded search centered on a *strong* partial reads cleanly where
// the fundamental cannot.

/// Highest partial index examined by [`bass_partials`]. The strobe bank holds
/// 12 references; the interesting bass energy is well inside the first six.
const MAX_BASS_PARTIAL: usize = 6;

/// Strobe reference frequencies for a key, in the **shipped** convention
/// ([`tuner_core::models::TuningCurve::strobe_partials`]):
/// `f₀* = f₁*/√(1+B)`, then `fₙ* = n·f₀*·√(1+B n²)`.
///
/// Getting this exactly right matters twice over. The obvious form
/// `n·f_ET·√(1+B n²)` treats f_ET as the *flexible-string* f₀, but the curve —
/// and every target in this app — is defined on the **audible first partial**
/// f₁. The two differ by √(1+B): 0.09 ¢ at A0, but **17 ¢ at A7**, which would
/// silently poison any treble column. And under the correct form the n = 1
/// reference is identically `f₁*` with B cancelling — the R4 B-immunity the
/// register table's treble entry relies on.
///
/// Offline there is no curve, so `f1` is the key's ET frequency: the ET-mode
/// reference the GUI uses for a guitar, and the cold-start target for a piano.
fn strobe_refs(f1: f32, b: f32, max_n: usize, out: &mut [f32; MAX_STROBE_REFS]) -> usize {
    let f0 = f1 / (1.0 + b).sqrt();
    let mut count = 0;
    for (i, slot) in out.iter_mut().enumerate().take(max_n.min(MAX_STROBE_REFS)) {
        let n = (i + 1) as f32;
        let f_n = n * f0 * (1.0 + b * n * n).sqrt();
        if f_n >= SAMPLE_RATE as f32 / 2.0 {
            break;
        }
        *slot = f_n;
        count += 1;
    }
    count
}

/// Least-squares `(f₀, B)` from measured partial frequencies — the standard
/// linearization of the stiff-string law: `(fₙ/n)² = f₀² + f₀²B·n²` is linear
/// in `n²`, so a regression of `y = (fₙ/n)²` on `x = n²` gives `f₀² =`
/// intercept and `B =` slope/intercept.
///
/// Used to answer "which partial would be best if the reference B were right",
/// without depending on the `analysis.json` files (whose deep-bass entries are
/// known rumble-seeded — the standing rule is to consume regenerated partials).
/// Here the partials come from the hi-res DFT truths, so this is a measurement
/// of B from the capture itself.
///
/// **n = 1 is excluded**, on two independent grounds that both point the same
/// way. Statistically it sits at `x = n² = 1`, the extreme of the regressor's
/// range, so it carries maximal leverage on the *intercept* — and B is
/// `slope/intercept`, so a bad fundamental corrupts B directly. Physically it
/// is the least informative point about B at all, since its `B·n²` term is the
/// smallest in the series (`f₁ ≈ f₀` for any plausible B). In the deep bass the
/// n = 1 "truth" is exactly the junk this whole investigation established it to
/// be, which is why the first version of this fit returned **negative B** on A0
/// and C#1.
fn fit_f0_b(freqs: &[(usize, f32)]) -> Option<(f32, f32)> {
    let freqs: Vec<(usize, f32)> = freqs.iter().copied().filter(|&(n, _)| n >= 2).collect();
    if freqs.len() < 2 {
        return None;
    }
    let (mut sx, mut sy, mut sxx, mut sxy) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let m = freqs.len() as f64;
    for &(n, f) in &freqs {
        let x = (n * n) as f64;
        let y = (f as f64 / n as f64).powi(2);
        sx += x;
        sy += y;
        sxx += x * x;
        sxy += x * y;
    }
    let denom = m * sxx - sx * sx;
    if denom.abs() < 1e-12 {
        return None;
    }
    let slope = (m * sxy - sx * sy) / denom;
    let intercept = (sy - slope * sx) / m;
    if intercept <= 0.0 {
        return None;
    }
    let f0 = intercept.sqrt();
    let b = slope / intercept;
    Some((f0 as f32, b as f32))
}

/// Pre-registered success criterion, fixed before the first run so the result
/// is a verdict rather than a curve-fit: at least one partial n ∈ 2..=6 must
/// reach **≥ 90 % availability**, **|median − truth_n| ≤ 2 ¢**, and
/// **jitter ≤ 10 ¢** on the deep-bass keys.
const BASS_PASS_AVAIL: f32 = 0.90;
const BASS_PASS_ERR_CENTS: f32 = 2.0;
const BASS_PASS_JITTER_CENTS: f32 = 10.0;

/// Per-partial reading of one capture. Returns whether any partial in
/// `2..=MAX_BASS_PARTIAL` met the pre-registered criterion.
fn bass_partials(
    dir: &Path,
    planner: &mut RealFftPlanner<f32>,
    span_cents: f32,
    min_bins: f32,
    gate: Gate,
) -> Option<bool> {
    let key = dir
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(key_from_dirname)?;
    let signal = read_raw_f32(&dir.join("audio.raw"))?;
    if signal.len() < BASS_WINDOW_SIZE {
        return None;
    }
    let f_et = NOTES[key as usize].frequency;
    // The prior B is what a cold-start app has: no capture has been measured
    // for this key yet, so the reference partials come from the Rigaud prior.
    let b = get_expected_beta(key);

    // One reference set, all partials, in the shipped convention and installed
    // exactly as the live app does.
    let mut refs = [0.0f32; MAX_STROBE_REFS];
    let count = strobe_refs(f_et, b, MAX_BASS_PARTIAL, &mut refs);
    let angles = strobe_angles(&signal, &refs, count);

    println!(
        "{:<4} key {:>2}  f_ET={:>9.3}  B_prior={:.3e}  ({} partials)",
        NOTES[key as usize].name, key, f_et, b, count
    );

    let mut any_pass = false;
    for n in 1..=count {
        let r_n = refs[n - 1];
        let truth_n = dtft_truth(&signal, r_n, planner).map(|f| cents(f, r_n));
        let (series, no_ref, med_ref) = spectral_series(
            &signal, r_n, f_et, // spacing ≈ f₀ — NOT r_n (the n>1 trap)
            8192, 0.001, span_cents, min_bins, gate, planner,
        );
        let s = score(&series, r_n);
        let err = truth_n.map(|t| s.median_cents - t);
        let band = slope_from_angles(&angles[n - 1], r_n, BAND_WIN_HOPS);

        // The criterion applies to partials above the fundamental only: n = 1
        // is the thing already known not to work down here.
        let pass = n >= 2
            && s.avail >= BASS_PASS_AVAIL
            && err.is_some_and(|e| e.abs() <= BASS_PASS_ERR_CENTS)
            && s.jitter <= BASS_PASS_JITTER_CENTS;
        any_pass |= pass;

        print!("   n={n} r={r_n:>9.2}  ");
        match truth_n {
            Some(t) => print!("truth {t:>+7.1}¢  "),
            None => print!("truth      --   "),
        }
        print!("pk8192 av{:>4.0}% ", s.avail * 100.0);
        match err {
            Some(e) if s.median_cents.is_finite() => print!("e{e:>+7.1} j{:>6.1}", s.jitter),
            _ => print!("e     -- j    --"),
        }
        if med_ref > 0 || no_ref > 0 {
            print!(
                "  [ref {med_ref:>3} bins, no-ref {:>3.0}%]",
                100.0 * no_ref as f32 / series.len().max(1) as f32
            );
        }
        match band {
            Some((m, j, run)) => print!("  band {m:>+7.1}¢ ±{j:<5.1} run{run:>4}"),
            None => print!("  band        --            "),
        }
        if pass {
            print!("  ✓PASS");
        }
        println!();
    }
    Some(any_pass)
}

/// **The candidate shipping gate**, settled by the round-2 flank sweep and
/// used by every measurement below so their results are mutually comparable.
///
/// 25th percentile, ±2 guard bins, flanking references with a **32-bin floor**,
/// exact finite-N multiplier. The floor is the load-bearing part: deep-bass
/// partials are ≈ 5 bins apart at 8192, so the natural `1.5 × spacing` flank
/// (≈ 8 bins) reaches only the strong low partials and the order statistic reads
/// signal as if it were noise — the threshold then rejects the true peak.
/// Widening to 32 bins spans partials ≈ 1–11 and brings in the weak upper ones,
/// which is what the low order statistic actually selects (`--refset`).
///
/// This is exactly the configuration Measurement A was mistakenly run without,
/// which is why its bass rows measured a broken gate rather than a policy.
fn shipping_gate_hz(flank_min_hz: f32) -> Gate {
    Gate::Cfar(CfarCfg {
        quantile: 0.25,
        guard_bins: 0,
        flanking: true,
        flank_min_hz,
        finite_n: true,
    })
}

/// [`shipping_gate_hz`] at the default flank floor.
fn shipping_gate() -> Gate {
    shipping_gate_hz(FLANK_MIN_HZ)
}

/// Reference-flank floor in Hz (see [`CfarCfg::flank_min_hz`]).
const FLANK_MIN_HZ: f32 = 172.0;

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
            .and_then(key_from_dirname)
        else {
            continue;
        };
        let Some(signal) = read_raw_f32(&dir.join("audio.raw")) else {
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
                .and_then(key_from_dirname)
            else {
                continue;
            };
            let Some(full) = read_raw_f32(&dir.join("audio_full_event.raw")) else {
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
        let Some(full) = read_raw_f32(&dir.join("audio_full_event.raw")) else {
            continue;
        };
        if full.len() < preroll {
            continue;
        }
        let quiet = &full[..preroll];
        let mut frame = ProcessingFrame::new();
        let mut gate = Gatekeeper::new(Arc::new(ArrayQueue::new(1)));
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

/// Per-hop reading and CFAR margin for one reference partial.
type HopRead = (Option<f32>, f32);

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
            .and_then(key_from_dirname)
        else {
            continue;
        };
        let Some(signal) = read_raw_f32(&dir.join("audio.raw")) else {
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
                .and_then(key_from_dirname)
            else {
                continue;
            };
            let Some(signal) = read_raw_f32(&dir.join("audio.raw")) else {
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

#[allow(clippy::too_many_arguments)]
fn multi_partial_series(
    signal: &[f32],
    refs: &[f32; MAX_STROBE_REFS],
    count: usize,
    spacing_hz: f32,
    fft_size: usize,
    noise_floor: f32,
    span_cents: f32,
    min_bins: f32,
    gate: Gate,
    planner: &mut RealFftPlanner<f32>,
) -> Vec<Vec<HopRead>> {
    let fftp = planner.plan_fft_forward(fft_size);
    let mut time = vec![0.0f32; fft_size];
    let mut spec = vec![Complex { re: 0.0, im: 0.0 }; fft_size / 2 + 1];
    let mut mag = vec![0.0f32; fft_size / 2];
    let mut scratch: Vec<f32> = Vec::new();
    let mut out: Vec<Vec<HopRead>> = Vec::new();
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
        let mut hop = Vec::with_capacity(count);
        for r in refs.iter().take(count) {
            let ro = spectral_read(
                &mag,
                &spec,
                fft_size,
                *r,
                spacing_hz,
                noise_floor,
                span_cents,
                min_bins,
                gate,
                &mut scratch,
            );
            hop.push((
                match ro.read {
                    Read::Hit(f) => Some(f),
                    _ => None,
                },
                ro.margin,
            ));
        }
        out.push(hop);
        cursor += HOP_SIZE;
    }
    out
}

/// **Measurement A — fixed n\* vs strongest-partial-per-hop.**
///
/// Both policies exploit the equal-cents identity: because `fₙ = n·f₀·√(1+Bn²)`
/// is linear in f₀, scaling the string by x cents scales every partial by
/// exactly x cents, so *any* partial's deviation from its own reference is the
/// string's deviation. The policies differ only in which partial supplies it.
///
/// The strongest policy picks, each hop, the admitted partial with the largest
/// CFAR margin (comparable across partials — each is normalized by its own
/// local noise). Reported per policy: availability, median cents, jitter; and
/// for the strongest policy the **switch rate** — the fraction of consecutive
/// admitted hops that changed partial, each of which steps the displayed
/// number by the reference error `866·ΔB·(n₂²−n₁²)` if B is imperfect.
#[allow(clippy::too_many_arguments)]
fn partial_policy(
    dir: &Path,
    planner: &mut RealFftPlanner<f32>,
    span_cents: f32,
    min_bins: f32,
    gate: Gate,
    use_measured_b: bool,
) -> Option<()> {
    let key = dir
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(key_from_dirname)?;
    let signal = read_raw_f32(&dir.join("audio.raw"))?;
    if signal.len() < BASS_WINDOW_SIZE {
        return None;
    }
    let f_et = NOTES[key as usize].frequency;
    let b_prior = get_expected_beta(key);

    // Optionally re-derive B from the capture's own partial truths, so the
    // references are right and the partial comparison is not confounded by the
    // prior's error (which the earlier run fitted at ≈ 4.7× on this piano).
    let mut refs = [0.0f32; MAX_STROBE_REFS];
    let n_refs = strobe_refs(f_et, b_prior, MAX_BASS_PARTIAL, &mut refs);
    let b_used = if use_measured_b {
        let mut obs: Vec<(usize, f32)> = Vec::new();
        for (i, r) in refs.iter().take(n_refs).enumerate() {
            if let Some(f) = dtft_truth(&signal, *r, planner) {
                obs.push((i + 1, f));
            }
        }
        fit_f0_b(&obs).map(|(_, b)| b).unwrap_or(b_prior)
    } else {
        b_prior
    };
    let count = strobe_refs(f_et, b_used, MAX_BASS_PARTIAL, &mut refs);

    let n_star = curves::default_display_partials()[key as usize] as usize;
    let series = multi_partial_series(
        &signal, &refs, count, f_et, 8192, 0.001, span_cents, min_bins, gate, planner,
    );

    // Truth in string-relative cents: any partial's own deviation works, so
    // take the median across partials of cents(truth_n, r_n).
    let mut truths: Vec<f32> = (0..count)
        .filter_map(|i| dtft_truth(&signal, refs[i], planner).map(|f| cents(f, refs[i])))
        .collect();
    truths.sort_by(f32::total_cmp);
    let truth_c = truths.get(truths.len() / 2).copied();

    // Fixed n* policy.
    let fixed: Vec<Option<f32>> = series
        .iter()
        .map(|hop| {
            hop.get(n_star - 1)
                .and_then(|(f, _)| f.map(|f| cents(f, refs[n_star - 1])))
        })
        .collect();

    // Strongest-margin policy, tracking partial switches AND the cents step
    // each switch puts on screen — the user-visible cost, whose analytic form
    // is 866·ΔB·(n₂²−n₁²) when the reference B is imperfect. A switch rate
    // alone cannot say whether switching is harmless or a visible jump.
    let mut strongest: Vec<Option<f32>> = Vec::with_capacity(series.len());
    let mut switches = 0usize;
    let mut admitted = 0usize;
    let mut steps: Vec<f32> = Vec::new();
    let mut prev_pick: Option<usize> = None;
    let mut prev_cents: Option<f32> = None;
    for hop in &series {
        let pick = hop
            .iter()
            .enumerate()
            .filter(|(_, (f, _))| f.is_some())
            .max_by(|a, b| a.1.1.total_cmp(&b.1.1))
            .map(|(i, _)| i);
        match pick {
            Some(i) => {
                admitted += 1;
                let c = hop[i].0.map(|f| cents(f, refs[i]));
                if let Some(p) = prev_pick
                    && p != i
                {
                    switches += 1;
                    if let (Some(a), Some(b)) = (prev_cents, c) {
                        steps.push((b - a).abs());
                    }
                }
                prev_pick = Some(i);
                prev_cents = c;
                strongest.push(c);
            }
            None => {
                prev_pick = None;
                prev_cents = None;
                strongest.push(None);
            }
        }
    }
    steps.sort_by(f32::total_cmp);
    let (med_step, max_step) = if steps.is_empty() {
        (f32::NAN, f32::NAN)
    } else {
        (steps[steps.len() / 2], *steps.last().unwrap())
    };

    let stat = |s: &[Option<f32>]| -> (f32, f32, f32) {
        let v: Vec<f32> = s
            .iter()
            .filter_map(|x| *x)
            .filter(|x| x.is_finite())
            .collect();
        let avail = v.len() as f32 / s.len().max(1) as f32;
        if v.is_empty() {
            return (avail, f32::NAN, f32::NAN);
        }
        let mut sorted = v.clone();
        sorted.sort_by(f32::total_cmp);
        let med = sorted[sorted.len() / 2];
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        let jit = (v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / v.len() as f32).sqrt();
        (avail, med, jit)
    };

    let (fa, fm, fj) = stat(&fixed);
    let (sa, sm, sj) = stat(&strongest);
    let e = |m: f32| truth_c.map(|t| m - t).unwrap_or(f32::NAN);

    print!(
        "{:<4} key {:>2}  B={:.3e}{}  n*={n_star}  ",
        NOTES[key as usize].name,
        key,
        b_used,
        if use_measured_b { "*" } else { " " }
    );
    match truth_c {
        Some(t) => print!("truth {t:>+7.1}¢ | "),
        None => print!("truth      --  | "),
    }
    println!(
        "fixed av{:>4.0}% e{:>+7.1} j{:>6.1} | strongest av{:>4.0}% e{:>+7.1} j{:>6.1} sw{:>5.1}% step med{:>6} max{:>6}",
        fa * 100.0,
        e(fm),
        fj,
        sa * 100.0,
        e(sm),
        sj,
        100.0 * switches as f32 / admitted.max(1) as f32,
        if med_step.is_finite() {
            format!("{med_step:.1}¢")
        } else {
            "--".into()
        },
        if max_step.is_finite() {
            format!("{max_step:.1}¢")
        } else {
            "--".into()
        }
    );
    Some(())
}

/// **Measurement B — per-partial scores with references built from `b`.**
/// Same shape as [`bass_partials`] but takes the B to use, so the fixed-n
/// table can be re-run with the capture's own measured B.
fn partial_table_row(
    dir: &Path,
    planner: &mut RealFftPlanner<f32>,
    span_cents: f32,
    min_bins: f32,
    use_measured_b: bool,
) -> Option<Vec<(usize, f32, f32, f32)>> {
    let key = dir
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(key_from_dirname)?;
    let signal = read_raw_f32(&dir.join("audio.raw"))?;
    if signal.len() < BASS_WINDOW_SIZE {
        return None;
    }
    let f_et = NOTES[key as usize].frequency;
    let b_prior = get_expected_beta(key);
    let mut refs = [0.0f32; MAX_STROBE_REFS];
    let n0 = strobe_refs(f_et, b_prior, MAX_BASS_PARTIAL, &mut refs);
    let b_used = if use_measured_b {
        let mut obs: Vec<(usize, f32)> = Vec::new();
        for (i, r) in refs.iter().take(n0).enumerate() {
            if let Some(f) = dtft_truth(&signal, *r, planner) {
                obs.push((i + 1, f));
            }
        }
        fit_f0_b(&obs).map(|(_, b)| b).unwrap_or(b_prior)
    } else {
        b_prior
    };
    let count = strobe_refs(f_et, b_used, MAX_BASS_PARTIAL, &mut refs);

    let mut rows = Vec::new();
    for n in 1..=count {
        let r_n = refs[n - 1];
        let truth_n = dtft_truth(&signal, r_n, planner).map(|f| cents(f, r_n));
        let (series, _, _) = spectral_series(
            &signal,
            r_n,
            f_et,
            8192,
            0.001,
            span_cents,
            min_bins,
            Gate::Ambient,
            planner,
        );
        let s = score(&series, r_n);
        let err = truth_n.map(|t| s.median_cents - t).unwrap_or(f32::NAN);
        rows.push((n, s.avail, err, s.jitter));
    }
    Some(rows)
}

/// The gate variants compared by [`gate_ab`]: the shipped ambient-σ control
/// plus the four OS-CFAR corners (median vs 25th percentile × in-band vs
/// flanking reference cells), all at ±2 guard bins.
fn gate_variants() -> Vec<Gate> {
    let mut v = vec![Gate::Ambient];
    // In-band control (known degenerate in the deep bass), then the flank-floor
    // sweep in Hz at the two order statistics that matter.
    v.push(Gate::Cfar(CfarCfg {
        quantile: 0.5,
        guard_bins: 2, // the one variant a guard can matter for: refs inside the band
        flanking: false,
        flank_min_hz: 172.0,
        finite_n: true,
    }));
    for &hz in &[86.0f32, 172.0, 344.0, 688.0] {
        for &q in &[0.25f32, 0.5] {
            v.push(Gate::Cfar(CfarCfg {
                quantile: q,
                guard_bins: 0,
                flanking: true,
                flank_min_hz: hz,
                finite_n: true,
            }));
        }
    }
    v
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
                .and_then(key_from_dirname)
            else {
                continue;
            };
            let Some(signal) = read_raw_f32(&dir.join("audio.raw")) else {
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

/// **Reference-offset reach.** The live substitute for detuning a real piano:
/// hold the capture fixed and move the *reference* instead. A reference `x` ¢
/// below the string is indistinguishable, to the bounded search, from a string
/// `x` ¢ above the reference — so this measures how far off pitch the coarse
/// read still works, on real audio, without touching an instrument.
///
/// Reports availability and |read − DFT truth| per offset. The band's own limit
/// is printed alongside: it hands over at `BAND_READABLE_HZ` = 18 Hz, which in
/// cents is ≈ 37200/f, so the coarse read only *adds* range where that is narrow.
/// **T6 — regime-switch chatter.** `main_view` shows the band-slope read while
/// `!gated && !out_of_range && band_cents.is_some()`, and the coarse read
/// otherwise. `out_of_range` is decided from the **coarse** read's cents,
/// converted to Hz at the displayed reference and compared with
/// `BAND_READABLE_HZ` — so near the boundary the decision is made by an
/// estimator whose own treble error is comparable to the 3.5 Hz margin it
/// protects, and the displayed *source* can flip hop to hop.
///
/// Sweeps the reference offset across each key's boundary and reports how often
/// the source changes between consecutive hops. Assumes the band is ungated and
/// filled, which is the worst case for chatter and the normal case in a treble
/// sustain; a `None` coarse read leaves `out_of_range` false and so shows the
/// band, exactly as shipped, and is counted as a source change too.
fn switch_chatter(caps: &[PathBuf], planner: &mut RealFftPlanner<f32>) {
    const BAND_READABLE_HZ: f32 = 18.0; // mirrors tuner-gui/src/views/main_view.rs
    println!(
        "Readout-source chatter at the band/coarse boundary. Offsets are relative to each\n\
         key's own boundary (1.0 = exactly at it). flip% = consecutive hops that changed\n\
         source; drop% = hops with no coarse read (which shows the band by default).\n"
    );
    println!(
        "{:>4} {:>5} {:>8} | {:>34} | {:>6}",
        "key", "note", "bound ¢", "flip% | aliased-hold% : none/8,8/1,8/2,8/4,8", "drop%"
    );

    let table = curves::default_display_partials();
    for dir in caps {
        let Some(key) = dir
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(key_from_dirname)
        else {
            continue;
        };
        let Some(signal) = read_raw_f32(&dir.join("audio.raw")) else {
            continue;
        };
        if signal.len() < BASS_WINDOW_SIZE {
            continue;
        }
        let f_et = NOTES[key as usize].frequency;
        let b = get_expected_beta(key);
        let n_coarse = curves::coarse_read_partial(key) as usize;
        let n_disp = table[key as usize] as usize;
        let mut refs = [0.0f32; MAX_STROBE_REFS];
        let count = strobe_refs(f_et, b, MAX_STROBE_REFS, &mut refs);
        if n_coarse > count || n_disp > count {
            continue;
        }
        let coarse_center = refs[n_coarse - 1];
        let disp_center = refs[n_disp - 1];
        let spacing = f_et / (1.0 + b).sqrt();
        // The boundary in cents: the offset whose Hz-equivalent at the displayed
        // reference equals BAND_READABLE_HZ.
        let bound_c = 1200.0 * (1.0 + BAND_READABLE_HZ / disp_center).log2();

        let fftp = planner.plan_fft_forward(BASS_WINDOW_SIZE);
        let mut time = vec![0.0f32; BASS_WINDOW_SIZE];
        let mut spec = vec![Complex { re: 0.0, im: 0.0 }; BASS_WINDOW_SIZE / 2 + 1];
        let mut mag = vec![0.0f32; BASS_WINDOW_SIZE / 2];
        let mut scratch = vec![0.0f32; BASS_WINDOW_SIZE / 2];

        // Verdicts are pooled over offsets straddling the boundary — the state
        // the chatter lives in. Each offset is replayed independently.
        // (hops to switch TO coarse, hops to switch BACK to the band)
        const VARIANTS: [(usize, usize); 5] = [(1, 1), (8, 8), (1, 8), (2, 8), (4, 8)];
        let mut flips = [0usize; VARIANTS.len()];
        let mut stale = [0usize; VARIANTS.len()];
        let mut pairs = 0usize;
        let (mut drops, mut hops_total) = (0usize, 0usize);
        for mult in [0.9f32, 1.0, 1.1] {
            let off = mult * bound_c;
            let c_off = coarse_center * 2f32.powf(-off / 1200.0);
            let d_off = disp_center * 2f32.powf(-off / 1200.0);
            let s_off = spacing * 2f32.powf(-off / 1200.0);
            let mut verdicts: Vec<bool> = Vec::new();
            let mut cursor = 0usize;
            while cursor + BASS_WINDOW_SIZE <= signal.len() {
                let end = cursor + BASS_WINDOW_SIZE;
                cursor += HOP_SIZE;
                fft(
                    &signal[end - BASS_WINDOW_SIZE..end],
                    &mut time,
                    &mut spec,
                    &fftp,
                    BASS_WINDOW_SIZE,
                );
                magnitude_spectrum(&spec, BASS_WINDOW_SIZE, &mut mag);
                let read = peaks::coarse_read(
                    &mag,
                    &spec,
                    BASS_WINDOW_SIZE,
                    SAMPLE_RATE,
                    c_off,
                    s_off,
                    &mut scratch,
                );
                hops_total += 1;
                // The shipped predicate, verbatim: no coarse read ⇒ in range.
                verdicts.push(match read {
                    Some(hz) => {
                        let cents_off = 1200.0 * (hz / c_off).log2();
                        (d_off * ((cents_off / 1200.0).exp2() - 1.0)).abs() >= BAND_READABLE_HZ
                    }
                    None => {
                        drops += 1;
                        false
                    }
                });
            }
            if verdicts.len() < 2 {
                continue;
            }
            pairs += verdicts.len() - 1;
            // Debounce, in three symmetries. `(m_out, m_back)` = hops of opposing
            // evidence needed to switch *to* coarse and back *to* the band.
            // Asymmetric variants matter because holding the band past the
            // boundary means displaying an aliased number, while holding the
            // coarse read merely means displaying a jitterier true one.
            for (i, &(m_out, m_back)) in VARIANTS.iter().enumerate() {
                let mut state = verdicts[0];
                let mut run = 0usize;
                for &v in &verdicts[1..] {
                    if v == state {
                        run = 0;
                    } else {
                        run += 1;
                        let need = if v { m_out } else { m_back };
                        if run >= need {
                            state = v;
                            run = 0;
                            flips[i] += 1;
                        }
                    }
                }
                // Exposure: hops displaying the band while the verdict says the
                // band is aliased — the cost the out-ward debounce buys.
                let mut state = verdicts[0];
                let mut run = 0usize;
                for &v in &verdicts[1..] {
                    if v == state {
                        run = 0;
                    } else {
                        run += 1;
                        let need = if v { m_out } else { m_back };
                        if run >= need {
                            state = v;
                            run = 0;
                        }
                    }
                    if v && !state {
                        stale[i] += 1;
                    }
                }
            }
        }
        if pairs == 0 {
            continue;
        }
        let pct = |i: usize| 100.0 * flips[i] as f32 / pairs as f32;
        let st = |i: usize| 100.0 * stale[i] as f32 / pairs as f32;
        println!(
            "{:>4} {:>5} {:>7.1} | {} | {:>5.0}",
            key,
            NOTES[key as usize].name,
            bound_c,
            (0..5)
                .map(|i| format!("{:>5.1}|{:<5.1}", pct(i), st(i)))
                .collect::<Vec<_>>()
                .join(" "),
            100.0 * drops as f32 / hops_total.max(1) as f32
        );
    }
}

fn reach_sweep(caps: &[PathBuf], planner: &mut RealFftPlanner<f32>) {
    println!(
        "Reference-offset reach — how far off pitch the coarse read still reads.\n\
              Offsetting the reference == detuning the string, on real capture audio.\n"
    );
    let offsets = [0.0f32, 10.0, 25.0, 50.0, 75.0, 100.0, 150.0];
    print!("{:>4} {:>5} {:>9} |", "key", "note", "band to");
    for o in offsets {
        print!(" {:>13}", format!("{o:.0} c"));
    }
    println!("\n{:->4} {:->5} {:->9} |{:->98}", "", "", "", "");

    for dir in caps {
        let Some(key) = dir
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(key_from_dirname)
        else {
            continue;
        };
        let Some(signal) = read_raw_f32(&dir.join("audio.raw")) else {
            continue;
        };
        if signal.len() < BASS_WINDOW_SIZE {
            continue;
        }
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
        let Some(truth) = dtft_truth(&signal, center, planner) else {
            continue;
        };
        let band_c = 1200.0 * ((center - 18.0).max(1.0) / center).log2();

        print!(
            "{:>4} {:>5} {:>8.0}c |",
            key, NOTES[key as usize].name, band_c
        );
        for &off in &offsets {
            // Reference moved DOWN by `off` cents == string `off` cents sharp of it.
            let c_off = center * 2f32.powf(-off / 1200.0);
            let s_off = spacing * 2f32.powf(-off / 1200.0);
            let fftp = planner.plan_fft_forward(BASS_WINDOW_SIZE);
            let mut time = vec![0.0f32; BASS_WINDOW_SIZE];
            let mut spec = vec![Complex { re: 0.0, im: 0.0 }; BASS_WINDOW_SIZE / 2 + 1];
            let mut mag = vec![0.0f32; BASS_WINDOW_SIZE / 2];
            let mut scratch = vec![0.0f32; BASS_WINDOW_SIZE / 2];
            let (mut hits, mut hops, mut err) = (0usize, 0usize, Vec::new());
            let mut cursor = 0usize;
            while cursor + BASS_WINDOW_SIZE <= signal.len() {
                let end = cursor + BASS_WINDOW_SIZE;
                fft(
                    &signal[end - BASS_WINDOW_SIZE..end],
                    &mut time,
                    &mut spec,
                    &fftp,
                    BASS_WINDOW_SIZE,
                );
                magnitude_spectrum(&spec, BASS_WINDOW_SIZE, &mut mag);
                cursor += HOP_SIZE;
                hops += 1;
                if let Some(hz) = peaks::coarse_read(
                    &mag,
                    &spec,
                    BASS_WINDOW_SIZE,
                    SAMPLE_RATE,
                    c_off,
                    s_off,
                    &mut scratch,
                ) {
                    hits += 1;
                    err.push((1200.0 * (hz / truth).log2()).abs());
                }
            }
            if hits == 0 {
                print!("      --     ");
                continue;
            }
            err.sort_by(f32::total_cmp);
            print!(
                " {:>4.0}% {:>5.1}c",
                100.0 * hits as f32 / hops as f32,
                err[err.len() / 2]
            );
        }
        println!();
    }
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
        .and_then(key_from_dirname)?;
    let signal = read_raw_f32(&dir.join("audio.raw"))?;
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

/// **Q4 — does a readout survive a fast-moving string?** Synthesizes a note
/// whose f₀ glides at a known rate (turning a peg) and scores each method
/// against the known instantaneous truth **at the newest sample** — "what is
/// the string doing now", the only epoch a live readout is judged on.
///
/// Two effects separate in this table. Every method is centred on its own
/// analysis window, so it necessarily lags the newest sample by `win/2`
/// samples (1024 → 11.6 ms, 2048 → 23.2, 4096 → 46.4, 8192 → 92.9); against a
/// glide of `R` ¢/s that shows up as a floor of `R·win/(2·fs)` cents, and that
/// floor **is** the latency cost of the window (Prompt N open question 2).
/// Errors far above the floor are the second effect: the adaptive tracker's
/// EMA (α = 0.05, τ ≈ 0.46 s) losing the string, whereupon `|f_live − f_target|`
/// passes the ±21.5 Hz unwrap limit and the reading aliases. A fixed-reference
/// spectral search carries no such state and cannot fail that way.
fn detune_sweep(span_cents: f32, min_bins: f32, gate: Gate, planner: &mut RealFftPlanner<f32>) {
    println!("Fast-detune survival — synthetic glide from the reference, 2.0 s per run.");
    println!(
        "Error = median |read − true at the newest sample| (¢); (%) = availability.\n\
         Expected floor = window group delay × rate: at 100 ¢/s that is 1.2 ¢ (1024), \
         2.3 (2048), 4.6 (4096), 9.3 (8192).\n\
         Errors far above the floor = the adaptive tracker aliasing after its EMA lost the string.\n"
    );
    let secs = 2.0f32;
    let len = (SAMPLE_RATE as f32 * secs) as usize;
    let amps = [0.6f32, 1.0, 0.7, 0.45, 0.25];

    for &(key, name) in &[(19u8, "E2"), (43, "E4"), (48, "A4")] {
        let f_ref = NOTES[key as usize].frequency;
        println!("── {name} (ref {f_ref:.2} Hz) ──");
        println!(
            "{:>10}  {:>22}  {:>22}  {:>22}  {:>22}",
            "rate ¢/s", "trkFIX (adaptive)", "pk2048", "pk8192", "dual (tier-1)"
        );
        for &rate in &[0.0f32, 50.0, 100.0, 200.0, 400.0] {
            // f(t) = f_ref·2^(rate·t/1200); phase = ∫2π f dt integrated exactly.
            let k = rate / 1200.0;
            let signal: Vec<f32> = (0..len)
                .map(|i| {
                    let t = i as f32 / SAMPLE_RATE as f32;
                    let mut s = 0.0;
                    for (j, &a) in amps.iter().enumerate() {
                        let n = (j + 1) as f32;
                        // ∫₀ᵗ f_ref·2^(k·u) du = f_ref·(2^(k t) − 1)/(k·ln2)
                        let ph = if k.abs() < 1e-9 {
                            f_ref * t
                        } else {
                            f_ref * (2f32.powf(k * t) - 1.0) / (k * std::f32::consts::LN_2)
                        };
                        s += a * (TAU * n * ph).sin();
                    }
                    0.1 * s
                })
                .collect();

            let win = register_window(f_ref);
            let s8: Vec<Option<f32>> = spectral_series(
                &signal, f_ref, f_ref, 8192, 0.001, span_cents, min_bins, gate, planner,
            )
            .0;
            let s2: Vec<Option<f32>> = spectral_series(
                &signal, f_ref, f_ref, 2048, 0.001, span_cents, min_bins, gate, planner,
            )
            .0;
            // ── Tier-1 dual-window selection ──────────────────────────────
            // Both spectra are computed every hop anyway, so read both and
            // prefer 8192 when it is admitted, falling back to 2048. No
            // constants and no state: 8192's bins smear when the tone moves,
            // so its own availability collapse IS the motion signal. `churn`
            // counts hops whose source differs from the previous hop — the
            // mixed-source artifact, where a 93 ms-lagged read can sit beside
            // a 23 ms one.
            let mut dual: Vec<Option<f32>> = Vec::with_capacity(s8.len());
            let (mut churn, mut prev_src, mut picks) = (0usize, None::<u8>, 0usize);
            // Per-source error, so a surprising pooled median can be attributed:
            // an accounting bug would show each source matching its own column,
            // whereas a *selection* effect shows the 2048-sourced subset worse
            // than 2048's own column — those are exactly the hops where 8192
            // rejected, i.e. where the tone was smearing hardest.
            let (mut e8, mut e2): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
            for (h, (a, b)) in s8.iter().zip(s2.iter()).enumerate() {
                let (v, src) = match (a, b) {
                    (Some(x), _) => (Some(*x), Some(0u8)),
                    (None, Some(y)) => (Some(*y), Some(1u8)),
                    (None, None) => (None, None),
                };
                if let (Some(f), Some(sc)) = (v, src) {
                    let t_now = (h * HOP_SIZE + BASS_WINDOW_SIZE) as f32 / SAMPLE_RATE as f32;
                    let f_true = f_ref * 2f32.powf(k * t_now);
                    let err = (cents(f, f_ref) - cents(f_true, f_ref)).abs();
                    if sc == 0 { e8.push(err) } else { e2.push(err) }
                }
                if let Some(sc) = src {
                    picks += 1;
                    if prev_src.is_some_and(|p| p != sc) {
                        churn += 1;
                    }
                    prev_src = Some(sc);
                } else {
                    prev_src = None;
                }
                dual.push(v);
            }
            let churn_pct = 100.0 * churn as f32 / picks.max(1) as f32;
            let med = |v: &mut Vec<f32>| -> f32 {
                if v.is_empty() {
                    return f32::NAN;
                }
                v.sort_by(f32::total_cmp);
                v[v.len() / 2]
            };
            let (n8, n2) = (e8.len(), e2.len());
            let (m8, m2) = (med(&mut e8), med(&mut e2));
            // Percentiles of the pooled dual errors: a pooled median far above
            // both source medians can only come from overlapping spreads, and
            // the spread is what a readout actually shows the user.
            let mut pooled: Vec<f32> = e8.iter().chain(e2.iter()).copied().collect();
            pooled.sort_by(f32::total_cmp);
            let pct = |v: &[f32], q: f32| -> f32 {
                if v.is_empty() {
                    return f32::NAN;
                }
                v[(((v.len() - 1) as f32) * q).round() as usize]
            };
            let (p10, p50, p90) = (pct(&pooled, 0.1), pct(&pooled, 0.5), pct(&pooled, 0.9));
            let (s2p10, s2p90) = (pct(&e2, 0.1), pct(&e2, 0.9));
            let series: [(&str, Vec<Option<f32>>); 4] = [
                ("trkFIX", tracker_series(&signal, f_ref, win, 0.001)),
                (
                    "pk2048",
                    spectral_series(
                        &signal, f_ref, f_ref, 2048, 0.001, span_cents, min_bins, gate, planner,
                    )
                    .0,
                ),
                ("pk8192", s8.clone()),
                ("dual", dual),
            ];
            print!("{rate:>10.0}");
            let _ = churn_pct;
            for (_, s) in &series {
                // Truth at the hop's window centre — the estimate's own epoch.
                let mut errs: Vec<f32> = Vec::new();
                let mut hits = 0usize;
                for (h, v) in s.iter().enumerate() {
                    let Some(f) = v else { continue };
                    hits += 1;
                    // The newest sample in this hop's COLA window — every
                    // method saw exactly this much audio, so scoring here
                    // charges each its own group delay rather than a shared one.
                    let t_now = (h * HOP_SIZE + BASS_WINDOW_SIZE) as f32 / SAMPLE_RATE as f32;
                    let f_true = f_ref * 2f32.powf(k * t_now);
                    errs.push((cents(*f, f_ref) - cents(f_true, f_ref)).abs());
                }
                if errs.is_empty() {
                    print!("  {:>22}", "-- (0%)");
                } else {
                    errs.sort_by(f32::total_cmp);
                    let med = errs[errs.len() / 2];
                    print!(
                        "  {:>13}{:>9}",
                        format!("{med:.1}¢"),
                        format!("({:.0}%)", 100.0 * hits as f32 / s.len() as f32)
                    );
                }
            }
            println!(
                "   churn {churn_pct:>4.0}%  src: 8192 {n8}@{}  2048 {n2}@{}  \
                 2048spread[{}..{}]  pooled p10/p50/p90 {}/{}/{}",
                if m8.is_finite() {
                    format!("{m8:.1}")
                } else {
                    "--".into()
                },
                if m2.is_finite() {
                    format!("{m2:.1}")
                } else {
                    "--".into()
                },
                if s2p10.is_finite() {
                    format!("{s2p10:.1}")
                } else {
                    "--".into()
                },
                if s2p90.is_finite() {
                    format!("{s2p90:.1}")
                } else {
                    "--".into()
                },
                if p10.is_finite() {
                    format!("{p10:.1}")
                } else {
                    "--".into()
                },
                if p50.is_finite() {
                    format!("{p50:.1}")
                } else {
                    "--".into()
                },
                if p90.is_finite() {
                    format!("{p90:.1}")
                } else {
                    "--".into()
                },
            );
        }
        println!();
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut dir = PathBuf::from("diagnostics");
    let mut key_filter: Option<Vec<u8>> = None;
    let mut alias = false;
    let mut window = false;
    let mut readout = false;
    let mut bass_partials_mode = false;
    let mut gate_ab_mode = false;
    let mut gate_ab_partial = 1usize;
    let mut policy_mode = false;
    let mut measured_b = false;
    let mut cfar_profile_mode = false;
    let mut verify_shipped_mode = false;
    let mut reach_mode = false;
    let mut refset_mode = false;
    let mut chatter_mode = false;
    let mut pfa_mode = false;
    let mut fft_size = 8192usize;
    let mut flank_hz = FLANK_MIN_HZ;
    let mut profile_max_n = MAX_BASS_PARTIAL;
    // Coarse-read search band, swept from the command line so the choice is
    // measured rather than asserted (Prompt N open question 3).
    let mut span_cents = 100.0f32;
    let mut min_bins = 4.0f32;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--selftest" => {
                selftest(&mut RealFftPlanner::<f32>::new());
                return;
            }
            "--inharm" => {
                inharm_sweep();
                return;
            }
            "--detune" => {
                detune_sweep(
                    span_cents,
                    min_bins,
                    shipping_gate_hz(flank_hz),
                    &mut RealFftPlanner::<f32>::new(),
                );
                return;
            }
            "--alias" => alias = true,
            "--window" => window = true,
            "--readout" => readout = true,
            "--bass-partials" => bass_partials_mode = true,
            "--gate-ab" => gate_ab_mode = true,
            "--policy" => policy_mode = true,
            "--cfar-profile" => cfar_profile_mode = true,
            "--verify-shipped" => verify_shipped_mode = true,
            "--reach" => reach_mode = true,
            "--refset" => refset_mode = true,
            "--chatter" => chatter_mode = true,
            "--flank-hz" => {
                i += 1;
                flank_hz = args
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(FLANK_MIN_HZ);
            }
            "--fft" => {
                i += 1;
                fft_size = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(8192);
            }
            "--max-n" => {
                i += 1;
                profile_max_n = args
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(MAX_BASS_PARTIAL);
            }
            "--pfa" => pfa_mode = true,
            "--measured-b" => measured_b = true,
            "--partial" => {
                i += 1;
                gate_ab_partial = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(1);
            }
            "--span" => {
                i += 1;
                span_cents = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(100.0);
            }
            "--min-bins" => {
                i += 1;
                min_bins = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(4.0);
            }
            "--keys" => {
                i += 1;
                key_filter = args
                    .get(i)
                    .map(|s| s.split(',').filter_map(|k| k.trim().parse().ok()).collect());
            }
            other => dir = PathBuf::from(other),
        }
        i += 1;
    }

    println!(
        "Pitch ground-truth audit — app (our hot path) vs truth (hi-res DFT) vs yin (autocorr).\n\
         cents shown are vs equal temperament (f_ET). Negative = flat.\n\
         The decisive column is app−truth; yin is the 'other tuners' family.\n"
    );

    let mut planner = RealFftPlanner::<f32>::new();
    let mut caps: Vec<PathBuf> = if dir.join("audio.raw").exists() {
        vec![dir.clone()]
    } else {
        let mut v: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.is_dir() && p.join("audio.raw").exists())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    };
    if let Some(keys) = &key_filter {
        caps.retain(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .and_then(key_from_dirname)
                .is_some_and(|k| keys.contains(&k))
        });
    }

    if caps.is_empty() {
        eprintln!("No captures found under {}", dir.display());
        return;
    }

    if alias {
        for cap in &caps {
            alias_sweep(cap, &mut planner);
        }
        return;
    }

    if window {
        println!("Fit-window sweep — longest ungated run and the band read per window length.\n");
        for cap in &caps {
            window_sweep(cap, &mut planner);
        }
        return;
    }

    if readout {
        println!(
            "Three-way readout comparison — tracker as-is / tracker + Defect-1 window / \
             bounded spectral peak.\n\
             av = availability over the whole note / over its first third (%); \
             e = median reading − truth (¢); j = jitter (¢).\n\
             * marks a key where the register rule selects the long tracker window. \
             Search band ±{span_cents:.0} ¢, floor {min_bins:.0} bins.\n"
        );
        for cap in &caps {
            readout_compare(cap, &mut planner, span_cents, min_bins);
        }
        return;
    }

    if cfar_profile_mode {
        println!(
            "T1 — per-key × per-partial profile under the settled gate \
             (os25 / ±2 guard / flank floor {FLANK_MIN_HZ:.0} Hz / finite-N, search-loss corrected) \
             at FFT {fft_size}.\n\
             Rows are medians over each key's repeat captures. margin = peak ÷ threshold \
             (> 1 admits; near 1 = one strike away from flipping).\n\
             ✓ marks a (key, partial) meeting the criterion: avail ≥ 90 %, |e| ≤ 2 ¢, jitter ≤ 10 ¢.\n"
        );
        cfar_profile(
            &caps,
            &mut planner,
            span_cents,
            min_bins,
            fft_size,
            profile_max_n,
        );
        return;
    }

    if verify_shipped_mode {
        verify_shipped(&caps, &mut planner);
        return;
    }

    if reach_mode {
        reach_sweep(&caps, &mut planner);
        return;
    }

    if chatter_mode {
        println!("T6 — does the band/coarse regime switch chatter near its boundary?\n");
        switch_chatter(&caps, &mut planner);
        return;
    }

    if refset_mode {
        println!(
            "T5 — reference-set anatomy: is the selected order statistic a valley cell or a \
             weak partial's lobe, and does the guard buy anything (Rohling §V)?\n"
        );
        ref_anatomy(&caps, &mut planner, span_cents, min_bins, fft_size);
        return;
    }

    if pfa_mode {
        println!(
            "T3 — realized false-alarm rate of the settled gate, measured on signal-free input.\n"
        );
        pfa_calibration(&caps, &mut planner, span_cents, min_bins, fft_size);
        return;
    }

    if policy_mode {
        println!(
            "Partial-selection policy — fixed n* (register table) vs strongest-margin-per-hop.\n\
             Both read the same 8192 spectra; cents are string-relative via the equal-cents \
             identity. References use {} B.\n\
             switch%% = fraction of consecutive admitted hops that changed partial (each one \
             steps the displayed number when B is imperfect).\n",
            if measured_b {
                "the capture's own MEASURED"
            } else {
                "the Rigaud prior"
            }
        );
        let gate = shipping_gate();
        for cap in &caps {
            partial_policy(cap, &mut planner, span_cents, min_bins, gate, measured_b);
        }
        return;
    }

    if measured_b && !bass_partials_mode {
        // `--measured-b` alone: the fixed-n table re-run with corrected refs.
        println!(
            "Fixed-n table with the capture's own MEASURED B (fitted from the partial truths) \
             vs the Rigaud prior.\n\
             Does the D5 register table's deep-bass n* = 6 clean up once its reference is right?\n"
        );
        let mut agg: Vec<Vec<(f32, f32)>> = vec![Vec::new(); MAX_BASS_PARTIAL];
        for cap in &caps {
            for use_meas in [false, true] {
                if let Some(rows) =
                    partial_table_row(cap, &mut planner, span_cents, min_bins, use_meas)
                {
                    for (n, _av, e, j) in rows {
                        if use_meas && e.is_finite() {
                            agg[n - 1].push((e.abs(), j));
                        }
                    }
                }
            }
        }
        println!("  MEASURED-B aggregate over {} captures:", caps.len());
        for (i, v) in agg.iter().enumerate() {
            if v.is_empty() {
                continue;
            }
            let mut es: Vec<f32> = v.iter().map(|x| x.0).collect();
            let mut js: Vec<f32> = v.iter().map(|x| x.1).collect();
            es.sort_by(f32::total_cmp);
            js.sort_by(f32::total_cmp);
            println!(
                "    n={}  median|e| {:>6.2}¢   median j {:>7.2}¢   ({} rows)",
                i + 1,
                es[es.len() / 2],
                js[js.len() / 2],
                v.len()
            );
        }
        return;
    }

    if bass_partials_mode {
        println!(
            "Partial-centered bass read — 8192 bounded search at each partial's PRIOR-B target.\n\
             Reference partials installed as ONE strobe ref set (the live path). Search band \
             ±{span_cents:.0} ¢, floor {min_bins:.0} bins, neighbour cap f₀/2.\n\
             Pre-registered criterion (fixed before the run): some n ≥ 2 with availability \
             ≥ {:.0} %, |median − truth| ≤ {:.0} ¢, jitter ≤ {:.0} ¢.\n",
            BASS_PASS_AVAIL * 100.0,
            BASS_PASS_ERR_CENTS,
            BASS_PASS_JITTER_CENTS
        );
        let mut passed = 0usize;
        let mut total = 0usize;
        for cap in &caps {
            if let Some(p) = bass_partials(cap, &mut planner, span_cents, min_bins, Gate::Ambient) {
                total += 1;
                passed += usize::from(p);
            }
        }
        println!(
            "\nCRITERION: {passed}/{total} captures had at least one partial n ≥ 2 meeting all three bars."
        );
        return;
    }

    if gate_ab_mode {
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
        for cap in &caps {
            gate_ab(
                cap,
                &mut planner,
                span_cents,
                min_bins,
                gate_ab_partial,
                fft_size,
            );
        }
        return;
    }

    for cap in &caps {
        process_capture(cap, &mut planner);
    }
    println!("\n{} capture(s).", caps.len());
}
