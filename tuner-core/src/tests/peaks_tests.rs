use crate::algorithms::peaks::{SpectralPeak, extract_peaks};
use rustfft::num_complex::Complex;

#[test]
fn single_peak_extraction() {
    let mut magnitudes = vec![0.0_f32; 1024];
    let sample_rate = 44100;
    let fft_size = 2048;

    // Create a peak at bin 10 (approx 215 Hz).
    magnitudes[9] = 0.5;
    magnitudes[10] = 1.0;
    magnitudes[11] = 0.5;

    let complex: Vec<Complex<f32>> = magnitudes.iter().map(|&m| Complex::new(m, 0.0)).collect();
    let mut peaks = [SpectralPeak::default(); 64];
    let count = extract_peaks(
        &magnitudes,
        &complex,
        sample_rate,
        fft_size,
        0.01,
        &mut peaks,
    );

    assert_eq!(count, 1);
    // Since neighbors are equal, offset should be 0
    let expected_freq = 10.0 * (sample_rate as f32 / fft_size as f32);
    assert!((peaks[0].frequency - expected_freq).abs() < 0.1);
    assert_eq!(peaks[0].magnitude, 1.0);
}

#[test]
fn noise_floor_filtering() {
    let mut magnitudes = vec![0.0_f32; 1024];
    let sample_rate = 44100;
    let fft_size = 2048;

    // Main peak (global max = 1.0)
    magnitudes[9] = 0.5;
    magnitudes[10] = 1.0;
    magnitudes[11] = 0.5;

    // Small peak below noise floor ratio (0.1)
    magnitudes[29] = 0.05;
    magnitudes[30] = 0.08;
    magnitudes[31] = 0.05;

    let complex: Vec<Complex<f32>> = magnitudes.iter().map(|&m| Complex::new(m, 0.0)).collect();
    let mut peaks = [SpectralPeak::default(); 64];
    // 0.1 noise floor -> 0.08 is below it, should be ignored
    let count = extract_peaks(
        &magnitudes,
        &complex,
        sample_rate,
        fft_size,
        0.1,
        &mut peaks,
    );

    assert_eq!(count, 1);
    assert_eq!(peaks[0].magnitude, 1.0);
}

#[test]
fn pure_silence() {
    let magnitudes = vec![0.0_f32; 1024];
    let complex = vec![Complex::new(0.0_f32, 0.0); 1024];
    let mut peaks = [SpectralPeak::default(); 64];

    let count = extract_peaks(&magnitudes, &complex, 44100, 2048, 0.01, &mut peaks);
    assert_eq!(count, 0);
}

#[test]
fn edge_index_arrays() {
    let mut magnitudes = vec![0.0_f32; 10];

    // Peak at index 0 (not a local maximum since there's no left neighbor to check, algorithm skips boundary)
    magnitudes[0] = 1.0;
    magnitudes[1] = 0.5;

    // Peak at index 9 (last element, skipped)
    magnitudes[8] = 0.5;
    magnitudes[9] = 1.0;

    let complex = vec![Complex::new(0.0_f32, 0.0); 10];
    let mut peaks = [SpectralPeak::default(); 64];
    let count = extract_peaks(&magnitudes, &complex, 44100, 20, 0.01, &mut peaks);

    // Should not panic, and should not find these edge peaks
    assert_eq!(count, 0);
}

#[test]
fn peak_count_cap() {
    let mut magnitudes = vec![0.0_f32; 1024];

    // Create 100 peaks
    for i in 1..101 {
        let bin = i * 4;
        magnitudes[bin - 1] = 0.5;
        magnitudes[bin] = 1.0;
        magnitudes[bin + 1] = 0.5;
    }

    let complex = vec![Complex::new(0.0_f32, 0.0); 1024];
    let mut peaks = [SpectralPeak::default(); 64];
    // Output array is only 64 items, but 100 peaks exist
    let count = extract_peaks(&magnitudes, &complex, 44100, 2048, 0.01, &mut peaks);

    assert_eq!(count, 64);
}
