//! GUI-specific Noise Floor Calibration logic and constants.
//!
//! This module holds the lock-free polling logic that calculates the
//! noise floor baseline natively within the GUI tick loop, removing the
//! need to spawn separate CPAL audio streams.



/// The multiplier applied to the absolute maximum RMS value recorded.
pub const DEFAULT_NOISE_MULTIPLIER: f32 = 1.5;

/// Number of buffers to ignore before taking measurements to avoid interface pops.
/// Extended to 45 hops (~1 sec) to outlast the EMA decay of initial interface artifacts.
pub const WARMUP_FRAMES: u32 = 45;

/// The total number of audio hops to measure the noise floor.
/// 120 hops at ~43Hz = ~2.8 seconds.
pub const CALIBRATION_FRAMES: u32 = 120;

/// Processes a single GUI tick during the startup or manual RMS noise calibration.
/// 
/// Updates the countdowns and tracking. Returns `Some(f32)` if the calibration
/// has completed on this tick, yielding the newly calculated silence threshold.
pub fn process_calibration_tick(
    settings: &mut crate::app::NoiseFloorSettings,
    current_rms: f32,
    new_frame_arrived: bool,
) -> Option<f32> {
    // We only progress the state machine if an actual new frame popped out of the DSP.
    if !new_frame_arrived {
        return None;
    }

    let active = match settings.active_calibration.as_mut() {
        Some(a) => a,
        None => return None,
    };

    // If we're still warming up, only decrement if a completely new audio buffer arrived.
    if let Some(mut warmup) = active.warmup_hops {
        if warmup == 0 {
            active.warmup_hops = None;
        } else {
            warmup -= 1;
            active.warmup_hops = Some(warmup);
        }
        return None;
    }

    // Warmup is done. Now measure exact audio hops to guarantee consistent duration across computers.
    if let Some(mut countdown) = active.countdown {
        // Track the max RMS seen so far
        if current_rms > active.max_seen_rms {
            active.max_seen_rms = current_rms;
        }

        if countdown == 0 {
            // Calibration finished!
            active.countdown = None;
            let final_threshold = active.max_seen_rms * DEFAULT_NOISE_MULTIPLIER;
            return Some(final_threshold);
        } else {
            countdown -= 1;
            active.countdown = Some(countdown);
        }
    }

    None
}
