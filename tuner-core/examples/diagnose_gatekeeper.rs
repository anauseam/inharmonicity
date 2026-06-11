use anyhow::{Context, Result, anyhow};
use crossbeam_queue::ArrayQueue;
use realfft::RealFftPlanner;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use tuner_core::algorithms::spectral::perform_fft;
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE, WINDOW_SIZE};
use tuner_core::gatekeeper::{Gatekeeper, SignalState};
use tuner_core::pipeline::ProcessingFrame;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("Usage: cargo run --example diagnose_gatekeeper -- <path_to_audio.raw>");
        return Ok(());
    }

    let file_path = &args[1];

    let path = Path::new(file_path);
    let parent_dir = path.parent().unwrap_or(Path::new(""));
    let json_path = parent_dir.join("analysis.json");

    let mut noise_floor = 0.0;
    if let Ok(json_str) = fs::read_to_string(&json_path)
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(&json_str)
        && let Some(nf) = json["metadata"]["noise_floor"].as_f64()
    {
        noise_floor = nf as f32;
    }

    if noise_floor <= 0.0 {
        println!(
            "Warning: Failed to read 'noise_floor' from analysis.json. Defaulting to 0.005 (-46 dBFS)."
        );
        noise_floor = 0.005;
    }

    println!("Loading file: {}", file_path);
    println!("Using noise floor: {:.6} (from analysis.json)", noise_floor);

    let audio_bytes = fs::read(file_path).context("Failed to read audio.raw")?;
    if audio_bytes.len() % 4 != 0 {
        return Err(anyhow!(
            "File size not divisible by 4, might not be an f32 array"
        ));
    }
    let num_samples = audio_bytes.len() / 4;
    let mut audio_f32 = vec![0.0f32; num_samples];

    unsafe {
        std::ptr::copy_nonoverlapping(
            audio_bytes.as_ptr(),
            audio_f32.as_mut_ptr() as *mut u8,
            audio_bytes.len(),
        );
    }

    println!(
        "Loaded {} samples ({:.2} seconds at 44100Hz)",
        num_samples,
        num_samples as f32 / 44100.0
    );

    if num_samples < BASS_WINDOW_SIZE {
        return Err(anyhow!("File too short for one frame"));
    }

    // --- Setup DSP ---
    let mut planner = RealFftPlanner::<f32>::new();
    let fft_instance = planner.plan_fft_forward(WINDOW_SIZE);

    let mut processing_frame = ProcessingFrame::new();

    // Create a dummy AudioPool for the Gatekeeper
    let audio_pool = Arc::new(ArrayQueue::new(1));
    let mut gatekeeper = Gatekeeper::new(audio_pool);
    // Explicitly set the silence threshold from the JSON
    gatekeeper.config.silence_threshold = noise_floor;
    gatekeeper.capture_mode_enabled = true; // So it can traverse all 5 states

    // Setup output files
    let mut gatekeeper_csv = File::create(parent_dir.join("gatekeeper.csv"))?;
    writeln!(
        gatekeeper_csv,
        "frame_idx,time_ms,rms_ema,nhwrsf,ninos2_ema,ninos2_raw,state_enum,is_new_onset,state_name"
    )?;

    // Loop through frame-by-frame- Sliding Window Loop ---
    let mut frame_idx = 0;
    let mut cursor = 0;

    println!("==================================================");
    println!("Gatekeeper Execution Timeline");
    println!("==================================================");

    while cursor + BASS_WINDOW_SIZE <= num_samples {
        let frame_audio = &audio_f32[cursor..cursor + BASS_WINDOW_SIZE];

        // Copy audio into processing frame
        processing_frame.audio_buffer[..BASS_WINDOW_SIZE].copy_from_slice(frame_audio);

        // Perform WINDOW_SIZE FFT for Gatekeeper
        let newest_start = BASS_WINDOW_SIZE - WINDOW_SIZE;
        perform_fft(
            &processing_frame.audio_buffer[newest_start..BASS_WINDOW_SIZE],
            &mut processing_frame.time_buffer[..WINDOW_SIZE],
            &mut processing_frame.frequency_buffer[..],
            &fft_instance,
            WINDOW_SIZE,
        );

        let gate_result = gatekeeper.process_frame(&processing_frame);

        // The Gatekeeper analyzes the newest WINDOW_SIZE samples of the BASS_WINDOW_SIZE buffer.
        let gatekeeper_cursor = cursor + BASS_WINDOW_SIZE - WINDOW_SIZE;
        let time_ms = (gatekeeper_cursor as f32 / 44100.0) * 1000.0;

        let state_enum = match gate_result.state {
            SignalState::Silence => 0,
            SignalState::Unstable => 1,
            SignalState::Stable => 2,
        };

        let state_name = match gate_result.state {
            SignalState::Silence => "Silence",
            SignalState::Unstable => "Unstable",
            SignalState::Stable => "Stable",
        };

        if gate_result.is_new_onset {
            println!(
                "\n>>> [{:>6.1} ms] ONSET DETECTED (NHWRSF: {:.3}) <<<",
                time_ms, gate_result.nhwrsf
            );
        }

        // Only print interesting frames to terminal to avoid spam
        if gate_result.state != SignalState::Silence || gate_result.is_new_onset {
            println!(
                "Frame {:4} | {:6.1} ms | {:8} | RMS EMA: {:.5} | NINOS2 EMA: {:.1}",
                frame_idx, time_ms, state_name, gate_result.rms_ema, gate_result.ninos2_ema
            );
        }

        writeln!(
            gatekeeper_csv,
            "{},{:.2},{:.5},{:.3},{:.3},{:.3},{},{},{}",
            frame_idx,
            time_ms,
            gate_result.rms_ema,
            gate_result.nhwrsf,
            gate_result.ninos2_ema,
            gate_result.ninos2_raw,
            state_enum,
            gate_result.is_new_onset,
            state_name
        )?;

        cursor += HOP_SIZE;
        frame_idx += 1;
    }

    println!("\nDiagnostics complete.");
    println!("Generated gatekeeper.csv in the audio directory.");

    Ok(())
}
