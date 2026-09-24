//! The unison line estimator against synthetic truth.
//!
//! Report 0012 states the law this asserts: a pair resolves when its separation
//! clears `2/T` and not before, the transition is sharp rather than gradual, and
//! above ≈ 1.6 × that floor the reported split is exact to a systematic −0.085 Hz.
//!
//! The trials are driven as audio through the shipped Goertzel front end —
//! analysis window, decay and noise all in the loop — not through a model of
//! it. The full E6 sweep this is drawn from is `cargo lab strobe replay`.

use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_RATE_HZ, HOP_SIZE, SAMPLE_RATE};
use tuner_core::strobe::{MAX_STROBE_REFS, Strobe, StrobeRefUpdate};

/// Mid-treble, where the feature lives and the 1024-sample window is the one
/// that runs.
const REF_HZ: f32 = 440.0;

/// Ring cap: the longest record the bank publishes, `2/T` = 1.54 Hz.
const FULL_RECORD_HOPS: usize = 56;

const TRIALS: usize = 20;

/// One synthetic string: a partial at `REF_HZ + offset_hz`, decaying.
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

/// Renders `sources` as audio. `snr_db` is against the total source power.
fn synth_audio(sources: &[Source], hops: usize, snr_db: f32, seed: u32) -> Vec<f32> {
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
                let f = REF_HZ + s.offset_hz;
                let phase = 2.0 * std::f32::consts::PI * f * t + 1.1 * k as f32;
                x += s.amplitude * (-t / s.tau_secs).exp() * phase.sin();
            }
            x
        })
        .collect()
}

/// Drives the shipped bank over one trial and returns the split it reported
/// at the longest record it reached, or `None` where it published one line.
fn reported_split(split_hz: f32, hops: usize, seed: u32) -> Option<f32> {
    let sources = [
        Source {
            offset_hz: -split_hz / 2.0,
            amplitude: 1.0,
            tau_secs: 1.5,
        },
        Source {
            offset_hz: split_hz / 2.0,
            amplitude: 1.0,
            tau_secs: 1.5,
        },
    ];
    let audio = synth_audio(&sources, hops, 40.0, seed);

    let mut refs = [0.0f32; MAX_STROBE_REFS];
    refs[0] = REF_HZ;
    let mut strobe = Strobe::new(SAMPLE_RATE);
    strobe.retarget(StrobeRefUpdate {
        count: 1,
        refs,
        coarse_index: 0,
        spacing_hz: REF_HZ,
    });

    let mut frame = tuner_core::pipeline::ProcessingFrame::new();
    let mut best_record = 0usize;
    let mut best = None;
    for h in 0..audio.len().saturating_sub(BASS_WINDOW_SIZE) / HOP_SIZE {
        frame.audio_buffer[..BASS_WINDOW_SIZE]
            .copy_from_slice(&audio[h * HOP_SIZE..h * HOP_SIZE + BASS_WINDOW_SIZE]);
        let out = strobe.process(&frame, 1e-6, false);
        // The published resolution is 2·f_hop/L, so it is the record length.
        let record = if out.line_resolution_hz[0] > 0.0 {
            (2.0 * HOP_RATE_HZ / out.line_resolution_hz[0]).round() as usize
        } else {
            0
        };
        if record > best_record {
            best_record = record;
            best = (out.line_count[0] >= 2).then(|| {
                let mut got: Vec<f32> = out.lines[0][..2].iter().map(|l| l.offset_hz).collect();
                got.sort_by(f32::total_cmp);
                got[1] - got[0]
            });
        }
    }
    best
}

/// Fraction of trials resolving two lines, and the median split they reported.
fn resolve_rate(split_hz: f32, hops: usize) -> (f32, Option<f32>) {
    let mut hits: Vec<f32> = (0..TRIALS)
        .filter_map(|t| reported_split(split_hz, hops, 0x9e37_79b9u32.wrapping_mul(t as u32 + 1)))
        .collect();
    let rate = hits.len() as f32 / TRIALS as f32;
    hits.sort_by(f32::total_cmp);
    (rate, hits.get(hits.len() / 2).copied())
}

/// The published floor for a record of `hops`.
fn two_over_t(hops: usize) -> f32 {
    2.0 * HOP_RATE_HZ / hops as f32
}

/// report 0012: the transition is sharp, not gradual — at 40 dB the outcome
/// is essentially deterministic in the split. Below the floor nothing resolves;
/// clear of it everything does.
#[test]
fn a_pair_resolves_when_it_clears_two_over_t_and_not_before() {
    let floor = two_over_t(FULL_RECORD_HOPS);
    assert!(
        (floor - 1.54).abs() < 0.01,
        "the ring cap's floor moved: 2/T = {floor:.2} Hz, report 0012 §4 states 1.54"
    );

    let (below, _) = resolve_rate(0.65 * floor, FULL_RECORD_HOPS);
    assert_eq!(
        below,
        0.0,
        "a pair at 0.65 × 2/T resolved on {:.0} % of trials; report 0012 §4 has it at 0 %",
        100.0 * below
    );

    let (above, _) = resolve_rate(1.95 * floor, FULL_RECORD_HOPS);
    assert_eq!(
        above,
        1.0,
        "a pair at 1.95 × 2/T resolved on only {:.0} % of trials; report 0012 §4 has it at 100 %",
        100.0 * above
    );
}

/// report 0012: above ~1.6 × the floor the reported split is exact, to the
/// systematic −0.085 Hz below. "Two lines" and "the right two lines" are
/// different claims, and this is the second one.
#[test]
fn a_clearly_separated_split_is_reported_exactly() {
    let floor = two_over_t(FULL_RECORD_HOPS);
    for split in [1.95 * floor, 3.25 * floor] {
        let (rate, reported) = resolve_rate(split, FULL_RECORD_HOPS);
        assert_eq!(rate, 1.0, "{split:.2} Hz did not resolve on every trial");
        let reported = reported.expect("resolved trials report a split");
        let err = reported - split;
        assert!(
            (-0.15..=0.05).contains(&err),
            "reported {reported:.3} Hz for a true {split:.3} Hz ({err:+.3} Hz); \
             report 0012 §4 states a systematic −0.085 Hz and nothing larger"
        );
    }
}

/// report 0012: at or below the floor the reported split collapses onto the
/// limit itself, whatever the truth was — survivorship, not measurement. A
/// 0.7 Hz pair is reported at 3.48 Hz there, so `2/T` must be published with the
/// lines.
#[test]
fn an_unresolvable_split_that_survives_is_reported_at_the_limit_not_the_truth() {
    // A short record, where the floor is high enough that a true 0.7 Hz pair is
    // hopeless yet occasionally looks wide enough to pass.
    let hops = 28;
    let floor = two_over_t(hops);
    let truth = 0.7;
    assert!(truth < floor, "the cell must sit below its own floor");
    if let (rate, Some(reported)) = resolve_rate(truth, hops)
        && rate > 0.0
    {
        assert!(
            reported > floor,
            "a survivor at {reported:.2} Hz sits below the {floor:.2} Hz floor it \
             cleared to be seen — the collapse report 0012 §4 describes is absent"
        );
        assert!(
            reported > 2.0 * truth,
            "reported {reported:.2} Hz for a true {truth:.2} Hz is close enough to \
             the truth to be a measurement; report 0012 §4 has it collapsing onto the limit"
        );
    }
}
