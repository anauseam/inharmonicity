// tuner-core/src/tests/audio_tests.rs
use crate::algorithms::{pitch, dpyin, spectral, metrics};
use rustfft::num_complex::Complex;
use realfft::RealFftPlanner;
use std::f32::consts::PI;

/// Generates a perfect sine wave signal
fn generate_sine_wave(freq: f32, sample_rate: u32, length: usize) -> Vec<f32> {
    (0..length)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            (t * freq * 2.0 * PI).sin()
        })
        .collect()
}

#[test]
fn test_fft_magnitude_calculation() {
    let sample_rate = 44100;
    let target_freq = 440.0; // A4

    let buffer_size = crate::audio::WINDOW_SIZE;
    let audio = generate_sine_wave(target_freq, sample_rate as u32, buffer_size);

    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(buffer_size);
    let mut time_buffer = vec![0.0; buffer_size];
    let mut freq_buffer = vec![Complex { re: 0.0, im: 0.0 }; buffer_size / 2 + 1];

    spectral::perform_fft(&audio, &mut time_buffer, &mut freq_buffer, &r2c, buffer_size);
    let magnitudes = spectral::spectrum_to_magnitudes(&freq_buffer, buffer_size);

    assert_eq!(magnitudes.len(), buffer_size / 2);

    let mut peak_idx = 0;
    let mut peak_mag: f32 = 0.0;

    for (i, &mag) in magnitudes.iter().enumerate() {
        if mag > peak_mag {
            peak_mag = mag;
            peak_idx = i;
        }
    }

    let detected_freq = peak_idx as f32 * sample_rate as f32 / buffer_size as f32;

    assert!(peak_mag.is_finite());
    assert!(peak_mag > 0.0);

    let bin_resolution = sample_rate as f32 / buffer_size as f32;
    assert!(
        (detected_freq - target_freq).abs() <= bin_resolution,
        "Detected freq {} is not within {} Hz of target {}",
        detected_freq,
        bin_resolution,
        target_freq
    );
}

#[test]
fn test_qifft_core() {
    let audio = generate_sine_wave(440.0, 44100, 2048);
    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(2048);
    
    let mut time_buffer = vec![0.0; 2048];
    let mut freq_buffer = vec![Complex { re: 0.0, im: 0.0 }; 1025];
    
    spectral::perform_fft(&audio, &mut time_buffer, &mut freq_buffer, &r2c, 2048);
    let magnitudes = spectral::spectrum_to_magnitudes(&freq_buffer, 2048);
    
    let freq = pitch::detect_pitch_qifft(&magnitudes, 44100).unwrap();
    assert!((freq - 440.0).abs() < 1.0, "Expected ~440.0, got {}", freq);
}

#[test]
fn test_dpyin_core() {
    let audio = generate_sine_wave(110.0, 44100, 8192);
    let mut scratch = vec![0.0; 8192];
    
    let result = dpyin::detect_pitch_dpyin(
        &audio, 44100, &mut scratch, None
    );
    let (freq, _conf) = result.expect("DPYIN failed to detect pitch");
    assert!((freq - 110.0).abs() < 2.0, "Expected ~110.0, got {}", freq);
}

#[test]
fn test_band_energy_routing() {
    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(2048);

    // Bass note C2 (65.4 Hz)
    let bass_audio = generate_sine_wave(65.4, 44100, 2048);
    let mut time_buffer = vec![0.0; 2048];
    let mut freq_buffer = vec![Complex { re: 0.0, im: 0.0 }; 1025];
    
    spectral::perform_fft(&bass_audio, &mut time_buffer, &mut freq_buffer, &r2c, 2048);
    let bass_ratio = metrics::evaluate_band_energy_ratio(&freq_buffer);
    assert!(bass_ratio > 0.25, "Bass ratio too low: {}", bass_ratio);
    
    // Treble note C6 (1046.5 Hz)
    let treble_audio = generate_sine_wave(1046.5, 44100, 2048);
    spectral::perform_fft(&treble_audio, &mut time_buffer, &mut freq_buffer, &r2c, 2048);
    let treble_ratio = metrics::evaluate_band_energy_ratio(&freq_buffer);
    assert!(treble_ratio < 0.15, "Treble ratio too high: {}", treble_ratio);
}
