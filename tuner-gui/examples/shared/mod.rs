//! Shared testing utilities for visual examples
use crossbeam_channel::{Receiver, unbounded};
use ringbuf::traits::{Consumer, Observer, Split};
use std::thread;
use tuner_core::{
    AnalysisResult,
    algorithms::{fft, pitch, tuning},
    audio,
};

/// Starts a background audio thread that captures microphone input,
/// processes it through the entire FFT and pitch detection pipeline,
/// and returns a Receiver channel yielding complete `AnalysisResult` frames.
pub fn start_audio_feed() -> Receiver<AnalysisResult> {
    let (tx, rx) = unbounded();

    thread::spawn(move || {
        let rb = ringbuf::HeapRb::<f32>::new(audio::BUFFER_SIZE * 8);
        let (producer, mut consumer) = rb.split();

        // Start CPAL audio capture
        let (_stream, sample_rate) = match audio::start_audio_capture(producer) {
            Ok(tuple) => tuple,
            Err(e) => {
                eprintln!("Failed to start audio feed for test: {}", e);
                return;
            }
        };

        let mut audio_frame = Vec::with_capacity(audio::BUFFER_SIZE);
        let amplitude_threshold = 0.01;

        let mut planner = rustfft::FftPlanner::new();
        let fft_instance = planner.plan_fft_forward(audio::BUFFER_SIZE);
        let mut complex_buffer =
            vec![rustfft::num_complex::Complex { re: 0.0, im: 0.0 }; audio::BUFFER_SIZE];
        let mut time_buffer = vec![0.0; audio::BUFFER_SIZE / 2];

        loop {
            if consumer.occupied_len() >= audio::BUFFER_SIZE {
                audio_frame.clear();
                audio_frame.resize(audio::BUFFER_SIZE, 0.0);
                consumer.pop_slice(&mut audio_frame);

                // Replicate the main app's analysis pipeline
                fft::perform_fft(&audio_frame, &mut complex_buffer, &fft_instance);
                let spectrogram_data = fft::spectrum_to_magnitudes(&complex_buffer);

                let (detected_frequency, confidence) = if let Some((freq, conf)) =
                    pitch::detect_pitch_pyin(
                        &audio_frame,
                        sample_rate,
                        amplitude_threshold,
                        &mut time_buffer,
                    ) {
                    let refined_freq =
                        pitch::refine_from_spectrum(&spectrogram_data, freq, sample_rate);
                    (refined_freq, Some(conf))
                } else {
                    (None, None)
                };

                let (cents_deviation, note_name) = if let Some(freq) = detected_frequency {
                    let (name, target_freq) = tuning::find_nearest_note(freq);
                    let deviation = tuning::calculate_cents_deviation(freq, target_freq);
                    (Some(deviation), Some(name))
                } else {
                    (None, None)
                };

                let partials = if let Some(fundamental) = detected_frequency {
                    pitch::find_partials(&spectrogram_data, fundamental, sample_rate, 7)
                } else {
                    vec![]
                };

                let result = AnalysisResult {
                    detected_frequency,
                    confidence,
                    cents_deviation,
                    note_name,
                    spectrogram_data,
                    partials,
                };

                if tx.send(result).is_err() {
                    break; // Channel closed, exit thread
                }
            } else {
                thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    });

    rx
}
