#[cfg(test)]
mod tests {
    use tuner_core::gatekeeper::{Gatekeeper, SignalState};
    use tuner_core::pipeline::{AudioPool, ProcessingFrame};
    use rustfft::num_complex::Complex;
    use realfft::RealFftPlanner;
    use std::sync::Arc;
    use crossbeam_queue::ArrayQueue;

    pub fn generate_sine_wave(freq: f32, sample_rate: u32, length: usize) -> Vec<f32> {
        (0..length).map(|i| {
            let t = i as f32 / sample_rate as f32;
            (t * freq * 2.0 * std::f32::consts::PI).sin()
        }).collect()
    }

    #[test]
    fn test_treble_gatekeeper() {
        let sample_rate = 44100;
        let pool = Arc::new(ArrayQueue::new(4));
        
        for freq in [2093.0, 4186.0] { // C7, C8
            let mut gatekeeper = Gatekeeper::new(pool.clone());
            let audio = generate_sine_wave(freq, sample_rate, 4096);
            
            let mut planner = RealFftPlanner::<f32>::new();
            let r2c = planner.plan_fft_forward(2048);

            // Simulate 5 frames of overlapping COLA
            for i in 0..5 {
                let start_idx = i * 1024;
                let end_idx = start_idx + 2048;
                let frame_audio = &audio[start_idx..end_idx];
                
                let mut time_buf = vec![0.0_f32; 2048];
                for j in 0..2048 {
                    let window = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * j as f32 / 2047.0).cos());
                    time_buf[j] = frame_audio[j] * window;
                }

                let mut freq_buf = vec![Complex { re: 0.0, im: 0.0 }; 1025];
                r2c.process(&mut time_buf, &mut freq_buf).unwrap();

                let mut audio_buf = vec![0.0_f32; 8192];
                audio_buf[..2048].copy_from_slice(frame_audio);

                let frame = ProcessingFrame {
                    audio_buffer: audio_buf.into_boxed_slice(),
                    time_buffer: vec![0.0_f32; 8192].into_boxed_slice(),
                    frequency_buffer: freq_buf.into_boxed_slice(),
                };

                let prev_state = gatekeeper.current_state.clone();
                gatekeeper.process_frame(&frame);
                let nin2 = tuner_core::algorithms::metrics::calculate_ninos2(&frame.frequency_buffer[..]);
                eprintln!(
                    "[TREBLE GATEKEEPER] Freq: {} | Frame {} | RMS: {:.5} | NHWRSF: {:.5} | NINOS2: {:.2} | State: {:?} -> {:?}",
                    freq, i, gatekeeper.current_rms_ema, gatekeeper.current_nhwrsf, nin2, prev_state, gatekeeper.current_state
                );
            }
        }
    }
}
