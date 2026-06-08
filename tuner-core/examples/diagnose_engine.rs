use anyhow::{Context, Result, anyhow};
use realfft::RealFftPlanner;
use rustfft::num_complex::Complex;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use crossbeam_queue::ArrayQueue;
use std::sync::Arc;
use tuner_core::algorithms::peaks::{SpectralPeak, extract_peaks};
use tuner_core::algorithms::spectral::{perform_fft, spectrum_to_magnitudes};
use tuner_core::algorithms::twm::score_candidate;
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE, WINDOW_SIZE};
use tuner_core::engine::KeyProfile;
use tuner_core::gatekeeper::Gatekeeper;
use tuner_core::models::{NOTES, get_expected_beta};
use tuner_core::pipeline::ProcessingFrame;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("Usage: cargo run --example diagnose_engine -- <path_to_audio.raw>");
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
            "Warning: Failed to read 'noise_floor' from analysis.json. Defaulting to 0.001 (-60 dBFS)."
        );
        noise_floor = 0.001;
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
    let sample_rate = 44100;
    let mut planner = RealFftPlanner::<f32>::new();
    let fft_instance = planner.plan_fft_forward(BASS_WINDOW_SIZE);

    let mut time_buffer = vec![0.0f32; BASS_WINDOW_SIZE];
    let mut frequency_buffer = vec![Complex { re: 0.0, im: 0.0 }; BASS_WINDOW_SIZE / 2 + 1];
    let mut magnitude_buffer = vec![0.0f32; BASS_WINDOW_SIZE / 2];

    let mut peak_scratch = vec![SpectralPeak::default(); 128].into_boxed_slice();

    let fft_gatekeeper = planner.plan_fft_forward(WINDOW_SIZE);
    let mut processing_frame = ProcessingFrame::new();
    let audio_pool = Arc::new(ArrayQueue::new(1));
    let mut gatekeeper = Gatekeeper::new(audio_pool);
    gatekeeper.config.silence_threshold = noise_floor;

    // Reconstruct KeyProfiles directly since they are private in Engine
    let mut profiles_vec = Vec::with_capacity(88);
    for i in 0..88 {
        let note = &NOTES[i];
        let beta = get_expected_beta(i as u8);
        profiles_vec.push(KeyProfile::new(note.frequency, beta));
    }

    let profiles_array: [KeyProfile; 88] = profiles_vec.try_into().unwrap();

    // Setup output files
    let path = Path::new(file_path);
    let parent_dir = path.parent().unwrap_or(Path::new(""));
    let mut spectrum_csv = File::create(parent_dir.join("spectrum.csv"))?;
    let mut peaks_csv = File::create(parent_dir.join("peaks.csv"))?;

    writeln!(
        peaks_csv,
        "frame,rms_power,key_idx,key_name,e_win,f0_et,num_peaks,peak_freqs,peak_mags"
    )?;

    // Calculate noise threshold formula just like engine.rs line 189
    let sum_w2 = 0.375 * BASS_WINDOW_SIZE as f32;
    let p_bin = noise_floor * noise_floor * sum_w2;
    let min_magnitude = if p_bin > 0.0 {
        (-p_bin * 0.001_f32.ln()).sqrt()
    } else {
        0.0
    };
    println!(
        "Pre-calculated min_magnitude for peak extraction: {:.3}",
        min_magnitude
    );

    // Loop through frame-by-frame- Sliding Window Loop ---
    let mut frame_idx = 0;
    let mut cursor = 0;

    while cursor + BASS_WINDOW_SIZE <= num_samples {
        let frame_audio = &audio_f32[cursor..cursor + BASS_WINDOW_SIZE];

        let mut sum_sq = 0.0;
        for &s in frame_audio {
            sum_sq += s * s;
        }
        let rms = (sum_sq / BASS_WINDOW_SIZE as f32).sqrt();
        let dbfs = 20.0 * rms.log10();

        perform_fft(
            frame_audio,
            &mut time_buffer,
            &mut frequency_buffer,
            &fft_instance,
            BASS_WINDOW_SIZE,
        );

        spectrum_to_magnitudes(&frequency_buffer, BASS_WINDOW_SIZE, &mut magnitude_buffer);

        // Dump spectrum (first ~5000Hz to save space, ~928 bins)
        let hz_per_bin = sample_rate as f32 / BASS_WINDOW_SIZE as f32;
        let limit_bin = (5000.0 / hz_per_bin) as usize;
        let limit_bin = limit_bin.min(magnitude_buffer.len());

        let row_strs: Vec<String> = magnitude_buffer[..limit_bin]
            .iter()
            .map(|f| format!("{:.3}", f))
            .collect();
        writeln!(spectrum_csv, "{}", row_strs.join(","))?;

        // Run Gatekeeper
        processing_frame.audio_buffer[..BASS_WINDOW_SIZE].copy_from_slice(frame_audio);
        let newest_start = BASS_WINDOW_SIZE - WINDOW_SIZE;
        perform_fft(
            &processing_frame.audio_buffer[newest_start..BASS_WINDOW_SIZE],
            &mut processing_frame.time_buffer[..WINDOW_SIZE],
            &mut processing_frame.frequency_buffer[..],
            &fft_gatekeeper,
            WINDOW_SIZE,
        );
        let gate_result = gatekeeper.process_frame(&processing_frame);

        let count = extract_peaks(
            &magnitude_buffer,
            &frequency_buffer,
            sample_rate,
            BASS_WINDOW_SIZE,
            min_magnitude,
            &mut peak_scratch,
        );

        let k = count.min(64);
        let active_peaks = &mut peak_scratch[..k];

        let valid_count = tuner_core::algorithms::peaks::mask_peaks(active_peaks);
        let active_peaks = &mut active_peaks[..valid_count];

        let winning_key;
        let e_win;
        let profile;
        let note_name;

        if gate_result.is_transient_bypass {
            winning_key = 255;
            e_win = 0.0;
            profile = &profiles_array[0];
            note_name = "BYPASS";
        } else {
            let mut current_errors = [0.0_f32; 88];
            for key in 0..88 {
                current_errors[key] = score_candidate(active_peaks, &profiles_array[key]);
            }

            let raw_winner = current_errors
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .unwrap()
                .0 as u8;
            winning_key = raw_winner;
            println!(
                "TWM Lock: {} ({:.1})",
                &NOTES[raw_winner as usize].name, current_errors[raw_winner as usize]
            );

            e_win = current_errors[winning_key as usize];
            profile = &profiles_array[winning_key as usize];
            note_name = &NOTES[winning_key as usize].name;
        }

        // Print terminal block
        println!(
            "=== Frame {:02} (Samples {} to {}) ===",
            frame_idx,
            cursor,
            cursor + BASS_WINDOW_SIZE
        );
        println!("RMS Power      : {:.5} ({:.1} dBFS)", rms, dbfs);
        println!("Min Magnitude  : {:.3}", min_magnitude);
        println!("Extracted Peaks: {}", count);

        let top_k = valid_count.min(16);
        let mut peak_str = String::new();
        // Print the first 16 peaks (now sorted by frequency)
        for p in &active_peaks[..top_k] {
            peak_str.push_str(&format!("{:.1}Hz (mag: {:.1}), ", p.frequency, p.magnitude));
        }
        println!("Top {} Frequencies: [{}]", top_k, peak_str);

        if gate_result.is_transient_bypass {
            println!("Gatekeeper     : [TRANSIENT BYPASS ACTIVE] -> TWM Discovery Skipped.");
        } else {
            println!(
                "TWM Discovery  : Winner = {} (key_idx {}) with e_win = {:.1}",
                note_name, winning_key, e_win
            );
        }
        println!("--------------------------------------------------");

        // Write to peaks CSV
        let freqs_str: Vec<String> = peak_scratch[..count]
            .iter()
            .map(|p| format!("{:.2}", p.frequency))
            .collect();
        let mags_str: Vec<String> = peak_scratch[..count]
            .iter()
            .map(|p| format!("{:.2}", p.magnitude))
            .collect();
        writeln!(
            peaks_csv,
            "{},{},{},{},{:.2},{:.2},{},\"{}\",\"{}\"",
            frame_idx,
            rms,
            winning_key,
            note_name,
            e_win,
            profile.f0_et,
            count,
            freqs_str.join(";"),
            mags_str.join(";")
        )?;

        cursor += HOP_SIZE;
        frame_idx += 1;
    }

    println!("\nDiagnostics complete.");
    println!("Generated spectrum.csv and peaks.csv in the audio directory.");

    Ok(())
}
