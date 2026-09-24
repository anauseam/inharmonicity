//! # The independent reference, and the gates read through it
//!
//! What every scored mode needs and none of them owns: a frequency estimate
//! made offline that the hot path had no part in, plus the detection
//! thresholds a reading passes through on its way to the panel.
//!
//! Three estimates of one note, by construction independent:
//!
//! 1. **`app`** — the shipped hot path: Gatekeeper + Engine driven at the real
//!    hop cadence in manual mode (`target = key`), reading the n = 1 `f_inst`.
//! 2. **`truth`** — a high-resolution Hann-windowed, heavily zero-padded DFT
//!    magnitude peak around f_ET. Method-independent from the phase vocoder
//!    (magnitude, not phase differencing); the arbiter.
//! 3. **`yin`** — a textbook YIN / autocorrelation estimate, the family the
//!    field's phone and web tuners use.
//!
//! If `app` disagrees with both `truth` and `yin`, the bias is ours. If
//! `app` and `yin` agree but both differ from `truth`, the autocorrelation
//! family is the biased reference.
//!
//! `strobe` reads a displayed number against this reference; `gates` reads a
//! detection threshold against it.

use realfft::RealFftPlanner;
use rustfft::num_complex::Complex;

use tuner_core::algorithms::spectral::{
    self, fft, goertzel_windowed, magnitude_spectrum, neyman_pearson_k,
};
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE, WINDOW_SIZE};
use tuner_core::engine::Engine;
use tuner_core::gatekeeper::{Gatekeeper, SignalState};
use tuner_core::models::{KeyProfile, NOTES, get_expected_beta};
use tuner_core::pipeline::ProcessingFrame;
use tuner_core::strobe::{MAX_STROBE_REFS, Strobe, StrobeRefUpdate};

pub(crate) const SAMPLE_RATE: u32 = 44_100;

pub(crate) const TAU: f32 = 2.0 * std::f32::consts::PI;

/// Sliding window for the band-slope readout: ≈ 0.5 s at the 43 Hz hop.
pub(crate) const BAND_WIN_HOPS: usize = 21;

/// Builds the 88 prior-B templates the live pipeline seeds when nothing is
/// measured (the guitar/ET case — no captured B for the key).
pub(crate) fn prior_profiles() -> [KeyProfile; 88] {
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
pub(crate) fn dtft_truth(
    signal: &[f32],
    center_hz: f32,
    planner: &mut RealFftPlanner<f32>,
) -> Option<f32> {
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

/// YIN (de Cheveigné & Kawahara 2002) over the freshest `win` samples:
/// difference function → cumulative-mean normalization → absolute threshold
/// 0.1 → parabolic interpolation. Represents the autocorrelation family.
pub(crate) fn yin(signal: &[f32], f_min: f32, f_max: f32) -> Option<f32> {
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

pub(crate) struct AppResult {
    pub(crate) f0: f32,
    pub(crate) locked_key: Option<u8>,
    pub(crate) gated_frac: f32,
    /// Hop-to-hop std of the instantaneous cents readout over the settled
    /// tail — the "strobe jitter" the user sees on the number (the band, being
    /// integrated, does not carry it).
    pub(crate) cents_jitter: f32,
}

/// **App.** Drives the real Gatekeeper + Engine at the live hop cadence
/// (`AudioPipeline::process_cola_hop`'s step order) in manual mode. Returns the
/// settled-tail mean of
/// the n = 1 `f_inst` and the engine's own `cents_deviation`.
pub(crate) fn run_engine(
    signal: &[f32],
    key: u8,
    noise_floor: f32,
    planner: &mut RealFftPlanner<f32>,
) -> Option<AppResult> {
    let fft_treble = planner.plan_fft_forward(WINDOW_SIZE);
    let fft_bass = planner.plan_fft_forward(BASS_WINDOW_SIZE);
    let profiles = prior_profiles();

    let mut frame = ProcessingFrame::new();
    let mut gate = Gatekeeper::new();
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

pub(crate) fn cents(f: f32, f_ref: f32) -> f32 {
    1200.0 * (f / f_ref).log2()
}

/// Additive guitar-ish tone: partial n at `n·f0·√(1+B·n²)`, per-partial decay
/// `exp(−t·n^0.6/tau0)` (higher partials die faster — the string physics).
pub(crate) fn synth_tone(f0: f32, b: f32, amps: &[f32], len: usize, tau0: f32) -> Vec<f32> {
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

/// Gate-aware unwrap → longest contiguous ungated run → sliding
/// least-squares slope → beat Hz → cents. The post-processing half of the
/// band-slope readout, shared by the single-reference and full-set drivers.
///
/// Gate-awareness matters: a gated hop holds the bank's angle, so counting
/// it would contribute zero drift and drag the fit toward 0 ¢ — a decayed note
/// would read "in tune". Gated hops therefore break the run, and the fit takes
/// the longest contiguous ungated stretch, which for a fast-decaying treble
/// note is its early, still-ringing life.
pub(crate) fn slope_from_angles(
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
/// The reference set is installed in one `StrobeRefUpdate`, exactly as the
/// live app does: the bank's long-window rule keys off `refs[0]`, so retargeting
/// with a lone higher-partial reference would select the short window and
/// diverge from the shipped path.
pub(crate) fn strobe_angles(
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

/// The band-slope readout at a single reference (`f_ET`, the guitar and ET
/// case): accumulate the beat phase through the `Strobe`, then
/// take the least-squares slope of the unwrapped angle over a sliding
/// `win_hops` window. Returns `(mean_cents, jitter_std, run_hops)`.
pub(crate) fn band_slope_cents_win(
    signal: &[f32],
    f_et: f32,
    win_hops: usize,
) -> Option<(f32, f32, usize)> {
    let mut refs = [0.0f32; MAX_STROBE_REFS];
    refs[0] = f_et;
    let angles = strobe_angles(signal, &refs, 1);
    slope_from_angles(&angles[0], f_et, win_hops)
}

/// [`band_slope_cents_win`] at the shipped window length.
pub(crate) fn band_slope_cents(signal: &[f32], f_et: f32) -> Option<(f32, f32)> {
    band_slope_cents_win(signal, f_et, BAND_WIN_HOPS).map(|(m, j, _)| (m, j))
}

/// The fixed-reference readable range (Hz): the largest |f_live − f_ref| whose
/// per-hop phase advance stays under ½ cycle, hence unwraps correctly. Beyond
/// it the band-slope aliases. Not a Goertzel limit — a hop/unwrap one.
pub(crate) const ALIAS_HZ: f32 = 0.5 * SAMPLE_RATE as f32 / HOP_SIZE as f32;

/// Symmetric Hann coefficients, the window [`goertzel_windowed`]'s callers use,
/// at a length chosen at runtime.
pub(crate) fn hann_vec(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 * (1.0 - (TAU * i as f32 / (n as f32 - 1.0)).cos()))
        .collect()
}

/// `Engine`'s and `Strobe`'s long-window rule: the 1024-sample Hann
/// main-lobe half-width is `2·fs/1024`; take the long window whenever the
/// partial spacing (≈ f₀, proxied by the f₁ seed) falls inside it.
pub(crate) fn register_window(seed_hz: f32) -> usize {
    if seed_hz * 1024.0 < 2.0 * SAMPLE_RATE as f32 {
        4096
    } else {
        1024
    }
}

/// Methods 1–2 — the adaptive phase-vocoder tracker (`Engine`'s tracking
/// state, n = 1 only) at an arbitrary analysis window. A faithful replica of
/// the engine's recurrence: Goertzel at the adaptive center over the freshest
/// `win` samples of the COLA buffer, wrapped phase difference against the
/// expected advance at the target, NP amplitude gate at `K(win)`, then the
/// 0.95/0.05 EMA re-centering. `--readout` prints the shipped engine's own
/// reading alongside as a check on the replica.
///
/// Returns one entry per hop: `None` where the method yields nothing (gated or
/// non-physical) — that is the availability signal.
pub(crate) fn tracker_series(
    signal: &[f32],
    seed_hz: f32,
    win: usize,
    noise_floor: f32,
) -> Vec<Option<f32>> {
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

/// Search half-width for the bounded spectral read, in Hz, as the shipped
/// `peaks::coarse_read` computes it: a cents span, floored at `min_bins`, then
/// capped at half the partial spacing. The cap is what MAT's 4-bin floor cannot
/// be copied without: in the Worker's 2¹⁶ FFT that floor is 2.7 Hz, but at 2048
/// it is an 86 Hz half-width, and uncapped the read at E2/A2/A0 returns the 2nd
/// partial (+1200 ¢). `spacing_hz` (≈ f₀) is not `center_hz`: they coincide only
/// for n = 1.
pub(crate) fn search_halfwidth_hz(
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
pub(crate) const P_FA: f32 = 0.001;

/// Ordered-statistic CFAR configuration (Rohling 1983).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct CfarCfg {
    /// Order statistic as a fraction of the reference count (0.5 = median).
    pub(crate) quantile: f32,
    /// Cells excluded either side of the peak — its own main lobe. The shipped
    /// read uses 0 (Rohling §V: unnecessary for an OS detector, and measured
    /// inert — audit 13). Retained here to sweep it and for the in-band control.
    pub(crate) guard_bins: usize,
    /// Take reference cells from outside the search band rather than inside.
    pub(crate) flanking: bool,
    /// Floor on the reference half-width, in Hz, since a floor in bins changes
    /// physical width with the FFT size. In the deep bass a `1.5 × spacing` flank
    /// samples only the strong low partials; 172 Hz (32 bins at 8192) reaches the
    /// weak upper ones the low quantile lands on (`--refset`, audit 13).
    pub(crate) flank_min_hz: f32,
    /// Use Rohling's exact finite-N scaling factor instead of the asymptotic
    /// quantile one.
    pub(crate) finite_n: bool,
}

/// Which detection threshold the bounded spectral read applies.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Gate {
    /// The ambient gate, Kay 1998 Neyman–Pearson against the silence threshold
    /// (`Engine::process`'s peak and tracker gates, `Strobe::process`). Its H₀ is
    /// a quiet room, the wrong null during a sustain (report 0015). The control.
    Ambient,
    /// Ordered-statistic CFAR against a local noise estimate taken from
    /// reference cells around the peak (Rohling 1983).
    Cfar(CfarCfg),
}

impl Gate {
    pub(crate) fn label(self) -> String {
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

/// Asymptotic threshold multiplier on an ordered-statistic noise estimate,
/// for Rayleigh magnitude bins — ours, derived.
///
/// Magnitude bins of locally-flat complex Gaussian noise are Rayleigh, so
/// `P(X > T) = exp(−T²/2σ²)` and the `q`-quantile is `σ·√(−2·ln(1−q))`.
/// Estimating σ from that quantile and solving for the P_fa threshold gives
/// `T = x_q · √( ln(P_fa) / ln(1−q) )` — 3.157 at the median, 4.900 at the
/// 25th percentile, both for P_fa = 0.001. Exact only as the reference count
/// → ∞; [`cfar_multiplier_finite`] is the finite-N form, and the two agree in
/// the limit (core's `coarse_cfar_multiplier_pinned` pins it).
pub(crate) fn cfar_multiplier(quantile: f32) -> f32 {
    (P_FA.ln() / (1.0 - quantile).ln()).sqrt()
}

/// Exact finite-N scaling factor — a faithful port of Rohling (1983).
///
/// His Eq. 14 gives the false-alarm probability of an OS-CFAR detector with
/// `N` reference cells selecting rank `k`, for an exponentially distributed
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
/// Our cells are Rayleigh magnitudes, not exponential powers, so his Table II
/// does not apply directly; Eq. 17 converts it for a receiver taking the
/// absolute value, whose cells "obey a Rayleigh distribution", as
/// `T_lin = √T_q`.
///
/// # Reference
/// Rohling, H. (1983). "Radar CFAR Thresholding in Clutter and Multiple Target
/// Situations." IEEE Trans. Aerospace and Electronic Systems, AES-19(4),
/// pp. 608–621. DOI: 10.1109/TAES.1983.309350. (Eqs. 9–10, 12, 14, 17.)
/// Lineage: Finn, H. M. & Johnson, R. S. (1968), "Adaptive Detection Mode with
/// Threshold Control as a Function of Spatially Sampled Clutter-Level
/// Estimates", RCA Review 29(3), pp. 414–464 — the cell-averaging predecessor.
pub(crate) fn cfar_multiplier_finite(n_ref: usize, k: usize, p_fa: f32) -> f32 {
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
pub(crate) fn ambient_threshold(noise_floor: f32, fft_size: usize) -> f32 {
    let p_bin = noise_floor * noise_floor * 0.375 * fft_size as f32;
    (-p_bin * P_FA.ln()).sqrt()
}

/// Outcome of one bounded spectral read, distinguishing why a hop produced
/// nothing — a gate that rejects and a gate that cannot be calibrated are very
/// different failures, and in the deep bass both occur.
pub(crate) enum Read {
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
pub(crate) struct ReadOut {
    pub(crate) read: Read,
    /// Reference cells used to set the threshold (0 for [`Gate::Ambient`]).
    pub(crate) n_ref: usize,
    /// Peak magnitude ÷ threshold — the CFAR margin. `> 1` iff admitted, and
    /// the quantity a strongest-partial policy ranks partials by (comparable
    /// across partials because each is normalized by its own local noise).
    pub(crate) margin: f32,
}

/// Outer bounds of the two flanking reference bands, before guard exclusion.
///
/// Shared by `spectral_read` and `ref_anatomy` so the anatomy report cannot
/// drift from the gate it describes. Both terms are compared in Hz and converted
/// to bins once.
pub(crate) fn ref_window(
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

/// Method 3 — bounded spectral peak + `jacobsen`. The candidate coarse
/// read: argmax of the already-computed magnitude spectrum within
/// [`search_halfwidth_hz`] of `center_hz`, refined sub-bin by the audited
/// Candan estimator, admitted by `gate`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spectral_read(
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
                // Rohling's P_fa is for one cell under test, but this detector
                // takes the argmax over the whole search band, so it gets M
                // independent chances to false-alarm and the realized rate is
                // ≈ M·P_fa. Measured directly: collapsing the band to a single
                // bin brought the realized AWGN rate to 0.0012 against a
                // nominal 0.001 (exactly right), while the full band gave
                // 0.0386, a 32× search loss (39× nominal). The correction is the standard
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
pub(crate) fn spectral_series(
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
pub(crate) struct Score {
    pub(crate) avail: f32,
    /// Availability over the note's first third — the treble's usable life.
    pub(crate) avail_early: f32,
    pub(crate) median_cents: f32,
    pub(crate) jitter: f32,
}

pub(crate) fn score(series: &[Option<f32>], f_ref: f32) -> Score {
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

/// Highest partial index examined by `bass_partials`. The strobe bank holds
/// 12 references; the interesting bass energy is well inside the first six.
pub(crate) const MAX_BASS_PARTIAL: usize = 6;

/// Strobe reference frequencies for a key, in the shipped convention
/// ([`tuner_core::models::TuningCurve::strobe_partials`]):
/// `f₀* = f₁*/√(1+B)`, then `fₙ* = n·f₀*·√(1+B n²)`.
///
/// The obvious form `n·f_ET·√(1+B n²)` treats f_ET as the flexible-string f₀,
/// but the curve —
/// and every target in this app — is defined on the audible first partial
/// f₁. The two differ by √(1+B): 0.09 ¢ at A0, but 17 ¢ at A7, which would
/// silently poison any treble column. And under the correct form the n = 1
/// reference is identically `f₁*` with B cancelling — the B-immunity the
/// register table's treble entry relies on.
///
/// Offline there is no curve, so `f1` is the key's ET frequency: the ET-mode
/// reference the GUI uses for a guitar, and the cold-start target for a piano.
pub(crate) fn strobe_refs(
    f1: f32,
    b: f32,
    max_n: usize,
    out: &mut [f32; MAX_STROBE_REFS],
) -> usize {
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
/// n = 1 is excluded on two grounds. Statistically it sits at `x = n² = 1`, the
/// extreme of the regressor's range, so it carries maximal leverage on the
/// intercept, and B is `slope/intercept`, so a bad fundamental corrupts B
/// directly. Physically it is the least informative point about B, its `B·n²`
/// term the smallest in the series. In the deep bass the n = 1 "truth" is junk,
/// and including it returns negative B on A0 and C#1.
pub(crate) fn fit_f0_b(freqs: &[(usize, f32)]) -> Option<(f32, f32)> {
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

/// The shipped coarse-read gate, used by every measurement below so their
/// results are mutually comparable: 25th percentile, no guard cells, flanking
/// references floored at `flank_min_hz`, exact finite-N multiplier. Without the
/// floor the deep-bass flank reaches only the strong low partials and the gate
/// rejects the true peak, so a bass row measured without it measures a broken gate.
pub(crate) fn shipping_gate_hz(flank_min_hz: f32) -> Gate {
    Gate::Cfar(CfarCfg {
        quantile: 0.25,
        guard_bins: 0,
        flanking: true,
        flank_min_hz,
        finite_n: true,
    })
}

/// [`shipping_gate_hz`] at the default flank floor.
pub(crate) fn shipping_gate() -> Gate {
    shipping_gate_hz(FLANK_MIN_HZ)
}

/// Reference-flank floor in Hz (see [`CfarCfg::flank_min_hz`]).
pub(crate) const FLANK_MIN_HZ: f32 = 172.0;

/// Per-hop reading and CFAR margin for one reference partial.
pub(crate) type HopRead = (Option<f32>, f32);

/// Per-hop reads of every reference partial from a single FFT, the structure
/// the pipeline would use (one spectrum, several bounded searches). Returns
/// `[hop][partial] = (reading, CFAR margin)`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn multi_partial_series(
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

/// The gate variants compared by `gate_ab`: the ambient-σ control, the in-band
/// median control with ±2 guard bins, and the guard-free flank-floor sweep at
/// the median and the 25th percentile.
pub(crate) fn gate_variants() -> Vec<Gate> {
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
