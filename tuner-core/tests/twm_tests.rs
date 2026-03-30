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

// 3. key_hint = Some(440.0) → ~440 Hz (targeted mode)
#[test]
fn test_twm_targeted_mode() {
    // Deliberately messy peaks with a stronger wrong harmonic series
    let peaks = vec![
        SpectralPeak { freq: 220.0, magnitude: 2.0 },
        SpectralPeak { freq: 440.0, magnitude: 1.0 },
        SpectralPeak { freq: 660.0, magnitude: 1.5 },
        SpectralPeak { freq: 880.0, magnitude: 0.8 },
    ];
    // Asking it to look around 440 Hz specifically
    let result = detect_pitch_twm(&peaks, 44100, RoutingState::Unclassified, Some(440.0), None);
    assert!(result.is_some());
    let (f0, _) = result.unwrap();
    assert!((f0 - 440.0).abs() < 10.0, "Targeted mode failed, got {}", f0);
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

// 5. < 3 peaks → None (guard)
#[test]
fn test_twm_insufficient_peaks() {
    let peaks = vec![
        SpectralPeak { freq: 440.0, magnitude: 1.0 },
        SpectralPeak { freq: 880.0, magnitude: 0.8 },
    ];
    let result = detect_pitch_twm(&peaks, 44100, RoutingState::Unclassified, None, None);
    assert!(result.is_none(), "Should reject with < 3 peaks");
}
