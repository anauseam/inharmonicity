use tuner_core::algorithms::twm::{detect_pitch_twm, SpectralPeak};
use tuner_core::engine::RoutingState;

// 1. Sine 440 Hz → TWM returns ~440 Hz
#[test]
fn test_twm_basic_440hz() {
    let peaks = vec![
        SpectralPeak { freq: 440.0, magnitude: 1.0 },
        SpectralPeak { freq: 880.0, magnitude: 0.8 },
        SpectralPeak { freq: 1320.0, magnitude: 0.6 },
        SpectralPeak { freq: 1760.0, magnitude: 0.4 },
    ];
    let result = detect_pitch_twm(&peaks, 44100, RoutingState::Unclassified, None, None);
    assert!(result.is_some());
    let (f0, confidence) = result.unwrap();
    assert!((f0 - 440.0).abs() < 5.0, "Expected f0 around 440.0 Hz, got {}", f0);
    assert!(confidence.is_some());
}

// 2. Sines 440 + 880 Hz → TWM returns 440 Hz (not octave)
#[test]
fn test_twm_octave_robustness() {
    let peaks = vec![
        SpectralPeak { freq: 440.0, magnitude: 1.0 },
        SpectralPeak { freq: 880.0, magnitude: 1.2 }, // Octave is stronger
        SpectralPeak { freq: 1320.0, magnitude: 0.8 },
        SpectralPeak { freq: 1760.0, magnitude: 0.6 },
        SpectralPeak { freq: 2200.0, magnitude: 0.4 },
    ];
    let result = detect_pitch_twm(&peaks, 44100, RoutingState::Unclassified, None, None);
    assert!(result.is_some());
    let (f0, _) = result.unwrap();
    assert!((f0 - 440.0).abs() < 5.0, "TWM failed octave robustness, got {}", f0);
}



// 4. inharmonicity_b = Some(0.01) → correct F0 (stretched template)
#[test]
fn test_twm_inharmonic_template() {
    let b = 0.01;
    // Generate peaks according to stretched math: f_n = n * 440 * sqrt(1 + b * n^2)
    let f0_true = 440.0;
    
    let mut peaks = Vec::new();
    for n in 1..=5 {
        let n_f32 = n as f32;
        let stretched_freq = f0_true * n_f32 * (1.0 + b * n_f32 * n_f32).sqrt();
        peaks.push(SpectralPeak { freq: stretched_freq, magnitude: 1.0 / n_f32 });
    }
    
    // Evaluate without B (Should have higher error, maybe pick slightly wrong F0)
    let result_no_b = detect_pitch_twm(&peaks, 44100, RoutingState::Unclassified, Some(440.0), None);
    
    // Evaluate with B
    let result_with_b = detect_pitch_twm(&peaks, 44100, RoutingState::Unclassified, Some(440.0), Some(b));
    
    assert!(result_with_b.is_some());
    let (f0_b, conf_b) = result_with_b.unwrap();
    assert!((f0_b - 440.0).abs() < 2.0, "Inharmonic template failed, got {}", f0_b);
    
    if let Some((_, conf_no_b)) = result_no_b {
        // The one with the matching B should have higher confidence (lower error)
        assert!(conf_b.unwrap() > conf_no_b.unwrap());
    }
}

// 5. 0 peaks → None (guard)
#[test]
fn test_twm_zero_peaks() {
    let peaks = vec![];
    let result = detect_pitch_twm(&peaks, 44100, RoutingState::Unclassified, None, None);
    assert!(result.is_none(), "Should reject with 0 peaks");
}

#[test]
fn test_twm_octave_rejection() {
    // Simulate piano C4: weak fundamental at 261 Hz, strong 2nd/3rd partials
    let peaks = vec![
        SpectralPeak { freq: 261.6, magnitude: 0.1 },  // weak fundamental
        SpectralPeak { freq: 523.2, magnitude: 1.0 },  // strong 2nd harmonic
        SpectralPeak { freq: 784.9, magnitude: 0.8 },  // 3rd harmonic
        SpectralPeak { freq: 1046.5, magnitude: 0.6 }, // 4th harmonic
        SpectralPeak { freq: 1308.1, magnitude: 0.4 }, // 5th harmonic
    ];
    let result = detect_pitch_twm(&peaks, 44100, RoutingState::LockedTreble, None, None);
    assert!(result.is_some());
    let (f0, _) = result.unwrap();
    assert!((f0 - 261.6).abs() < 10.0, "Expected C4 (~261 Hz), got {} Hz", f0);
}

#[test]
fn test_twm_treble_weak_signal() {
    // Simulate fast-decaying treble note with 1-2 peaks and amplitude well below 0.01
    let peaks = vec![
        SpectralPeak { freq: 2093.0, magnitude: 0.005 },
        SpectralPeak { freq: 4186.0, magnitude: 0.002 },
    ];
    let result = detect_pitch_twm(&peaks, 44100, RoutingState::LockedTreble, None, None);
    assert!(result.is_some(), "Should not reject with < 3 peaks or weak signals");
    let (f0, _) = result.unwrap();
    assert!((f0 - 2093.0).abs() < 50.0, "Expected C7 (~2093 Hz), got {} Hz", f0);
}

#[test]
fn test_twm_subharmonic_veto_ignores_noise() {
    // A 440 Hz fundamental with a 4% noise floor peak at 220 Hz
    let peaks = vec![
        SpectralPeak { freq: 220.0, magnitude: 0.04 }, // 4% noise
        SpectralPeak { freq: 440.0, magnitude: 1.0 },  // 100% fundamental
    ];
    let result = detect_pitch_twm(&peaks, 44100, RoutingState::LockedTreble, Some(440.0), None);
    assert!(result.is_some(), "Should not reject candidate 440 due to 4% noise");
    let (f0, _) = result.unwrap();
    assert!((f0 - 440.0).abs() < 5.0, "Expected ~440 Hz, got {}", f0);
}

#[test]
fn test_twm_subharmonic_veto_rejects_weak_fundamental() {
    // A C4 note with 10% fundamental at 261.6 Hz and 100% 2nd partial at 523.2 Hz.
    // Testing candidate = 523.2 Hz should be strictly vetoed by the 10% sub-peak.
    let peaks = vec![
        SpectralPeak { freq: 261.6, magnitude: 0.1 },  // 10% fundamental
        SpectralPeak { freq: 523.2, magnitude: 1.0 },  // 100% partial
    ];
    let result = detect_pitch_twm(&peaks, 44100, RoutingState::LockedTreble, Some(523.2), None);
    // 523.2 generates a veto of 0.300 against a 0.25 ceiling, so it strictly fails.
    assert!(result.is_none(), "Candidate 523.2 should be decisively vetoed by the 10% subharmonic");
}
