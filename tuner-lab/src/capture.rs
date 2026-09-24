//! Finding captures, and naming the key one belongs to.
//!
//! One discovery path for every subcommand. A capture set is a directory of
//! `key_<index>_<note>_<timestamp>/` subdirectories, either at the root
//! (`diagnostics_piano2/key_*`) or one level down under an instrument id
//! (`<dump root>/<identity.id>/key_*`, which is where the app writes). Both
//! shapes are searched, so a set moves between them without a flag.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Every capture directory under `root`, sorted, from either layout.
pub fn find(root: &Path) -> Result<Vec<PathBuf>> {
    let is_capture = |p: &Path| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("key_"))
    };
    let mut out = Vec::new();
    let rd = fs::read_dir(root).with_context(|| format!("read dir {}", root.display()))?;
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        if is_capture(&p) {
            out.push(p);
        } else {
            for sub in fs::read_dir(&p).into_iter().flatten().flatten() {
                let p2 = sub.path();
                if p2.is_dir() && is_capture(&p2) {
                    out.push(p2);
                }
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The key a capture is of, read from its `analysis.json`.
///
/// The authority for key identity: a directory can be renamed, and the note
/// name in it is derived, not recorded.
pub fn key_of(dir: &Path) -> Option<u8> {
    let text = fs::read_to_string(dir.join("analysis.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json["key_index"].as_u64().map(|k| k as u8)
}

/// `key_034_G3_1752264903` → 34.
///
/// The fallback for a capture whose `analysis.json` is absent or unreadable —
/// a raw-audio-only dump. Prefer [`key_of`].
pub fn key_from_dirname(name: &str) -> Option<u8> {
    let rest = name.strip_prefix("key_")?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Register label on the four-band split the unison and strobe summaries
/// report against (`strobe replay`, `strobe isolation`).
pub fn strobe_register(key: u8) -> &'static str {
    match key {
        0..=27 => "bass",
        28..=51 => "tenor",
        52..=75 => "treble",
        _ => "high 76–87",
    }
}

/// Register label on the three-band split the curve tooling reports
/// against: bass = A0–C#3, the wound-string region; treble = C6 up, where
/// partial counts thin. Same idea as [`strobe_register`], different
/// boundaries — a figure read against curve-side tables uses this one.
pub fn curve_register(key: u8) -> &'static str {
    match key {
        0..=27 => "bass",
        28..=62 => "mid",
        _ => "treble",
    }
}

/// The three bands [`curve_register`] can return, in compass order.
pub const CURVE_REGISTERS: [&str; 3] = ["bass", "mid", "treble"];

/// Captures under `root`, or `root` itself when it is one capture (it holds
/// an `audio.raw`). Membership is the presence of that file rather than the
/// `key_` prefix, so a single-capture path can be passed directly.
pub fn find_or_single(root: &Path) -> Vec<PathBuf> {
    if root.join("audio.raw").exists() {
        return vec![root.to_path_buf()];
    }
    let mut v: Vec<PathBuf> = fs::read_dir(root)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_dir() && p.join("audio.raw").exists())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// Keeps only captures of the listed keys.
pub fn retain_keys(caps: &mut Vec<PathBuf>, keys: &[u8]) {
    caps.retain(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .and_then(key_from_dirname)
            .is_some_and(|k| keys.contains(&k))
    });
}
