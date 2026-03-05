//! Shared testing utilities for visual examples
use crossbeam_channel::{Receiver, unbounded};
use ringbuf::traits::{Consumer, Observer, Split};
use std::thread;
use tuner_core::{AnalysisResult, audio, fft, pitch, tuning};

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

        loop {
            if consumer.occupied_len() >= audio::BUFFER_SIZE {
                audio_frame.clear();
                audio_frame.resize(audio::BUFFER_SIZE, 0.0);
                consumer.pop_slice(&mut audio_frame);

                // Replicate the main app's analysis pipeline
                let complex_spectrum = fft::perform_fft(&audio_frame);
                let spectrogram_data = fft::spectrum_to_magnitudes(&complex_spectrum);

                let (detected_frequency, confidence) = if let Some((freq, conf)) =
                    pitch::detect_pitch_pyin(&audio_frame, sample_rate, amplitude_threshold)
                {
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
