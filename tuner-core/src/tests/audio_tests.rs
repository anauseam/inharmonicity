// tuner-core/src/tests/audio_tests.rs
use crate::fft;
use std::f32::consts::PI;

/// Generates a perfect sine wave signal
fn generate_sine(frequency: f32, sample_rate: usize, duration_sec: f32) -> Vec<f32> {
    let num_samples = (sample_rate as f32 * duration_sec) as usize;
    (0..num_samples)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            (2.0 * PI * frequency * t).sin()
        })
        .collect()
}

#[test]
fn test_fft_magnitude_calculation() {
    let sample_rate = 44100;
    let target_freq = 440.0; // A4

    // Generate exactly BUFFER_SIZE samples of a pure A4 tone
    let buffer_size = crate::audio::BUFFER_SIZE;
    let duration = buffer_size as f32 / sample_rate as f32;
    let signal = generate_sine(target_freq, sample_rate, duration);

    // Process FFT
    let spectrum = fft::perform_fft(&signal);
    let magnitudes = fft::spectrum_to_magnitudes(&spectrum);

    // Ensure output array has exactly 1024 bins (BUFFER_SIZE / 2)
    assert_eq!(magnitudes.len(), buffer_size / 2);

    // Find highest peak
    let mut peak_idx = 0;
    let mut peak_mag: f32 = 0.0;

    for (i, &mag) in magnitudes.iter().enumerate() {
        if mag > peak_mag {
            peak_mag = mag;
            peak_idx = i;
        }
    }

    let detected_freq = peak_idx as f32 * sample_rate as f32 / buffer_size as f32;

    // Verify peak magnitude is not negative, NaN, or infinite
    assert!(peak_mag.is_finite());
    assert!(peak_mag > 0.0);

    // The peak bin frequency should be within half a bin resolution for accuracy, but we check 1 bin just in case
    let bin_resolution = sample_rate as f32 / buffer_size as f32;
    assert!(
        (detected_freq - target_freq).abs() <= bin_resolution,
        "Detected freq {} is not within {} Hz of target {}",
        detected_freq,
        bin_resolution,
        target_freq
    );
}
