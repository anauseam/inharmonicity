use tuner_core::algorithms::pitch::detect_pitch_xqifft_seeded;
use tuner_core::algorithms::spectral::{perform_fft, spectrum_to_magnitudes};
use realfft::RealFftPlanner;
use tuner_core::pipeline::ProcessingFrame;

fn generate_windowed_sine(freq: f32, sample_rate: u32, length: usize) -> Vec<f32> {
    let mut signal = vec![0.0; length];
    for i in 0..length {
        let t = i as f32 / sample_rate as f32;
        signal[i] = (t * freq * 2.0 * std::f32::consts::PI).sin();
        let n_f32 = i as f32;
        let l_f32 = length as f32;
        // apply Hann window to simulate COLA environment
        signal[i] *= 0.5 * (1.0 - (2.0 * std::f32::consts::PI * n_f32 / l_f32).cos());
    }
    signal
}

#[test]
fn test_xqifft_accuracy() {
    let sample_rate = 44100;
    let window_size = 2048;
    
    // 440 Hz is halfway between bins (Bin 20.43). Interpolation needed.
    let true_freq = 440.0;
    let signal = generate_windowed_sine(true_freq, sample_rate, window_size);
    
    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(window_size);
    
    let mut frame = ProcessingFrame::new();
    let mut time_buffer = vec![0.0; window_size];
    
    perform_fft(&signal, &mut time_buffer, &mut frame.frequency_buffer[..1025], &r2c, window_size);
    let magnitudes = spectrum_to_magnitudes(&frame.frequency_buffer[..1025], window_size);
    
    // Pass seed_hz of 435.0 (close enough) with exponential power weighting of p=0.5
    let refined_freq = detect_pitch_xqifft_seeded(&magnitudes, sample_rate, 435.0, 0.5)
        .expect("XQIFFT should successfully detect pitch from seed");
    
    let error_cents = 1200.0 * (refined_freq / true_freq).log2();
    assert!(
        error_cents.abs() < 1.0, 
        "Expected sub-cent accuracy (error < 1 cent), got {:.2} cents (detected freq={:.4})", 
        error_cents, 
        refined_freq
    );
}
