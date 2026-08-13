// tuner-core/src/tests/audio_tests.rs
use crate::algorithms::spectral;
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

    spectral::fft(
        &audio,
        &mut time_buffer,
        &mut freq_buffer,
        &r2c,
        buffer_size,
    );
    spectral::magnitude_spectrum(&freq_buffer, buffer_size, &mut magnitudes);

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
fn test_cspe_super_resolution() {
    // A deliberately off-bin sinusoid: bin resolution is 44100/8192 ≈ 5.38 Hz, so this tone
    // sits ~58% of a bin above bin 80. CSPE must recover it far better than the DFT grid.
    let n = 8192usize;
    let sample_rate = 44100u32;
    let bin_hz = sample_rate as f32 / n as f32;
    let target_freq = 80.583 * bin_hz; // ≈ 433.7 Hz, not a bin centre

    // One extra sample for the one-sample-shifted frame.
    let audio = generate_sine_wave(target_freq, sample_rate, n + 1);

    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(n);
    let mut time_buffer = vec![0.0; n];
    let mut x0 = vec![Complex { re: 0.0, im: 0.0 }; n / 2 + 1];
    let mut x1 = vec![Complex { re: 0.0, im: 0.0 }; n / 2 + 1];
    let mut magnitudes = vec![0.0f32; n / 2];
    let mut cspe = vec![0.0f32; n / 2];

    spectral::fft(&audio[..n], &mut time_buffer, &mut x0, &r2c, n);
    spectral::fft(&audio[1..n + 1], &mut time_buffer, &mut x1, &r2c, n);
    spectral::magnitude_spectrum(&x0, n, &mut magnitudes);
    spectral::cspe(&x0, &x1, n, sample_rate, &mut cspe);

    // Peak bin of the DFT, then read the CSPE-refined frequency there.
    let peak_bin = magnitudes
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .map(|(i, _)| i)
        .unwrap();

    let dft_freq = peak_bin as f32 * bin_hz;
    let cspe_freq = cspe[peak_bin];

    // The bare DFT bin centre is off by up to half a bin; CSPE should be sub-0.1 Hz.
    assert!(
        (dft_freq - target_freq).abs() > 1.0,
        "sanity: the off-bin tone should not sit on a bin centre (dft {dft_freq}, true {target_freq})"
    );
    assert!(
        (cspe_freq - target_freq).abs() < 0.1,
        "CSPE freq {cspe_freq} not within 0.1 Hz of true {target_freq}"
    );
}

#[test]
fn test_jacobsen_bias() {
    // Candan 2015 Eq. 1 + Eq. 12 regression: the corrected estimator must be
    // essentially exact across the fractional-offset range, at both pipeline FFT
    // sizes. The pre-audit implementation erred by ≈ −2.5·δ bins — worse than the
    // bin centre (faithfulness-audit-03); this pins the faithful behavior.
    let sample_rate = 44100u32;
    for &n in &[2048usize, 8192] {
        let bin_hz = sample_rate as f32 / n as f32;
        let mut planner = RealFftPlanner::<f32>::new();
        let r2c = planner.plan_fft_forward(n);
        let mut time_buffer = vec![0.0; n];
        let mut spectrum = vec![Complex { re: 0.0, im: 0.0 }; n / 2 + 1];

        let m = n / 16; // in-band peak bin, far from DC and Nyquist
        for &delta in &[-0.45f32, -0.3, -0.15, -0.05, 0.05, 0.15, 0.3, 0.45] {
            let target_freq = (m as f32 + delta) * bin_hz;
            let audio = generate_sine_wave(target_freq, sample_rate, n);
            spectral::fft(&audio[..n], &mut time_buffer, &mut spectrum, &r2c, n);

            let est = spectral::jacobsen(&spectrum, m, n, sample_rate);
            let err_bins = (est - target_freq) / bin_hz;
            assert!(
                err_bins.abs() < 1e-3,
                "N={n} δ={delta}: jacobsen error {err_bins} bins (est {est} Hz, true {target_freq} Hz)"
            );
        }
    }
}

/// The tabulated `c_N` values `jacobsen` uses on the hot path must be what
/// Candan Eq. 12 actually evaluates to, and the short lengths the unison ring
/// transforms at must be nowhere near the 2.0 asymptote the table falls back to
/// — 2.4 % of scale at 56 points, applied to every reported line offset.
#[test]
fn candan_c_n_reproduces_the_jacobsen_table() {
    assert!((spectral::candan_c_n(2048) - 2.001_329).abs() < 1e-6);
    assert!((spectral::candan_c_n(8192) - 2.000_332).abs() < 1e-6);

    // The unison ring's own range (`strobe::unison`), against the fallback.
    for &(n, want) in &[(25usize, 2.116_2f32), (56, 2.050_2), (64, 2.043_7)] {
        let got = spectral::candan_c_n(n);
        assert!(
            (got - want).abs() < 1e-3,
            "c_N({n}) = {got}, expected {want}"
        );
        assert!(
            got - 2.0 > 0.02,
            "c_N({n}) = {got} is within the asymptote's rounding — the table's \
             2.0 fallback would be harmless and this test would not be needed"
        );
    }

    // Monotone toward the asymptote: the correction is a finite-N effect.
    assert!(spectral::candan_c_n(56) > spectral::candan_c_n(128));
    assert!(spectral::candan_c_n(128) > spectral::candan_c_n(8192));
}

/// `find_supported_config` must only ever return a range that **contains** the
/// target rate. The pipeline's buffer sizes and timing constants are
/// dimensioned for `SAMPLE_RATE`, so a merely-nearby range is not usable — and
/// cpal's `with_sample_rate` panics when handed one.
#[cfg(test)]
mod find_supported_config {
    use crate::audio::find_supported_config;
    use cpal::{SampleFormat, SupportedBufferSize, SupportedStreamConfigRange};

    /// Builds a config range; `buffer_size` is irrelevant to the selection.
    fn range(
        channels: u16,
        min_rate: u32,
        max_rate: u32,
        format: SampleFormat,
    ) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(
            channels,
            min_rate,
            max_rate,
            SupportedBufferSize::Range { min: 64, max: 8192 },
            format,
        )
    }

    const TARGET: u32 = 44_100;

    #[test]
    fn accepts_a_range_covering_the_target() {
        let found = find_supported_config(vec![range(1, 8_000, 96_000, SampleFormat::F32)], TARGET);
        assert!(
            found.is_some(),
            "a covering mono f32 range must be accepted"
        );
    }

    #[test]
    fn rejects_a_range_that_excludes_the_target() {
        // The panic case: nearest-by-distance would have returned this range,
        // and `with_sample_rate(44100)` on it panics.
        let found =
            find_supported_config(vec![range(1, 48_000, 48_000, SampleFormat::F32)], TARGET);
        assert!(
            found.is_none(),
            "48 kHz-only device must yield None, not a panicking config"
        );
    }

    #[test]
    fn rejects_stereo_and_non_f32() {
        let configs = vec![
            range(2, 8_000, 96_000, SampleFormat::F32), // stereo: one DcBlocker state
            range(1, 8_000, 96_000, SampleFormat::I16), // wrong sample format
        ];
        assert!(find_supported_config(configs, TARGET).is_none());
    }

    #[test]
    fn picks_the_covering_range_over_a_closer_but_excluding_one() {
        let configs = vec![
            range(1, 44_000, 44_050, SampleFormat::F32), // closer bounds, excludes target
            range(1, 8_000, 96_000, SampleFormat::F32),  // wider, covers target
        ];
        let found = find_supported_config(configs, TARGET).expect("covering range exists");
        assert!(found.min_sample_rate() <= TARGET && TARGET <= found.max_sample_rate());
    }
}
