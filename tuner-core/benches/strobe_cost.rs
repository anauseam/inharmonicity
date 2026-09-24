//! Per-hop cost of the strobe bank against the audio callback's budget.
//!
//! Both cases are the worst case a real capture does not reach — every
//! reference live and every ring at the cap, where a capture's upper partials
//! gate out and stop transforming. The budget to compare against is one hop:
//! `HOP_SIZE / SAMPLE_RATE` = 23.2 ms.
//!
//! Run with `cargo bench -p tuner-core`. Reproduces report 0012's E9.

use criterion::{Criterion, criterion_group, criterion_main};
use rustfft::num_complex::Complex;
use std::hint::black_box;

use tuner_core::algorithms::peaks::{LineScratch, MAX_UNISON_LINES, resolve_lines};
use tuner_core::algorithms::spectral;
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_RATE_HZ, HOP_SIZE, SAMPLE_RATE};
use tuner_core::models::UnisonLine;
use tuner_core::strobe::unison::UNISON_RING_HOPS;
use tuner_core::strobe::{MAX_STROBE_REFS, Strobe, StrobeRefUpdate};

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

/// Twelve harmonics of A3, each with a companion 0.9 Hz away so every ring has
/// a two-line problem to solve rather than a trivially single one.
fn worst_case_audio(hops: usize) -> Vec<f32> {
    let mut noise = Noise(0x5bf0_3635);
    (0..BASS_WINDOW_SIZE + hops * HOP_SIZE)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let mut x = 0.01 * noise.next();
            for k in 0..MAX_STROBE_REFS {
                for f in [220.0 * (k + 1) as f32, 220.0 * (k + 1) as f32 + 0.9] {
                    x += (2.0 * std::f32::consts::PI * f * t + 1.1 * k as f32).sin();
                }
            }
            x
        })
        .collect()
}

fn bank_per_hop(c: &mut Criterion) {
    let hops = UNISON_RING_HOPS * 2;
    let audio = worst_case_audio(hops);
    let mut refs = [0.0f32; MAX_STROBE_REFS];
    for (i, r) in refs.iter_mut().enumerate() {
        *r = 220.0 * (i + 1) as f32;
    }

    let mut strobe = Strobe::new(SAMPLE_RATE);
    strobe.retarget(StrobeRefUpdate {
        count: MAX_STROBE_REFS,
        refs,
        coarse_index: 0,
        spacing_hz: 440.0,
    });
    let mut frame = tuner_core::pipeline::ProcessingFrame::new();
    // Fill every ring to the cap first: the hops under test are past the fill.
    for h in 0..UNISON_RING_HOPS {
        frame.audio_buffer[..BASS_WINDOW_SIZE]
            .copy_from_slice(&audio[h * HOP_SIZE..h * HOP_SIZE + BASS_WINDOW_SIZE]);
        strobe.process(&frame, 1e-6, false);
    }

    let mut h = UNISON_RING_HOPS;
    c.bench_function("strobe bank, 12 references, one hop", |b| {
        b.iter(|| {
            let start = (h % hops) * HOP_SIZE;
            frame.audio_buffer[..BASS_WINDOW_SIZE]
                .copy_from_slice(&audio[start..start + BASS_WINDOW_SIZE]);
            h += 1;
            black_box(strobe.process(&frame, 1e-6, false));
        })
    });
}

fn resolve_lines_at_the_cap(c: &mut Criterion) {
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

    c.bench_function("resolve_lines x 12 records at the cap", |b| {
        b.iter(|| {
            for _ in 0..MAX_STROBE_REFS {
                black_box(resolve_lines(
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
        })
    });
}

criterion_group!(benches, bank_per_hop, resolve_lines_at_the_cap);
criterion_main!(benches);
