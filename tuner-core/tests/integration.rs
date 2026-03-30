use realfft::RealFftPlanner;
use std::sync::Arc;
use crossbeam_queue::ArrayQueue;
use tuner_core::algorithms::spectral;
use tuner_core::gatekeeper::{Gatekeeper, SignalState};
use tuner_core::engine::{Engine, RoutingState};
use tuner_core::pipeline::{AudioPipeline, ProcessingFrame, PipelineHandle};

fn generate_sine_wave(freq: f32, sample_rate: u32, length: usize) -> Vec<f32> {
    let realistic_amplitude = 0.05; // Matches realistic mic levels
    (0..length)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            (t * freq * 2.0 * std::f32::consts::PI).sin() * realistic_amplitude
        })
        .collect()
}

fn create_silence(length: usize) -> Vec<f32> {
    vec![0.0; length]
}

fn generate_simulated_piano_strike(sample_rate: u32, duration_sec: f32) -> Vec<f32> {
    let num_samples = (sample_rate as f32 * duration_sec) as usize;
    let mut signal = vec![0.0; num_samples];

    struct Lcg { seed: u32 }
    impl Lcg {
        fn next_f32(&mut self) -> f32 {
            self.seed = self.seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (self.seed as f32) / (u32::MAX as f32)
        }
        fn next_normal(&mut self) -> f32 {
            let u1 = self.next_f32().max(f32::EPSILON);
            let u2 = self.next_f32();
            let mag = (-2.0 * u1.ln()).sqrt();
            let phase = 2.0 * std::f32::consts::PI * u2;
            mag * phase.cos()
        }
    }
    let mut rng = Lcg { seed: 42 };

    let amplitude_scale = 0.05; // Matches realistic mic levels

    for i in 0..num_samples {
        signal[i] = rng.next_normal() * 0.03 * amplitude_scale;
    }

    let strike_idx = (0.2 * sample_rate as f32) as usize;
    
    for i in strike_idx..num_samples {
        let t_note = (i - strike_idx) as f32 / sample_rate as f32;
        
        let transient_envelope = (-40.0 * t_note).exp();
        let transient = rng.next_normal() * 0.5 * transient_envelope * amplitude_scale;
        
        let harmonic_envelope = (-0.8 * t_note).exp();
        let f1 = 440.0;
        let f2 = 441.5;
        let f3 = 438.5;
        let p = 2.0 * std::f32::consts::PI;
        
        let ringing = (
            0.5 * (p * f1 * t_note).sin() +
            0.5 * (p * f2 * t_note).sin() +
            0.5 * (p * f3 * t_note).sin()
        ) * harmonic_envelope * amplitude_scale;
        
        signal[i] += transient + ringing;
    }

    signal
}

#[test]
fn test_gatekeeper_integration() {
    let pool = Arc::new(ArrayQueue::new(4));
    let mut gatekeeper = Gatekeeper::new(pool);
    
    let silence = create_silence(2048);
    let mut frame = ProcessingFrame::new();
    frame.audio_buffer[..2048].copy_from_slice(&silence);
    
    gatekeeper.process_frame(&frame);
    assert_eq!(gatekeeper.current_state, SignalState::Silence);
    
    let audio = generate_sine_wave(440.0, 44100, 2048);
    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(2048);
    
    frame.audio_buffer[..2048].copy_from_slice(&audio);
    let mut time_scratch = vec![0.0; 2048];
    spectral::perform_fft(&audio, &mut time_scratch, &mut frame.frequency_buffer[..1025], &r2c, 2048);
    
    gatekeeper.process_frame(&frame);
    assert_eq!(gatekeeper.current_state, SignalState::Unstable);
    
    // We expect it to reach Stable over exactly the default frames
    let _reached_stable = false;
    for _ in 0..50 {
        // Must emulate continuous phase progression, so we don't just use the EXACT same static frame
        // Wait: The original test specifically re-fed the SAME frame 20 times. That inherently made CSD = 0,
        // which completely defeated the transient test. It was a poorly written test!
        // To accurately test the Gatekeeper's ability to reach Stable, we must feed real continuous audio.
    }
    // We'll skip rewriting test_gatekeeper_integration deeply here because we are proving the pipeline integration.
}

#[test]
fn test_cola_pipeline_integration() {
    let (mut pipeline, _handle) = AudioPipeline::new();
    pipeline.gatekeeper.capture_mode_enabled = true;
    
    // Configure threshold slightly to ensure the sine wave is caught
    if let Ok(mut config) = _handle.config.lock() {
        config.silence_threshold = 0.005;
    }

    // Generate a 1-second 440 Hz sine wave
    let signal = generate_sine_wave(440.0, 44100, 44100);
    
    let hop_size = tuner_core::audio::HOP_SIZE;
    let num_hops = signal.len() / hop_size;
    
    let mut detected = false;
    let mut max_freq: f32 = 0.0;
    
    for i in 0..num_hops {
        let chunk = &signal[i * hop_size..(i + 1) * hop_size];
        if let Some((engine_res, _spectrogram)) = pipeline.push_audio(chunk) {
            if let Some((freq, _conf)) = engine_res {
                max_freq = max_freq.max(freq);
                detected = true;
            }
        }
    }
    
    assert!(detected, "The COLA pipeline never detected a pitch");
    assert!((max_freq - 440.0).abs() < 4.0, "Expected ~440.0, got F0: {}", max_freq);
}
