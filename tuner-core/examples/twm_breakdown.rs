use anyhow::{Context, Result};
use realfft::RealFftPlanner;
use rustfft::num_complex::Complex;
use std::env;
use std::fs::{self};
use std::path::Path;

use tuner_core::algorithms::peaks::extract_peaks;
use tuner_core::algorithms::spectral::{fft, magnitude_spectrum};
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE};
use tuner_core::models::SpectralPeak;
use tuner_core::models::{KeyProfile, NOTES, get_expected_beta};

#[derive(Debug)]
struct TwmBreakdown {
    total: f32,
    err_pm_raw: f32,
    err_pm_norm: f32,
    err_mp_raw: f32,
    err_mp_norm: f32,
}

fn score_detailed(peaks: &[SpectralPeak], profile: &KeyProfile) -> TwmBreakdown {
    let cfg = tuner_core::algorithms::twm::TwmConfig::default();

    let valid_count = profile.valid_partial_count;
    if valid_count == 0 || peaks.is_empty() {
        return TwmBreakdown {
            total: f32::MAX,
            err_pm_raw: 0.0,
            err_pm_norm: 0.0,
            err_mp_raw: 0.0,
            err_mp_norm: 0.0,
        };
    }

    let mut a_max = 0.0_f32;
    let mut max_obs_freq = 0.0_f32;
    for peak in peaks {
        if peak.magnitude > a_max {
            a_max = peak.magnitude;
        }
        if peak.frequency > max_obs_freq {
            max_obs_freq = peak.frequency;
        }
    }
    a_max = a_max.max(1e-6);

    let cutoff_freq = max_obs_freq + profile.f0_et;
    let mut active_predicted = 0_usize;
    for &p_freq in &profile.predicted_partials[..valid_count] {
        if p_freq <= cutoff_freq {
            active_predicted += 1;
        } else {
            break;
        }
    }
    if active_predicted == 0 {
        active_predicted = 1;
    }
    let predicted = &profile.predicted_partials[..active_predicted];

    let mut err_pm = 0.0_f32;
    let mut j = 0;
    for &f_n in predicted {
        while j + 1 < peaks.len()
            && (peaks[j + 1].frequency - f_n).abs() <= (peaks[j].frequency - f_n).abs()
        {
            j += 1;
        }
        let delta_f_n = (peaks[j].frequency - f_n).abs();
        let a_n = peaks[j].magnitude;

        let f_weight = 1.0 / f_n.max(1.0).sqrt();
        let amp_ratio = a_n / a_max;
        let err_pm_n = delta_f_n * f_weight + amp_ratio * (cfg.q * delta_f_n * f_weight - cfg.r);
        err_pm += err_pm_n;
    }

    let mut err_mp = 0.0_f32;
    let mut i = 0;
    for peak in peaks {
        let f_k = peak.frequency;
        let a_k = peak.magnitude;

        while i + 1 < predicted.len()
            && (predicted[i + 1] - f_k).abs() <= (predicted[i] - f_k).abs()
        {
            i += 1;
        }
        let delta_f_k = (predicted[i] - f_k).abs();

        let f_weight = 1.0 / f_k.max(1.0).sqrt();
        let amp_ratio = a_k / a_max;
        err_mp += delta_f_k * f_weight + amp_ratio * (cfg.q * delta_f_k * f_weight - cfg.r);
    }

    let n = active_predicted as f32;
    let k = peaks.len() as f32;

    let err_pm_norm = err_pm / n;
    let err_mp_norm = cfg.rho * (err_mp / k);

    TwmBreakdown {
        total: err_pm_norm + err_mp_norm,
        err_pm_raw: err_pm,
        err_pm_norm,
        err_mp_raw: err_mp,
        err_mp_norm,
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("Usage: cargo run --example twm_breakdown -- <path_to_audio.raw>");
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
        noise_floor = 0.0001;
    } // fallback

    let audio_bytes = fs::read(file_path).context("Failed to read audio.raw")?;
    let num_samples = audio_bytes.len() / 4;
    let mut audio_f32 = vec![0.0f32; num_samples];
    unsafe {
        std::ptr::copy_nonoverlapping(
            audio_bytes.as_ptr(),
            audio_f32.as_mut_ptr() as *mut u8,
            audio_bytes.len(),
        );
    }

    let sample_rate = 44100;
    let mut planner = RealFftPlanner::<f32>::new();
    let fft_instance = planner.plan_fft_forward(BASS_WINDOW_SIZE);

    let mut time_buffer = vec![0.0f32; BASS_WINDOW_SIZE];
    let mut frequency_buffer = vec![Complex { re: 0.0, im: 0.0 }; BASS_WINDOW_SIZE / 2 + 1];
    let mut magnitude_buffer = vec![0.0f32; BASS_WINDOW_SIZE / 2];
    let mut peak_scratch = vec![SpectralPeak::default(); 128].into_boxed_slice();

    let mut profiles_vec = Vec::with_capacity(88);
    for i in 0..88 {
        let note = &NOTES[i];
        let beta = get_expected_beta(i as u8);
        profiles_vec.push(KeyProfile::new(note.frequency, beta));
    }
    let profiles_array: [KeyProfile; 88] = profiles_vec.try_into().unwrap();

    let sum_w2 = 0.375 * BASS_WINDOW_SIZE as f32;
    let p_bin = noise_floor * noise_floor * sum_w2;
    let min_magnitude = if p_bin > 0.0 {
        (-p_bin * 0.001_f32.ln()).sqrt()
    } else {
        0.0
    };

    let mut cursor = 0;
    let mut frame_idx = 0;

    println!(
        "Analyzing {} with min_magnitude={:.4}",
        file_path, min_magnitude
    );

    while cursor + BASS_WINDOW_SIZE <= num_samples {
        let frame_audio = &audio_f32[cursor..cursor + BASS_WINDOW_SIZE];

        let mut sum_sq = 0.0;
        for &s in frame_audio {
            sum_sq += s * s;
        }
        let rms = (sum_sq / BASS_WINDOW_SIZE as f32).sqrt();
        let dbfs = 20.0 * rms.log10();

        // Skip silent frames
        if dbfs < -60.0 {
            cursor += HOP_SIZE;
            frame_idx += 1;
            continue;
        }

        fft(
            frame_audio,
            &mut time_buffer,
            &mut frequency_buffer,
            &fft_instance,
            BASS_WINDOW_SIZE,
        );
        magnitude_spectrum(&frequency_buffer, BASS_WINDOW_SIZE, &mut magnitude_buffer);

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
        active_peaks.sort_unstable_by(|a, b| a.frequency.partial_cmp(&b.frequency).unwrap());

        println!(
            "\n=== Frame {} ({} peaks, {:.1} dBFS) ===",
            frame_idx, k, dbfs
        );

        let a0_idx = 1; // A#0
        let a0_profile = &profiles_array[a0_idx];
        let a0_breakdown = score_detailed(active_peaks, a0_profile);

        let c4_idx = 14; // B1
        let c4_profile = &profiles_array[c4_idx];
        let c4_breakdown = score_detailed(active_peaks, c4_profile);

        println!("A#0 (29.1 Hz) Breakdown:");
        println!("  Total: {:.3}", a0_breakdown.total);
        println!(
            "  Err_pm: {:.3} (raw) / N -> {:.3}",
            a0_breakdown.err_pm_raw, a0_breakdown.err_pm_norm
        );
        println!(
            "  Err_mp: {:.3} (raw) / K * rho -> {:.3}",
            a0_breakdown.err_mp_raw, a0_breakdown.err_mp_norm
        );

        println!("B1 (61.7 Hz) Breakdown:");
        println!("  Total: {:.3}", c4_breakdown.total);
        println!(
            "  Err_pm: {:.3} (raw) / N -> {:.3}",
            c4_breakdown.err_pm_raw, c4_breakdown.err_pm_norm
        );
        println!(
            "  Err_mp: {:.3} (raw) / K * rho -> {:.3}",
            c4_breakdown.err_mp_raw, c4_breakdown.err_mp_norm
        );

        cursor += HOP_SIZE;
        frame_idx += 1;
    }
    Ok(())
}
