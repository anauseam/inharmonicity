//! Offline diagnostic: pitch-raise reach of TWM configs — the 1¢-resolution
//! key-40 detuning sweep behind ADR 0006's "pitch-raise-reach cost" numbers
//! (canonical 78¢ / conservative default 69¢; cf. `refined_recovers_detuned_notes`).
//!
//! For each config, sweep the true detuning 0..=100¢ (and 0..=-100¢) in 1¢
//! steps on ideal synthetic peaks (profile partials × s_true, 1/n magnitudes,
//! 20 partials) and report the reach: the last cent value before the full
//! discovery pipeline (Stage A K=3 → Stage B refine) first mis-identifies the
//! key. Extra configs are passed as argv 4-tuples: `name q r rho ...`
//! (p=0.5, λ=18 held); with no args it reports canonical M&B and the shipped
//! default. Used 2026-07-02 to price the pinned arm-6 candidate (seed-7 trial
//! 1898: 80¢ — no reach cost; ADR 0006 "Corrections" item 2).

use tuner_core::algorithms::discovery;
use tuner_core::algorithms::twm::TwmConfig;
use tuner_core::models::SpectralPeak;
use tuner_core::models::{KeyProfile, NOTES, get_expected_beta};

fn build_profiles() -> Box<[KeyProfile; 88]> {
    let mut v = Vec::with_capacity(88);
    for i in 0..88 {
        v.push(KeyProfile::new(
            NOTES[i].frequency,
            get_expected_beta(i as u8),
        ));
    }
    Box::new(<[KeyProfile; 88]>::try_from(v).ok().unwrap())
}

fn synth_peaks(profile: &KeyProfile, s_true: f32, n_partials: usize) -> Vec<SpectralPeak> {
    (0..profile.valid_partial_count.min(n_partials))
        .map(|i| SpectralPeak {
            frequency: profile.predicted_partials[i] * s_true,
            magnitude: 1.0 / (i as f32 + 1.0),
        })
        .collect()
}

/// Last cent (walking outward from 0 in `dir`) before the first failure.
fn reach(profiles: &[KeyProfile; 88], cfg: &TwmConfig, key: usize, dir: f32) -> i32 {
    let mut last_ok = -1;
    for c in 0..=100 {
        let cents = dir * c as f32;
        let s_true = (cents / 1200.0).exp2();
        let peaks = synth_peaks(&profiles[key], s_true, 20);
        let res = discovery::discover(&peaks, profiles, cfg, true);
        if res.key_index as usize == key {
            last_ok = c;
        } else {
            break;
        }
    }
    last_ok
}

fn main() {
    let profiles = build_profiles();
    let key = 40_usize;

    let canonical = TwmConfig {
        q: 1.4,
        r: 0.5,
        rho: 0.33,
        ..TwmConfig::default()
    };

    let mut configs: Vec<(String, TwmConfig)> = vec![
        ("canonical M&B (q=1.4 r=0.5 rho=0.33)".into(), canonical),
        (
            "conservative default (q=3.88 r=1.426 rho=0.298)".into(),
            TwmConfig::default(),
        ),
    ];

    // Candidates piped in as "name q r rho" lines via argv pairs: name q r rho ...
    let args: Vec<String> = std::env::args().skip(1).collect();
    for chunk in args.chunks(4) {
        if chunk.len() == 4 {
            configs.push((
                chunk[0].clone(),
                TwmConfig {
                    q: chunk[1].parse().unwrap(),
                    r: chunk[2].parse().unwrap(),
                    rho: chunk[3].parse().unwrap(),
                    ..TwmConfig::default()
                },
            ));
        }
    }

    println!("key {key} 1¢-resolution reach (last cent before first failure), 20 partials");
    println!("{:<50} {:>7} {:>7}", "config", "+reach", "-reach");
    for (name, cfg) in &configs {
        let up = reach(&profiles, cfg, key, 1.0);
        let down = reach(&profiles, cfg, key, -1.0);
        println!("{name:<50} {up:>6}¢ {:>6}¢", -down);
    }
}
