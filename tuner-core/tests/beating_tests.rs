#[cfg(test)]
mod tests {
    use tuner_core::algorithms::metrics::calculate_nhwrsf;
    use rustfft::num_complex::Complex;
    use realfft::RealFftPlanner;

    #[test]
    fn test_nhwrsf_beating_stability() {
        const SAMPLE_RATE: f32 = 44_100.0;
        const BUFFER_SIZE: usize = 2048;
        const THRESHOLD: f32 = 0.5; // Current default nhwrsf_threshold in GatekeeperConfig

        // Simulate two close partials beating (D3 + a slightly detuned copy, ~2 Hz beat)
        let freq1 = 146.83_f32; // D3
        let freq2 = 148.83_f32; // 2 Hz above

        let mut planner = RealFftPlanner::<f32>::new();
        let r2c = planner.plan_fft_forward(BUFFER_SIZE);

        let mut time_buf = vec![0.0_f32; BUFFER_SIZE];
        let mut freq_buf = vec![Complex { re: 0.0, im: 0.0 }; BUFFER_SIZE / 2 + 1];

        // Warm-up: first frame establishes prev_mags baseline
        let frame0: Vec<f32> = (0..BUFFER_SIZE)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE;
                (2.0 * std::f32::consts::PI * freq1 * t).sin()
                    + (2.0 * std::f32::consts::PI * freq2 * t).sin()
            })
            .collect();
        time_buf.copy_from_slice(&frame0);
        r2c.process(&mut time_buf, &mut freq_buf).unwrap();
        let mut prev_mags: Vec<f32> = freq_buf.iter().map(|c| c.norm()).collect();

        // Simulate 200 consecutive frames (~9.3s of audio at 2048-sample hop)
        // covering multiple full beat cycles. Record peak flux.
        // Wait, COLA hop is 1024 NOT 2048! Let's change the hop to 1024 to accurately simulate!
        let hop_size = 1024;
        let num_frames = 200;
        let mut peak_flux: f32 = 0.0;
        let mut frames_above_threshold = 0;

        for frame_idx in 1..=num_frames {
            let t_offset = frame_idx * hop_size;
            let frame: Vec<f32> = (0..BUFFER_SIZE)
                .map(|i| {
                    let t = (t_offset + i) as f32 / SAMPLE_RATE;
                    // Apply Hann window just like the real pipeline!
                    let window = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (BUFFER_SIZE - 1) as f32).cos());
                    window * ((2.0 * std::f32::consts::PI * freq1 * t).sin()
                        + (2.0 * std::f32::consts::PI * freq2 * t).sin())
                })
                .collect();

            time_buf.copy_from_slice(&frame);
            r2c.process(&mut time_buf, &mut freq_buf).unwrap();
            let flux = calculate_nhwrsf(&freq_buf, &mut prev_mags);

            if flux > peak_flux {
                peak_flux = flux;
            }
            if flux > THRESHOLD {
                frames_above_threshold += 1;
                eprintln!(
                    "[FRAME {}] NHWRSF {:.4} > threshold {:.4} — would trigger false onset!",
                    frame_idx, flux, THRESHOLD
                );
            }
        }

        eprintln!(
            "Peak NHWRSF over {} frames: {:.4}. Frames above threshold: {}/{}",
            num_frames, peak_flux, frames_above_threshold, num_frames
        );

        assert_eq!(
            frames_above_threshold, 0,
            "NHWRSF false-triggered on beating bass note {} times out of {} frames (peak: {:.4})",
            frames_above_threshold, num_frames, peak_flux
        );
    }
}
