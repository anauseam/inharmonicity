use rustfft::num_complex::Complex;
use crate::audio::SAMPLE_RATE;

/// Evaluates the computationally-cheap Truncated ArgMax power spectrum
/// on a pre-computed RFFT buffer to determine the predominant fundamental
/// frequency neighborhood of the piano audio.
pub fn process_scout(spectrum: &[Complex<f32>]) -> f32 {
    let mut max_power = 0.0_f32;
    let mut max_bin = 0;

    // We only scan up to bin 200 (approx. 4306 Hz), because the top key 
    // on a piano (C8) is 4186 Hz. There is no fundamental piano data above
    // this bin. Bins above this are purely harmonics/noise, so scanning them
    // just wastes CPU cycles.
    let search_limit = spectrum.len().min(200);

    for (k, complex) in spectrum.iter().enumerate().take(search_limit) {
        // Power Spectrum Bypass: Skip the sqrt() from the traditional
        // magnitude calculation, as P[k] = re^2 + im^2 scales identically 
        // for determining the mere index of the peak (ArgMax).
        let power = complex.re * complex.re + complex.im * complex.im;
        if power > max_power {
            max_power = power;
            max_bin = k;
        }
    }

    // N here matches the standard size of the underlying audio buffer (2048)
    let n = (spectrum.len() - 1) * 2; // For 1025 bins, N = 2048
    
    // f_scout = k_max * f_s / N
    let frequency = (max_bin as f32) * (SAMPLE_RATE as f32) / (n as f32);
    
    frequency
}
