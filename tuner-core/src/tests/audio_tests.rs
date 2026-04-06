// tuner-core/src/tests/audio_tests.rs
use crate::algorithms::{pitch, spectral, metrics};
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
    let mut magnitudes = vec![0.0f32; buffer_size / 2];

    spectral::perform_fft(&audio, &mut time_buffer, &mut freq_buffer, &r2c, buffer_size);
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
    let exact_freq = 53.8330078125; // Exactly 10 cycles inside 8192 window to simulate 0 DC offset
    let audio = generate_sine_wave(exact_freq, 44100, 8192);
    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(8192);
    
    let mut time_buffer = vec![0.0; 8192];
    let mut freq_buffer = vec![Complex { re: 0.0, im: 0.0 }; 4097];
    let mut magnitudes = vec![0.0f32; 4096];
    
    spectral::perform_fft(&audio, &mut time_buffer, &mut freq_buffer, &r2c, 8192);
    spectral::spectrum_to_magnitudes(&freq_buffer, 8192, &mut magnitudes);
    
    let freq =
        pitch::quinn_second_estimator(&magnitudes, 44100, exact_freq).expect("Quinn should find the peak");

    assert!((freq.frequency - exact_freq).abs() < 1.0, "Expected ~{}, got {}", exact_freq, freq.frequency);
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

#[test]
fn test_phantom_mask_zeroes_target_bins() {
    let sample_rate = 44100_u32;
    let window_size = 8192_usize;
    let f0 = 55.0_f32;     // A1 — deep bass note
    let beta = 2.5e-4_f32; // typical upright piano bass β

    let hz_per_bin = sample_rate as f32 / window_size as f32;
    let mut magnitudes = vec![1.0_f32; window_size / 2];

    crate::algorithms::phantom::apply_phantom_mask(&mut magnitudes, f0, beta, sample_rate, window_size);

    // Verify at least one (2,3) combination region was zeroed
    let f2 = 2.0 * f0 * (1.0 + beta * 4.0_f32).sqrt();
    let f3 = 3.0 * f0 * (1.0 + beta * 9.0_f32).sqrt();
    let f_center = f2 + f3;
    let center_bin = (f_center / hz_per_bin).round() as usize;

    assert_eq!(magnitudes[center_bin], 0.0, "Phantom center bin should be zeroed");
}

#[test]
fn test_asymmetry_index_pure_tone() {
    let mut magnitudes = vec![0.0_f32; 512];
    let peak_bin = 100;
    magnitudes[peak_bin - 1] = 0.5;
    magnitudes[peak_bin]     = 1.0;
    magnitudes[peak_bin + 1] = 0.5;

    let delta = 0.0; // perfect center alignment
    let asym = crate::algorithms::pitch::spectral_asymmetry_index(&magnitudes, peak_bin, delta);
    assert!(asym < 1.85, "Pure symmetric peak should pass asymmetry check");
}

#[test]
fn test_asymmetry_index_beating_unison() {
    let mut magnitudes = vec![0.0_f32; 512];
    let peak_bin = 100;
    magnitudes[peak_bin - 1] = 0.9; // left shoulder elevated
    magnitudes[peak_bin]     = 1.0;
    magnitudes[peak_bin + 1] = 0.1; // right shoulder suppressed
    
    let delta = 0.0;
    let asym = crate::algorithms::pitch::spectral_asymmetry_index(&magnitudes, peak_bin, delta);
    assert!(asym > 1.85, "Asymmetric peak should fail coherence check");
}
