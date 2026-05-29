// tuner-core/src/tests/audio_tests.rs
use crate::algorithms::{pitch, spectral};
use realfft::RealFftPlanner;
use rustfft::num_complex::Complex;
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
    let mut magnitudes = vec![0.0f32; buffer_size / 2];

    spectral::perform_fft(
        &audio,
        &mut time_buffer,
        &mut freq_buffer,
        &r2c,
        buffer_size,
    );
    spectral::spectrum_to_magnitudes(&freq_buffer, buffer_size, &mut magnitudes);

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
fn test_quinn_second_estimator() {
    let exact_freq = 53.833_008; // Exactly 10 cycles inside 8192 window to simulate 0 DC offset
    let audio = generate_sine_wave(exact_freq, 44100, 8192);
    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(8192);

    let mut time_buffer = vec![0.0; 8192];
    let mut freq_buffer = vec![Complex { re: 0.0, im: 0.0 }; 4097];
    let mut magnitudes = vec![0.0f32; 4096];

    spectral::perform_fft(&audio, &mut time_buffer, &mut freq_buffer, &r2c, 8192);
    spectral::spectrum_to_magnitudes(&freq_buffer, 8192, &mut magnitudes);

    let freq = pitch::quinn_second_estimator(&magnitudes, 44100, exact_freq)
        .expect("Quinn should find the peak");

    assert!(
        (freq - exact_freq).abs() < 1.0,
        "Expected ~{}, got {}",
        exact_freq,
        freq
    );
}
