#[cfg(test)]
mod tests {
    use rustfft::num_complex::Complex;
    use realfft::RealFftPlanner;

    #[test]
    fn test_scout_bass_energy_ratio() {
        let sample_rate = 44100;
        let mut planner = RealFftPlanner::<f32>::new();
        let r2c = planner.plan_fft_forward(2048);

        // Simulate A1 (55 Hz) with 5% fundamental and 95% harmonics
        let audio: Vec<f32> = (0..2048).map(|i| {
            let t = i as f32 / sample_rate as f32;
            let window = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / 2047.0).cos());
            
            let f0 = 55.0;
            let mut sample = 0.05 * (2.0 * std::f32::consts::PI * f0 * t).sin();
            // Add strong harmonics
            for h in 2..15 {
                let freq = f0 * h as f32;
                sample += 0.1 * (2.0 * std::f32::consts::PI * freq * t).sin();
            }
            sample * window
        }).collect();

        let mut time_buf = audio.clone();
        let mut freq_buf = vec![Complex { re: 0.0, im: 0.0 }; 1025];
        r2c.process(&mut time_buf, &mut freq_buf).unwrap();

        let ratio = tuner_core::algorithms::metrics::evaluate_band_energy_ratio(&freq_buf);
        eprintln!("[SCOUT TEST] A1 Energy Ratio: {:.4}", ratio);
        
        // Let's also simulate C8 (4186 Hz) with a heavy hammer thud (low freq noise)
        let audio_c8: Vec<f32> = (0..2048).map(|i| {
            let t = i as f32 / sample_rate as f32;
            let window = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / 2047.0).cos());
            
            let f0 = 4186.0;
            let string_sound = 0.1 * (2.0 * std::f32::consts::PI * f0 * t).sin();
            // Thud modeled as an exponential decay at 150 Hz
            let thud_env = (-10.0 * t).exp();
            let thud_sound = thud_env * (2.0 * std::f32::consts::PI * 150.0 * t).sin();
            
            (string_sound + thud_sound) * window
        }).collect();

        let mut time_buf = audio_c8.clone();
        let mut freq_buf_2 = vec![Complex { re: 0.0, im: 0.0 }; 1025];
        r2c.process(&mut time_buf, &mut freq_buf_2).unwrap();

        // Test Spectral Centroid
        let mut centroid_num = 0.0;
        let mut centroid_den = 0.0;
        let mut centroid_num2 = 0.0;
        let mut centroid_den2 = 0.0;

        for (k, complex) in freq_buf.iter().enumerate().take(500).skip(1) {
            let mag = (complex.re * complex.re + complex.im * complex.im).sqrt();
            let freq = k as f32 * 21.533;
            centroid_num += mag * freq;
            centroid_den += mag;
        }
        for (k, complex) in freq_buf_2.iter().enumerate().take(500).skip(1) {
            let mag = (complex.re * complex.re + complex.im * complex.im).sqrt();
            let freq = k as f32 * 21.533;
            centroid_num2 += mag * freq;
            centroid_den2 += mag;
        }

        eprintln!("[CENTROID SCOUT] A1 Centroid: {:.4} Hz", centroid_num / centroid_den);
        eprintln!("[CENTROID SCOUT] C8 + Thud Centroid: {:.4} Hz", centroid_num2 / centroid_den2);
    }

    #[test]
    fn test_mid_band_energy_ratio() {
        let sample_rate = 44100;
        let mut planner = RealFftPlanner::<f32>::new();
        let r2c = planner.plan_fft_forward(2048);

        // A1 (55 Hz)
        let audio_bass: Vec<f32> = (0..2048).map(|i| {
            let t = i as f32 / sample_rate as f32;
            let window = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / 2047.0).cos());
            let mut sample = 0.05 * (2.0 * std::f32::consts::PI * 55.0 * t).sin();
            for h in 2..15 {
                sample += 0.1 * (2.0 * std::f32::consts::PI * (55.0 * h as f32) * t).sin();
            }
            sample * window
        }).collect();

        // C8 (4186 Hz + 150 Hz thump)
        let audio_treble: Vec<f32> = (0..2048).map(|i| {
            let t = i as f32 / sample_rate as f32;
            let window = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / 2047.0).cos());
            let string = 0.1 * (2.0 * std::f32::consts::PI * 4186.0 * t).sin();
            let thump = (-10.0 * t).exp() * 1.0 * (2.0 * std::f32::consts::PI * 150.0 * t).sin();
            (string + thump) * window
        }).collect();

        for (name, audio) in [("A1 Bass", audio_bass), ("C8 Treble", audio_treble)] {
            let mut time_buf = audio.clone();
            let mut freq_buf = vec![Complex { re: 0.0, im: 0.0 }; 1025];
            r2c.process(&mut time_buf, &mut freq_buf).unwrap();

            // Ignore thud band! Start evaluating energy at bin 15 (323 Hz)
            // Low band: 323 Hz to 861 Hz (bins 15 to 40)
            // High band: 861 Hz to 4300 Hz (bins 41 to 200)
            let mut mid_band_energy = 0.0;
            let mut total_eval_energy = 0.0;

            for (k, complex) in freq_buf.iter().enumerate().take(200).skip(15) {
                let power = complex.re * complex.re + complex.im * complex.im;
                if k <= 40 {
                    mid_band_energy += power;
                }
                total_eval_energy += power;
            }

            let ratio = if total_eval_energy > f32::EPSILON {
                mid_band_energy / total_eval_energy
            } else {
                0.0
            };

            eprintln!("[MID-BAND SCOUT] {} | Ratio: {:.4} (Mid: {:.2}, Total: {:.2})", name, ratio, mid_band_energy, total_eval_energy);
        }
    }
}
