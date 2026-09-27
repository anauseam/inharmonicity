//! Per-frame cost of split discovery against the audio callback's budget.
//!
//! Stage A scores all 88 key profiles once; Stage B refines the top `TOP_K` with a
//! pre-grid plus golden-section search, ≈ 18 further `score_candidate` calls each,
//! so refined sits near `1 + 18·TOP_K/88` times discrete, a ratio that does not
//! depend on the hardware. That ratio moving is the signature of an allocation or
//! an O(n²) creeping into the scoring loop; the absolute figure is measured against
//! one hop, `HOP_SIZE / SAMPLE_RATE` = 23.2 ms.
//!
//! The templates discovery scores against are rebuilt off the hot path, once per
//! curve bundle: the fit that sets their B, on the Worker, and the 88 templates
//! `ProfileSender::push_templates` builds from it. Both are priced here against
//! the bundle.
//!
//! Run with `cargo bench -p tuner-core`.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use tuner_core::algorithms::curves;
use tuner_core::algorithms::discovery::discover;
use tuner_core::algorithms::rigaud::BXi;
use tuner_core::algorithms::twm::TwmConfig;
use tuner_core::models::{
    CurveInput, CurveKeyData, KeyProfile, NOTES, SpectralPeak, get_expected_beta,
};

fn profiles() -> Box<[KeyProfile; 88]> {
    let v: Vec<KeyProfile> = (0..88)
        .map(|i| KeyProfile::new(NOTES[i].frequency, get_expected_beta(i as u8)))
        .collect();
    let arr: [KeyProfile; 88] = v.try_into().unwrap();
    Box::new(arr)
}

/// Peaks on the profile's stretched partials × `s_true`, 1/n magnitudes,
/// ascending frequency — the `mask_peaks` output contract.
fn synth_peaks(profile: &KeyProfile, s_true: f32, n_partials: usize) -> Vec<SpectralPeak> {
    (0..profile.valid_partial_count.min(n_partials))
        .map(|i| SpectralPeak {
            frequency: profile.predicted_partials[i] * s_true,
            magnitude: 1.0 / (i as f32 + 1.0),
        })
        .collect()
}

fn discovery_per_frame(c: &mut Criterion) {
    let profiles = profiles();
    let cfg = TwmConfig::default();
    // D2, 2 % flat, 24 partials — a mid-bass frame with a full peak list.
    let peaks = synth_peaks(&profiles[17], 0.98, 24);

    c.bench_function("discovery, discrete (Stage A only), one frame", |b| {
        b.iter(|| black_box(discover(&peaks, &profiles, &cfg, false)))
    });
    c.bench_function("discovery, refined (Stage A + B), one frame", |b| {
        b.iter(|| black_box(discover(&peaks, &profiles, &cfg, true)))
    });
}

/// One template refresh: the fit on a complete profile, every key measured once at
/// 32 partials with the bass at twice the default's B, so all 27 bass keys are
/// fit points; then the 88 templates built from its B.
fn template_refresh(c: &mut Criterion) {
    let mut input = CurveInput::default();
    for (k, key) in input.keys.iter_mut().enumerate() {
        let scale = if k < 27 { 2.0 } else { 1.0 };
        let f0 = f64::from(NOTES[k].frequency);
        *key = Some(CurveKeyData {
            b: scale * BXi::DEFAULT_MEDIUM.b_at_key(k),
            f0,
            partials: (1..=32).map(|n| (n, f64::from(n) * f0, 1.0)).collect(),
        });
    }
    c.bench_function("discovery templates, the fit, one bundle", |b| {
        b.iter(|| black_box(curves::template_b_fit(black_box(&input))))
    });
    let bxi = curves::template_b_fit(&input);
    let template_b: [f32; 88] = core::array::from_fn(|k| bxi.b_at_key(k) as f32);
    c.bench_function(
        "discovery templates, 88 KeyProfile::new, one refresh",
        |b| {
            b.iter(|| {
                let templates: [KeyProfile; 88] =
                    core::array::from_fn(|k| KeyProfile::new(NOTES[k].frequency, template_b[k]));
                black_box(templates)
            })
        },
    );
}

criterion_group!(benches, discovery_per_frame, template_refresh);
criterion_main!(benches);
