//! # Strobe — fixed-reference phase comparator (strobe Path A)
//!
//! The DSP half of the absolute-partial strobe (strobe design note §5.2,
//! R1/R2/R3): a bank of Goertzel evaluations pinned to the *curve target*
//! frequencies of the key being tuned — unlike the engine's tracker, whose
//! adaptive seeds follow the physical string. Each hop the bank reads the
//! phase of the live signal at every reference and accumulates the wrapped
//! hop-to-hop drift into a per-reference **beat phase** (cycles, mod 1):
//! exactly `f_live − f_ref` cycles per second, the rotation a strobe
//! displays. Hop-to-hop phase differences at non-integer bins are exact
//! (the audit-08 property of [`spectral::goertzel`]'s finalization — the D1
//! basis), so the accumulated angle is a true beat count, not an estimate.
//!
//! Angle-as-state (R2): accumulation happens here, on the DSP thread, so the
//! lossy `FrameOutput` triple buffer cannot corrupt it — a dropped frame
//! skips one visual update instead of losing beat cycles.
//!
//! The bank ships that rotation twice: the accumulated angle, and its **rate** —
//! a least-squares fit of the unwrapped angle against hop index over
//! [`BAND_SLOPE_WINDOW_SECS`], in Hz. Rotation is exactly `f_live − f_ref`
//! (design §3), so that slope is the detuning.
//!
//! Deep-bass references evaluate a 4096-sample window (R3): partial spacing
//! ≈ f₀ falls inside the 1024-sample Hann main lobe below ≈ 86 Hz, and no
//! choice of displayed partial changes the spacing. Window length ≠ hop —
//! the bank still updates every hop.
//!
//! The bank runs in `process_cola_hop` whenever references are set,
//! independent of the engine's note lock — an early strike, or a note the
//! discovery mistracked, still spins the band. Gatekeeper `Silence` is the one
//! state that stops it: there is no note by definition, so any phase advance
//! would be room noise (see [`Strobe::process`]).

use crate::algorithms::{peaks, spectral};
use crate::audio::{BASS_WINDOW_SIZE, HOP_RATE_HZ, HOP_SIZE, WINDOW_SIZE};
use crate::pipeline::ProcessingFrame;

/// Capacity of the reference bank — mirrors the tracker's partial ceiling
/// and `FrameOutput`'s strobe arrays.
pub const MAX_STROBE_REFS: usize = 12;

/// Sliding window the band-slope fit spans (strobe design §5.5 / R12).
///
/// The trade is **linear cost against an inelastic benefit** (ADR 0011 §11): the
/// fit lags a turning peg by exactly `T/2` — an OLS slope estimates the rate at
/// the window's *midpoint* — while the jitter it buys falls far slower than the
/// `T⁻³` law, because the bank's analysis windows overlap 75–87 % and extra hops
/// are mostly the same audio. Measured, tripling `T` buys 1.8× on piano and
/// nothing measurable on guitar.
///
/// 0.6 s is the responsive-enough end of that trade in use, and 1 ¢ of steadiness
/// is inside the range a professional tuner works in. To trade the 300 ms of lag
/// back, shorten this; two lengths are derived rather than arbitrary, and each
/// *removes* a constant by absorbing [`BAND_SLOPE_MIN_SPAN_SECS`] (which can never
/// exceed it): `BASS_WINDOW_SIZE / SAMPLE_RATE` ≈ 0.186 s matches the coarse
/// read's own group delay, so the two readouts lag equally and the source switch
/// stops stepping the number; 0.25 s simply collapses the two constants. Escaping
/// the trade rather than moving along it needs a two-state (g–h / Kalman)
/// estimator on (phase, rate), which weights old data instead of discarding it.
pub const BAND_SLOPE_WINDOW_SECS: f32 = 0.6;

/// Points the fit retains — one per hop across [`BAND_SLOPE_WINDOW_SECS`], plus
/// the current hop.
pub const BAND_SLOPE_POINTS: usize = (BAND_SLOPE_WINDOW_SECS * HOP_RATE_HZ) as usize + 1;

/// Minimum span before a rate is published; the coarse read covers the fill-in.
///
/// Floored by the analysis window itself: the bank integrates over
/// `BASS_WINDOW_SIZE` samples ≈ 0.186 s, so a fit spanning less than that is
/// drawing a line through repeats of *one* window's audio and has no independent
/// information about the rate. This value clears that floor by 1.34×
/// (`test_band_slope_span_clears_the_analysis_window` pins it).
pub const BAND_SLOPE_MIN_SPAN_SECS: f32 = 0.25;

/// Points needed to span [`BAND_SLOPE_MIN_SPAN_SECS`]: `n` points bridge `n − 1`
/// hop intervals, and the truncating cast has to round *up* to reach the span —
/// hence `+ 2`, which `test_band_slope_point_counts_bracket_their_durations`
/// pins in both directions.
pub const BAND_SLOPE_MIN_POINTS: usize = (BAND_SLOPE_MIN_SPAN_SECS * HOP_RATE_HZ) as usize + 2;

/// One key's strobe reference set, in transit UI → DSP (crossing #4 charter:
/// grouped parameters that must apply atomically on a frame boundary).
/// Heap-free and `Copy` — legal across the real-time boundary.
///
/// Carries both the phase-comparison references and the coarse-read target
/// for the [`Strobe`]: one message atomically updates both readouts.
#[derive(Debug, Clone, Copy)]
pub struct StrobeRefUpdate {
    /// Number of valid entries in `refs`; 0 clears the bank (no key selected).
    pub count: usize,
    /// Per-partial reference frequencies f_n* in Hz, index = partial n − 1
    /// (the `TuningCurve::strobe_partials` order).
    pub refs: [f32; MAX_STROBE_REFS],
    /// Partial the coarse read centres on, as `n` (1-based, so `refs[n − 1]`).
    /// The frontend's policy call — `curves::coarse_read_partial` is the
    /// derived rule — and independent of the *displayed* partial, which the
    /// register table picks for the band. `0`, or any index past `count`,
    /// disables the coarse read.
    pub coarse_index: u8,
    /// The key's partial spacing f₀\* in Hz — the neighbour cap and the
    /// CFAR reference width both scale with it, and neither may use the
    /// centre frequency in its place (`peaks::coarse_read`).
    pub spacing_hz: f32,
}

/// Per-hop strobe telemetry, returned by value (the component convention:
/// the pipeline copies it into `FrameOutput`).
#[derive(Debug, Clone, Copy)]
pub struct StrobeResult {
    /// Number of live references.
    pub count: usize,
    /// Accumulated beat phase per reference, cycles in [0, 1).
    pub angle: [f32; MAX_STROBE_REFS],
    /// D3 amplitude gate: `true` = this reference was below the noise floor
    /// this hop (its angle was held, not advanced).
    pub gated: [bool; MAX_STROBE_REFS],
    /// Beat rate per reference, `f_live − f_ref` in Hz — the slope of the
    /// accumulated angle over [`BAND_SLOPE_WINDOW_SECS`]. `None` until the fit
    /// spans [`BAND_SLOPE_MIN_SPAN_SECS`]; held at its last value while the
    /// reference is gated, and dropped again when a re-strike restarts the fit.
    ///
    /// Hz, not cents: the reference is `refs[i]` and the frontend owns what it
    /// displays the offset against (crossing #2's rule for `coarse_hz`).
    pub beat_hz: [Option<f32>; MAX_STROBE_REFS],
    /// CFAR-gated coarse spectral readout (Hz), or `None`.
    pub coarse_hz: Option<f32>,
}

impl Default for StrobeResult {
    fn default() -> Self {
        Self {
            count: 0,
            angle: [0.0; MAX_STROBE_REFS],
            gated: [true; MAX_STROBE_REFS],
            beat_hz: [None; MAX_STROBE_REFS],
            coarse_hz: None,
        }
    }
}

/// Sliding-window OLS fit of one reference's unwrapped beat phase (cycles),
/// indexed by hop. The hop cadence is regular, so the abscissae are a fixed
/// integer sequence: the normal equations reduce to running sums, and the
/// window's span follows from its point count.
struct BandSlope {
    /// Retained unwrapped phase, oldest at `head`.
    y: [f32; BAND_SLOPE_POINTS],
    head: usize,
    len: usize,
    /// `Σ y_j` and `Σ j·y_j` over the retained points, `j = 0` at the oldest.
    sum_y: f64,
    sum_jy: f64,
    /// Running unwrapped phase (cycles). Rebased at every restart so it cannot
    /// grow past f32's useful precision during a long sustain.
    unwrapped: f32,
    /// Last published slope, cycles per hop.
    rate: Option<f32>,
    /// A gated hop breaks the run; the next live hop restarts the fit.
    restart: bool,
}

impl BandSlope {
    const fn new() -> Self {
        Self {
            y: [0.0; BAND_SLOPE_POINTS],
            head: 0,
            len: 0,
            sum_y: 0.0,
            sum_jy: 0.0,
            unwrapped: 0.0,
            rate: None,
            restart: true,
        }
    }

    /// Drops the fit and its published rate, and breaks the run so the next
    /// live hop starts a fresh baseline. Points past `len` are never read, so
    /// the ring itself needs no clearing.
    fn reset(&mut self) {
        self.head = 0;
        self.len = 0;
        self.sum_y = 0.0;
        self.sum_jy = 0.0;
        self.unwrapped = 0.0;
        self.rate = None;
        self.restart = true;
    }

    /// Gated hop: hold the last rate — a frozen band's angle does not advance,
    /// so integrating the gap would read the detuning toward zero — and break
    /// the run so a re-strike resumes without a phantom jump across it.
    fn hold(&mut self) {
        self.restart = true;
    }

    /// Live hop: `cycles` is the wrapped beat-phase advance since the last hop.
    fn push(&mut self, cycles: f32) {
        if self.restart {
            self.reset();
            self.restart = false;
        } else {
            self.unwrapped += cycles;
        }

        if self.len == BAND_SLOPE_POINTS {
            // Evicting the oldest point shifts every survivor one place down,
            // so `Σ j·y` loses the whole of `Σ y` less the evicted term.
            let old = self.y[self.head] as f64;
            self.sum_jy -= self.sum_y - old;
            self.sum_y -= old;
            self.head = (self.head + 1) % BAND_SLOPE_POINTS;
            self.len -= 1;
        }
        let j = self.len as f64;
        self.y[(self.head + self.len) % BAND_SLOPE_POINTS] = self.unwrapped;
        self.sum_y += self.unwrapped as f64;
        self.sum_jy += j * self.unwrapped as f64;
        self.len += 1;

        if self.len >= BAND_SLOPE_MIN_POINTS {
            self.rate = Some(self.slope());
        }
    }

    /// OLS slope in cycles per hop. The abscissae are `0..n`, so
    /// `n·Σj² − (Σj)²` reduces to `n²(n²−1)/12` — nonzero for every `n ≥ 2`,
    /// which [`BAND_SLOPE_MIN_POINTS`] guarantees.
    fn slope(&self) -> f32 {
        debug_assert!(self.len >= 2, "a slope needs two points");
        let n = self.len as f64;
        let sum_j = n * (n - 1.0) / 2.0;
        ((n * self.sum_jy - sum_j * self.sum_y) / (n * n * (n * n - 1.0) / 12.0)) as f32
    }
}

/// Fixed-reference Goertzel bank with DSP-side angle accumulation.
pub struct Strobe {
    sample_rate: u32,
    count: usize,
    refs: [f32; MAX_STROBE_REFS],
    /// R3: `true` while the reference spacing (≈ f₁*) sits inside the
    /// 1024-sample Hann main lobe, selecting the 4096-sample window.
    long_window: bool,
    prev_phase: [f32; MAX_STROBE_REFS],
    angle: [f32; MAX_STROBE_REFS],
    /// Sliding-window rate fit per reference.
    band: [BandSlope; MAX_STROBE_REFS],
    /// First hop after a retarget: seed `prev_phase` only, publish no drift.
    warmup: bool,
    /// Coarse-read parameters from the last retarget.
    coarse_index: u8,
    spacing_hz: f32,
    /// CFAR reference-cell scratch buffer, reused across hops.
    /// Sized to BASS_WINDOW_SIZE / 2 (the most cells the flanks can yield).
    coarse_scratch: Box<[f32]>,
}

impl Strobe {
    /// Creates an empty bank (no references — [`process`](Self::process) is
    /// a no-op until a [`StrobeRefUpdate`] arrives).
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            count: 0,
            refs: [0.0; MAX_STROBE_REFS],
            long_window: false,
            prev_phase: [0.0; MAX_STROBE_REFS],
            angle: [0.0; MAX_STROBE_REFS],
            band: [const { BandSlope::new() }; MAX_STROBE_REFS],
            warmup: true,
            coarse_index: 0,
            spacing_hz: 0.0,
            coarse_scratch: vec![0.0; BASS_WINDOW_SIZE / 2].into_boxed_slice(),
        }
    }

    /// (centre_hz, spacing_hz) for the coarse read, or `None` when no
    /// readable coarse partial is selected.
    fn coarse_target(&self) -> Option<(f32, f32)> {
        let n = self.coarse_index as usize;
        if n == 0 || n > self.count.min(MAX_STROBE_REFS) {
            return None;
        }
        let center = self.refs[n - 1];
        (center > 0.0 && self.spacing_hz > 0.0).then_some((center, self.spacing_hz))
    }

    /// Replaces the reference set (key change / re-lock / clear). Angles
    /// reset — an accumulated beat count against the old targets is
    /// meaningless against the new ones.
    pub fn retarget(&mut self, update: StrobeRefUpdate) {
        self.count = update.count.min(MAX_STROBE_REFS);
        self.refs = update.refs;
        self.coarse_index = update.coarse_index;
        self.spacing_hz = update.spacing_hz;
        // Long window iff the 1024-sample main-lobe half-width (2·fs/1024)
        // exceeds the partial spacing, proxied by f₁* = refs[0] (spacing is
        // ≈ f₀ ≤ f₁* for every partial pair of the key).
        self.long_window =
            self.count > 0 && update.refs[0] * 1024.0 < 2.0 * self.sample_rate as f32;
        self.prev_phase = [0.0; MAX_STROBE_REFS];
        self.angle = [0.0; MAX_STROBE_REFS];
        for band in &mut self.band {
            band.reset();
        }
        self.warmup = true;
    }

    /// One hop: evaluate every reference over the freshest window of
    /// `audio` (the pipeline's full 8192-sample buffer) and advance the
    /// accumulated angles. `noise_floor` is the calibrated silence
    /// threshold from the config atomics.
    ///
    /// `is_silence` gates every reference for the hop. It is not redundant with
    /// the per-reference amplitude test: that test admits a *single bin* above
    /// `noise_floor · K(n)`, and `K ∝ 1/√n` puts it at 10 % of the silence
    /// threshold at the 4096-sample window. Low-frequency room rumble clears
    /// that comfortably while the broadband RMS is still below the silence
    /// floor, so without this the band accumulates rumble phase and rotates
    /// with no note playing. Angles are held, not reset (R2).
    pub fn process(
        &mut self,
        frame: &ProcessingFrame,
        noise_floor: f32,
        is_silence: bool,
    ) -> StrobeResult {
        let mut out = StrobeResult {
            count: self.count,
            ..Default::default()
        };
        if self.count == 0 {
            return out;
        }

        let (eval, k): (spectral::GoertzelFn, f32) = if self.long_window {
            (spectral::goertzel_bass, spectral::neyman_pearson_k(4096))
        } else {
            (spectral::goertzel, spectral::neyman_pearson_k(1024))
        };
        let t_amp = noise_floor * k;
        let t_hop = HOP_SIZE as f32 / self.sample_rate as f32;
        let audio = &frame.audio_buffer[..BASS_WINDOW_SIZE];

        for i in 0..self.count {
            let f_ref = self.refs[i];
            let (amplitude, phase) = eval(audio, self.sample_rate, f_ref);

            if self.warmup {
                self.prev_phase[i] = phase;
                out.angle[i] = self.angle[i];
                self.band[i].hold();
                continue;
            }

            // Wrapped drift against the expected advance at exactly f_ref —
            // the engine's phase-vocoder step with a *fixed* target.
            let expected = 2.0 * core::f32::consts::PI * f_ref * t_hop;
            let raw = phase - self.prev_phase[i] - expected;
            let delta = (raw + core::f32::consts::PI).rem_euclid(2.0 * core::f32::consts::PI)
                - core::f32::consts::PI;
            // prev_phase advances even when gated, so a re-strike resumes
            // without a phantom phase jump.
            self.prev_phase[i] = phase;

            let gated = is_silence || amplitude < t_amp || !delta.is_finite();
            if gated {
                self.band[i].hold();
            } else {
                let cycles = delta / (2.0 * core::f32::consts::PI);
                self.angle[i] = (self.angle[i] + cycles).rem_euclid(1.0);
                self.band[i].push(cycles);
            }
            out.gated[i] = gated;
            out.angle[i] = self.angle[i];
            out.beat_hz[i] = self.band[i]
                .rate
                .map(|cycles_per_hop| cycles_per_hop / t_hop);
        }
        self.warmup = false;

        // ── Coarse spectral readout ──
        // Bounded CFAR-gated search at the nominated reference partial.
        // Try the bass (8192) spectrum first; fall back to treble (2048).
        let mag_count_bass = BASS_WINDOW_SIZE / 2;
        let mag_count = WINDOW_SIZE / 2;
        out.coarse_hz =
            self.coarse_target()
                .filter(|_| !is_silence)
                .and_then(|(center_hz, spacing_hz)| {
                    peaks::coarse_read(
                        &frame.bass_magnitude_buffer[..mag_count_bass],
                        &frame.bass_frequency_buffer[..],
                        BASS_WINDOW_SIZE,
                        self.sample_rate,
                        center_hz,
                        spacing_hz,
                        &mut self.coarse_scratch,
                    )
                    .or_else(|| {
                        peaks::coarse_read(
                            &frame.treble_magnitude_buffer[..mag_count],
                            &frame.frequency_buffer[..],
                            WINDOW_SIZE,
                            self.sample_rate,
                            center_hz,
                            spacing_hz,
                            &mut self.coarse_scratch,
                        )
                    })
                });

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::BASS_WINDOW_SIZE;

    /// The generalized K(n) must reproduce the engine's pinned constant at
    /// its window length — same gate, no second constant (D3).
    #[test]
    fn test_neyman_pearson_k_matches_engine() {
        assert!((spectral::neyman_pearson_k(1024) - 0.201184).abs() < 1e-4);
        // K ∝ 1/√n: the 4096 window's threshold is half the 1024 one.
        let ratio = spectral::neyman_pearson_k(4096) / spectral::neyman_pearson_k(1024);
        assert!((ratio - 0.5).abs() < 1e-6);
    }

    /// Rumble below the silence threshold must not turn the band. A single bin
    /// only has to clear `noise_floor · K(4096)` — 10 % of the silence floor —
    /// so low-frequency room noise passes the amplitude test while the broadband
    /// RMS says silence. `is_silence` is what stops the angle advancing.
    #[test]
    fn test_silence_state_overrides_the_amplitude_test() {
        let fs = 44_100u32;
        let f_ref = 30.87; // B0 — deep bass, where rumble lives
        let mut bank = Strobe::new(fs);
        let mut refs = [0.0; MAX_STROBE_REFS];
        refs[0] = f_ref;
        bank.retarget(StrobeRefUpdate {
            count: 1,
            refs,
            coarse_index: 1,
            spacing_hz: f_ref,
        });

        // Rumble 3 Hz off the reference, amplitude just over the band's gate
        // (K(4096) ≈ 0.1006) but far under the 0.005 silence threshold.
        let noise_floor = 0.005f32;
        let amp = noise_floor * spectral::neyman_pearson_k(4096) * 3.0;
        assert!(amp < noise_floor, "rumble must sit below the silence floor");
        let total = BASS_WINDOW_SIZE + 12 * HOP_SIZE;
        let signal: Vec<f32> = (0..total)
            .map(|i| {
                amp * (2.0 * std::f32::consts::PI * (f_ref + 3.0) * i as f32 / fs as f32).sin()
            })
            .collect();

        let mut ungated_moves = 0;
        let mut last = 0.0f32;
        let mut frame_buf = ProcessingFrame::new();
        for h in 0..12 {
            let w = &signal[h * HOP_SIZE..h * HOP_SIZE + BASS_WINDOW_SIZE];
            frame_buf.audio_buffer[..BASS_WINDOW_SIZE].copy_from_slice(w);
            // Not silence: the amplitude test admits the rumble and the band turns.
            let f = bank.process(&frame_buf, noise_floor, false);
            if h > 1 && !f.gated[0] && (f.angle[0] - last).abs() > 1e-6 {
                ungated_moves += 1;
            }
            last = f.angle[0];
        }
        assert!(
            ungated_moves > 0,
            "precondition: rumble at {amp:.6} clears the band's own gate"
        );

        // Same audio, Gatekeeper says Silence: gated, and the angle is held.
        let held = last;
        for h in 0..6 {
            let w = &signal[h * HOP_SIZE..h * HOP_SIZE + BASS_WINDOW_SIZE];
            frame_buf.audio_buffer[..BASS_WINDOW_SIZE].copy_from_slice(w);
            let f = bank.process(&frame_buf, noise_floor, true);
            assert!(f.gated[0], "silence must gate every reference");
            assert_eq!(f.angle[0], held, "gated angle must hold, not reset (R2)");
        }
    }

    /// The coarse partial resolves 1-based against `refs`, and every way of
    /// *not* selecting one yields `None` — the pipeline reads that as "skip the
    /// coarse read this hop" rather than reading `refs[0]` by accident.
    #[test]
    fn test_coarse_target_resolution() {
        let mut bank = Strobe::new(44_100);
        let mut refs = [0.0; MAX_STROBE_REFS];
        for (i, r) in refs.iter_mut().enumerate() {
            *r = 27.5 * (i + 1) as f32;
        }
        let base = StrobeRefUpdate {
            count: 6,
            refs,
            coarse_index: 4,
            spacing_hz: 27.5,
        };
        bank.retarget(base);
        assert_eq!(bank.coarse_target(), Some((110.0, 27.5)));

        bank.retarget(StrobeRefUpdate {
            coarse_index: 1,
            ..base
        });
        assert_eq!(bank.coarse_target(), Some((27.5, 27.5)));

        // Disabled, past `count`, cleared bank, and a degenerate spacing.
        bank.retarget(StrobeRefUpdate {
            coarse_index: 0,
            ..base
        });
        assert!(bank.coarse_target().is_none());

        bank.retarget(StrobeRefUpdate {
            coarse_index: 7,
            ..base
        });
        assert!(bank.coarse_target().is_none());

        bank.retarget(StrobeRefUpdate { count: 0, ..base });
        assert!(bank.coarse_target().is_none());

        bank.retarget(StrobeRefUpdate {
            spacing_hz: 0.0,
            ..base
        });
        assert!(bank.coarse_target().is_none());
    }

    /// The fit's minimum span must clear one analysis window, or it is drawing a
    /// line through repeats of the same audio (ADR 0011 §11).
    #[test]
    fn test_band_slope_span_clears_the_analysis_window() {
        let analysis_secs = BASS_WINDOW_SIZE as f32 / 44_100.0;
        assert!(
            BAND_SLOPE_MIN_SPAN_SECS >= analysis_secs,
            "min span {BAND_SLOPE_MIN_SPAN_SECS} s is under one analysis window ({analysis_secs} s)"
        );
    }

    /// Both point counts are truncating casts of a duration, so pin them from
    /// both sides: the retained window must cover its seconds, and the minimum
    /// must be the *first* count that spans its own.
    #[test]
    fn test_band_slope_point_counts_bracket_their_durations() {
        let span = |points: usize| (points - 1) as f32 / HOP_RATE_HZ;
        assert!(span(BAND_SLOPE_POINTS) <= BAND_SLOPE_WINDOW_SECS);
        assert!(span(BAND_SLOPE_POINTS + 1) > BAND_SLOPE_WINDOW_SECS);
        assert!(span(BAND_SLOPE_MIN_POINTS) >= BAND_SLOPE_MIN_SPAN_SECS);
        assert!(span(BAND_SLOPE_MIN_POINTS - 1) < BAND_SLOPE_MIN_SPAN_SECS);
    }

    /// The running sums must reproduce a textbook OLS over the retained points,
    /// evictions included — the fit's whole correctness argument.
    #[test]
    fn test_band_slope_matches_a_direct_fit() {
        let mut band = BandSlope::new();
        let mut pushed: Vec<f64> = Vec::new();
        let mut y = 0.0f32;
        for h in 0..BAND_SLOPE_POINTS * 3 {
            // Curved, so a mis-weighted eviction cannot cancel out.
            let cycles = 0.02 + 0.004 * (h as f32 * 0.3).sin();
            if h > 0 {
                y += cycles;
            }
            band.push(cycles);
            pushed.push(y as f64);

            let w = &pushed[pushed.len().saturating_sub(BAND_SLOPE_POINTS)..];
            assert_eq!(band.len, w.len(), "retained count at hop {h}");
            if w.len() < BAND_SLOPE_MIN_POINTS {
                assert!(band.rate.is_none(), "no rate before the minimum span");
                continue;
            }
            let n = w.len() as f64;
            let (sum_j, sum_y) = (n * (n - 1.0) / 2.0, w.iter().sum::<f64>());
            let sum_jy: f64 = w.iter().enumerate().map(|(j, v)| j as f64 * v).sum();
            let sum_jj: f64 = (0..w.len()).map(|j| (j * j) as f64).sum();
            let direct = (n * sum_jy - sum_j * sum_y) / (n * sum_jj - sum_j * sum_j);
            let got = band.rate.expect("rate past the minimum span") as f64;
            assert!(
                (got - direct).abs() < 1e-6,
                "hop {h}: incremental {got} vs direct {direct}"
            );
        }
    }

    /// A gated run holds the last rate and then drops it: the first live hop
    /// after the gap restarts the fit, so a re-strike shows nothing until the
    /// window refills rather than a stale rate.
    #[test]
    fn test_band_slope_holds_then_restarts_across_a_gate() {
        let mut band = BandSlope::new();
        for _ in 0..BAND_SLOPE_MIN_POINTS + 1 {
            band.push(0.02);
        }
        let held = band.rate.expect("filled");
        band.hold();
        assert_eq!(band.rate, Some(held), "a gated hop holds the last rate");
        band.push(0.02);
        assert!(band.rate.is_none(), "the re-strike restarts the fit");
        assert_eq!(band.len, 1);
    }

    /// Streams a sine `delta_hz` off the reference through the bank and returns
    /// (final angle, final beat rate, any hop gated after warmup).
    fn run_bank(f_ref: f32, delta_hz: f32, hops: usize) -> (f32, Option<f32>, bool) {
        let fs = 44_100u32;
        let f_sig = f_ref + delta_hz;
        let total = BASS_WINDOW_SIZE + hops * HOP_SIZE;
        let signal: Vec<f32> = (0..total)
            .map(|i| 0.1 * (2.0 * std::f32::consts::PI * f_sig * i as f32 / fs as f32).sin())
            .collect();

        let mut bank = Strobe::new(fs);
        let mut refs = [0.0; MAX_STROBE_REFS];
        refs[0] = f_ref;
        bank.retarget(StrobeRefUpdate {
            count: 1,
            refs,
            coarse_index: 1,
            spacing_hz: refs[0],
        });

        let mut angle = 0.0;
        let mut beat_hz = None;
        let mut any_gated = false;
        let mut frame_buf = ProcessingFrame::new();
        for h in 0..hops {
            let window = &signal[h * HOP_SIZE..h * HOP_SIZE + BASS_WINDOW_SIZE];
            frame_buf.audio_buffer[..BASS_WINDOW_SIZE].copy_from_slice(window);
            let frame = bank.process(&frame_buf, 0.005, false);
            angle = frame.angle[0];
            beat_hz = frame.beat_hz[0];
            if h > 0 && frame.gated[0] {
                any_gated = true;
            }
        }
        (angle, beat_hz, any_gated)
    }

    /// A 440 Hz reference hearing 441 Hz must accumulate ≈ Δf·t beat
    /// cycles — the display contract (rotation = the audible beat).
    #[test]
    fn test_beat_phase_accumulates_at_delta_f() {
        let hops = 20; // 19 integrating hops ≈ 0.4412 s after warmup
        let t = (hops - 1) as f32 * HOP_SIZE as f32 / 44_100.0;
        let (angle, _, any_gated) = run_bank(440.0, 1.0, hops);
        assert!(!any_gated, "clean sine above floor must not gate");
        assert!(
            (angle - t).abs() < 0.03,
            "expected ≈{t:.3} beat cycles, got {angle:.3}"
        );

        // Flat by 1 Hz ⇒ same magnitude, opposite direction (mod 1).
        let (angle_flat, _, _) = run_bank(440.0, -1.0, hops);
        assert!(
            (angle_flat - (1.0 - t)).abs() < 0.03,
            "expected ≈{:.3}, got {angle_flat:.3}",
            1.0 - t
        );
    }

    /// The published rate is the detuning itself, signed — the readout contract
    /// (design §3: rotation = f_live − f_ref).
    #[test]
    fn test_beat_rate_recovers_the_detuning() {
        for delta in [1.0f32, -1.0, 0.5] {
            let rate = run_bank(440.0, delta, BAND_SLOPE_MIN_POINTS + 2)
                .1
                .expect("past the minimum span");
            assert!(
                (rate - delta).abs() < 0.05,
                "expected ≈{delta} Hz, got {rate:.3}"
            );
        }
        // Withheld until the fit spans its minimum: warmup costs one hop.
        assert!(
            run_bank(440.0, 1.0, BAND_SLOPE_MIN_POINTS).1.is_none(),
            "a short run must publish no rate"
        );
    }

    /// In tune ⇒ stationary: zero offset accumulates ≈ nothing and reads ≈ 0 Hz.
    #[test]
    fn test_in_tune_is_stationary() {
        let (angle, rate, _) = run_bank(440.0, 0.0, 30);
        let drift = angle.min(1.0 - angle);
        assert!(drift < 0.02, "in-tune band must not rotate, got {angle:.4}");
        let rate = rate.expect("past the minimum span");
        assert!(
            rate.abs() < 0.05,
            "in-tune band must read ≈0 Hz, got {rate}"
        );
    }

    /// D3: silence gates every reference and holds the angle (R2 — the
    /// frozen band is state, not decay).
    #[test]
    fn test_silence_gates_and_holds_angle() {
        let fs = 44_100u32;
        let mut bank = Strobe::new(fs);
        let mut refs = [0.0; MAX_STROBE_REFS];
        refs[0] = 440.0;
        bank.retarget(StrobeRefUpdate {
            count: 1,
            refs,
            coarse_index: 1,
            spacing_hz: refs[0],
        });

        let signal: Vec<f32> = (0..BASS_WINDOW_SIZE + 10 * HOP_SIZE)
            .map(|i| 0.1 * (2.0 * std::f32::consts::PI * 441.0 * i as f32 / fs as f32).sin())
            .collect();
        let mut angle = 0.0;
        let mut frame_buf = ProcessingFrame::new();
        for h in 0..10 {
            frame_buf.audio_buffer[..BASS_WINDOW_SIZE]
                .copy_from_slice(&signal[h * HOP_SIZE..h * HOP_SIZE + BASS_WINDOW_SIZE]);
            let frame = bank.process(&frame_buf, 0.005, false);
            angle = frame.angle[0];
        }

        let silence = vec![0.0f32; BASS_WINDOW_SIZE];
        frame_buf.audio_buffer[..BASS_WINDOW_SIZE].copy_from_slice(&silence);
        for _ in 0..5 {
            let frame = bank.process(&frame_buf, 0.005, false);
            assert!(frame.gated[0], "silence must gate the band");
            assert_eq!(frame.angle[0], angle, "gated angle must hold");
        }
    }

    /// R3: a deep-bass reference set selects the long window; a treble set
    /// does not. (Boundary: spacing < 2·fs/1024 ≈ 86 Hz at 44.1 kHz.)
    #[test]
    fn test_long_window_selection() {
        let mut bank = Strobe::new(44_100);
        let mut refs = [0.0; MAX_STROBE_REFS];
        refs[0] = 27.5; // A0
        bank.retarget(StrobeRefUpdate {
            count: 1,
            refs,
            coarse_index: 1,
            spacing_hz: refs[0],
        });
        assert!(bank.long_window);
        refs[0] = 440.0;
        bank.retarget(StrobeRefUpdate {
            count: 1,
            refs,
            coarse_index: 1,
            spacing_hz: refs[0],
        });
        assert!(!bank.long_window);
    }
}
