//! Raw `f32` capture dumps.
//!
//! A capture writes headerless little-endian mono `f32`: `audio.raw` (the
//! strictly causal buffer the Worker analysed) and `audio_full_event.raw` (the
//! non-causal record — pre-roll, strike, decay). See
//! `capture-sets.md`.

use std::path::Path;

/// Reads a headerless little-endian `f32` mono dump. `None` if the file is
/// missing, empty, or not a whole number of samples.
pub fn read(path: &Path) -> Option<Vec<f32>> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() || bytes.len() % 4 != 0 {
        return None;
    }
    let mut out = vec![0.0f32; bytes.len() / 4];
    // SAFETY: `out` holds exactly `bytes.len()` bytes of `f32`, and `f32` has
    // no invalid bit patterns — every 4-byte group is a valid value.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.as_mut_ptr() as *mut u8, bytes.len());
    }
    Some(out)
}

/// The capture's analysed audio: `audio.raw`.
pub fn stable(dir: &Path) -> Option<Vec<f32>> {
    read(&dir.join("audio.raw"))
}

/// The capture's full event: `audio_full_event.raw`. Needed wherever the
/// measurement starts before the gatekeeper's verdict, since `audio.raw`
/// begins at that verdict.
pub fn full_event(dir: &Path) -> Option<Vec<f32>> {
    read(&dir.join("audio_full_event.raw"))
}
