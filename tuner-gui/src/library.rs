//! # Profile library — where instruments live on disk
//!
//! The frontend's file-location policy: the directories profiles, app settings
//! and capture dumps live in, the listing the browser renders, and the one-time
//! import of a pre-library profile.
//!
//! Why these locations, and why identity is part of the answer rather than
//! decoration, is argued in
//! `docs/design/session-persistence-and-profile-library.md` §3.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tuner_core::models::{self, InharmonicityProfile};

/// Subdirectory of the data dir holding profile documents.
const PROFILES_SUBDIR: &str = "profiles";

/// Extension of a profile document.
const PROFILE_EXTENSION: &str = "json";

/// Filename of the app-settings document inside the config dir.
const SETTINGS_FILE: &str = "settings.json";

/// Recent profiles retained in [`AppSettings::recents`].
const MAX_RECENTS: usize = 12;

/// Per-user directories for this app.
///
/// `directories` rather than a hand-rolled XDG lookup because the XDG variables
/// are unset on macOS and absent on Windows, where a hand-rolled fallback would
/// pick a non-native path or none at all; pre-built binaries are a planned
/// deliverable (TODO.md). On Linux this resolves to the XDG values as expected.
fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("org", "anauseam", "inharmonicity")
}

/// Directory holding profile documents, created if absent.
///
/// Falls back to `./profiles` when no home directory can be resolved — a
/// headless or sandboxed environment should still be able to save.
pub(crate) fn profiles_dir() -> PathBuf {
    let dir = project_dirs()
        .map(|d| d.data_dir().join(PROFILES_SUBDIR))
        .unwrap_or_else(|| PathBuf::from(PROFILES_SUBDIR));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Directory capture dumps are written under, created if absent.
///
/// `data_local_dir` rather than `data_dir` because dumps are large — raw audio
/// per capture — and on Windows `data_dir` is the *roaming* profile, which a
/// domain login would sync across the network. On Linux and macOS the two
/// resolve to the same path, so the choice costs nothing there.
///
/// Not the working directory: a released binary has no useful CWD (a macOS
/// `.app` launched from Finder gets `/`, a Windows exe gets its install
/// directory), so dumps would land somewhere unwritable or unfindable.
/// The repo's capture sets are a separate thing and stay where they are —
/// `docs/internals/06-capture-sets.md`.
pub(crate) fn diagnostics_dir() -> PathBuf {
    let dir = project_dirs()
        .map(|d| d.data_local_dir().join("diagnostics"))
        .unwrap_or_else(|| PathBuf::from("diagnostics"));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Path of the app-settings document.
fn settings_path() -> PathBuf {
    project_dirs()
        .map(|d| d.config_dir().join(SETTINGS_FILE))
        .unwrap_or_else(|| PathBuf::from(SETTINGS_FILE))
}

/// App-level state that is **not** a property of any instrument.
///
/// Deliberately small. Everything describing an instrument — its thresholds,
/// its engine, its reference mode — lives in the profile so it travels with the
/// instrument between machines; what is left here is the pointer to which
/// profile to reopen, which is by definition not a property of any one of them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppSettings {
    /// The profile to reopen at launch. `None` on a first run.
    #[serde(default)]
    pub last_profile: Option<PathBuf>,
    /// Recently opened profiles, most recent first.
    #[serde(default)]
    pub recents: Vec<PathBuf>,
}

impl AppSettings {
    /// Loads the settings document, or defaults if it is absent or unreadable.
    /// A corrupt settings file must never block startup — the worst it can cost
    /// is the resume-last pointer, which the browser can restore in one click.
    pub fn load() -> Self {
        std::fs::read_to_string(settings_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Writes the settings document atomically (temp file + rename), creating
    /// the config directory if needed.
    pub fn save(&self) -> std::io::Result<()> {
        let path = settings_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)
    }

    /// Records `path` as the current profile and promotes it to the head of the
    /// recents list.
    pub fn note_opened(&mut self, path: &Path) {
        self.last_profile = Some(path.to_path_buf());
        self.recents.retain(|p| p != path);
        self.recents.insert(0, path.to_path_buf());
        self.recents.truncate(MAX_RECENTS);
    }

    /// Drops `path` from the recents list and from the resume pointer.
    pub fn note_removed(&mut self, path: &Path) {
        self.recents.retain(|p| p != path);
        if self.last_profile.as_deref() == Some(path) {
            self.last_profile = None;
        }
    }
}

/// One row of the profile browser: enough to identify and sort an instrument
/// without opening it in full.
#[derive(Debug, Clone)]
pub struct ProfileEntry {
    /// Path of the document.
    pub path: PathBuf,
    /// Display name from the profile's identity.
    pub name: String,
    /// Manufacturer, if recorded — one of the three sort orders.
    pub make: Option<String>,
    /// Model, if recorded.
    pub model: Option<String>,
    /// Serial number, if recorded.
    pub serial: Option<String>,
    /// Instrument family, for the unit vocabulary.
    pub kind: models::InstrumentKind,
    /// Keys/strings carrying at least one measurement.
    pub measured_count: usize,
    /// Unix seconds of the last open.
    pub last_opened: u64,
    /// Unix seconds of the last write.
    pub modified: u64,
}

impl ProfileEntry {
    /// Reads one profile document into a browser row.
    fn read(path: PathBuf) -> Option<Self> {
        let profile = InharmonicityProfile::from_file(&path).ok()?;
        Some(Self {
            name: if profile.identity.name.is_empty() {
                path.file_stem()?.to_string_lossy().into_owned()
            } else {
                profile.identity.name.clone()
            },
            make: profile.identity.make.clone(),
            model: profile.identity.model.clone(),
            serial: profile.identity.serial.clone(),
            kind: profile.identity.kind.clone(),
            measured_count: profile.measurements.len(),
            last_opened: profile.last_opened,
            modified: profile.modified,
            path,
        })
    }

    /// Whether this row matches a browser search term (case-insensitive across
    /// every identifying field, so a serial number finds its instrument).
    pub fn matches(&self, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        let needle = needle.to_lowercase();
        let hit = |s: &Option<String>| {
            s.as_deref()
                .is_some_and(|v| v.to_lowercase().contains(&needle))
        };
        self.name.to_lowercase().contains(&needle)
            || hit(&self.make)
            || hit(&self.model)
            || hit(&self.serial)
    }
}

/// How the browser orders its rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProfileSort {
    /// Most recently opened first: the working tuner's default.
    #[default]
    LastOpened,
    /// Alphabetical by display name.
    Name,
    /// Alphabetical by manufacturer, unrecorded last.
    Make,
}

impl ProfileSort {
    /// Every order, in toggle sequence.
    pub const ALL: [ProfileSort; 3] = [
        ProfileSort::LastOpened,
        ProfileSort::Name,
        ProfileSort::Make,
    ];

    /// Label for the sort selector.
    pub fn label(self) -> &'static str {
        match self {
            ProfileSort::LastOpened => "Recent",
            ProfileSort::Name => "Name",
            ProfileSort::Make => "Make",
        }
    }
}

impl std::fmt::Display for ProfileSort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Every profile document in the library, ordered by `sort`.
///
/// Unreadable files are skipped rather than reported: the directory is the
/// user's own and may hold anything, and a browser that refuses to open
/// because of one stray file is worse than one that lists the rest.
pub(crate) fn list_profiles(sort: ProfileSort) -> Vec<ProfileEntry> {
    let dir = profiles_dir();
    let mut entries: Vec<ProfileEntry> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == PROFILE_EXTENSION))
        .filter_map(ProfileEntry::read)
        .collect();
    match sort {
        ProfileSort::LastOpened => entries.sort_by(|a, b| {
            b.last_opened
                .cmp(&a.last_opened)
                .then_with(|| b.modified.cmp(&a.modified))
        }),
        ProfileSort::Name => entries.sort_by_key(|e| e.name.to_lowercase()),
        ProfileSort::Make => entries.sort_by(|a, b| {
            let key = |e: &ProfileEntry| {
                e.make
                    .as_deref()
                    .map(|m| (0, m.to_lowercase()))
                    .unwrap_or((1, String::new()))
            };
            key(a).cmp(&key(b)).then_with(|| a.name.cmp(&b.name))
        }),
    }
    entries
}

/// Turns a display name into a safe filename stem, so a rename cannot escape
/// the profiles directory or collide with the OS's reserved characters.
fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            // Collapse runs: "Steinway B — #1234" has four separators in a row
            // between "b" and "1234", and one hyphen reads as one word break.
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "instrument".to_string()
    } else {
        slug
    }
}

/// An unused path in the library for `name`, suffixing `-2`, `-3`, … on
/// collision so creating two instruments with one name never overwrites.
pub(crate) fn unique_path_for(name: &str) -> PathBuf {
    let dir = profiles_dir();
    let stem = slugify(name);
    let mut candidate = dir.join(format!("{stem}.{PROFILE_EXTENSION}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem}-{n}.{PROFILE_EXTENSION}"));
        n += 1;
    }
    candidate
}

/// Default name for a newly created profile: dated, so a library of untitled
/// instruments is still orderable before anyone renames them.
pub(crate) fn default_profile_name() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    // Days since the epoch is enough to disambiguate a working day's profiles
    // without pulling in a date library for one label.
    format!("Untitled instrument ({})", secs / 86_400)
}

/// Imports a pre-move `tuning_profile.json` from the working directory, once.
///
/// Runs only when the library is empty, and **copies** rather than moves: the
/// original stays where the offline harnesses and `diagnose_engine --profile`
/// expect it. Returns the imported path, or `None` if there was nothing to do.
pub(crate) fn import_legacy_profile() -> Option<PathBuf> {
    let legacy = Path::new(models::PROFILE_PATH);
    if !legacy.is_file() || !list_profiles(ProfileSort::default()).is_empty() {
        return None;
    }
    // `from_file` migrates the v0 shape (one measurement per key, no identity)
    // in memory; writing it here is what makes the migration durable, and it
    // lands on the copy, never on the original.
    let mut profile = InharmonicityProfile::from_file(legacy).ok()?;
    if profile.identity.name.is_empty() {
        profile.identity.name = "Imported profile".to_string();
    }
    let path = unique_path_for(&profile.identity.name);
    profile.to_file(&path).ok()?;
    eprintln!(
        "[LIBRARY] Imported legacy {} → {} ({} keys). The original is untouched.",
        models::PROFILE_PATH,
        path.display(),
        profile.measurements.len()
    );
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_is_filename_safe() {
        assert_eq!(slugify("Steinway B — #1234/56"), "steinway-b-1234-56");
        assert_eq!(slugify("../../etc/passwd"), "etc-passwd");
        assert_eq!(slugify("   "), "instrument");
        assert_eq!(slugify(""), "instrument");
    }

    #[test]
    fn search_covers_every_identifying_field() {
        let entry = ProfileEntry {
            path: PathBuf::from("/tmp/x.json"),
            name: "Front room".into(),
            make: Some("Yamaha".into()),
            model: Some("U1".into()),
            serial: Some("H1234567".into()),
            kind: models::InstrumentKind::Piano,
            measured_count: 3,
            last_opened: 0,
            modified: 0,
        };
        assert!(entry.matches(""));
        assert!(entry.matches("yamaha"));
        assert!(entry.matches("h12345"));
        assert!(entry.matches("ROOM"));
        assert!(!entry.matches("bosendorfer"));
    }

    #[test]
    fn recents_promote_and_cap() {
        let mut s = AppSettings::default();
        for i in 0..(MAX_RECENTS + 4) {
            s.note_opened(Path::new(&format!("/tmp/p{i}.json")));
        }
        assert_eq!(s.recents.len(), MAX_RECENTS);
        assert_eq!(
            s.recents[0],
            PathBuf::from(format!("/tmp/p{}.json", MAX_RECENTS + 3))
        );

        let again = PathBuf::from(format!("/tmp/p{}.json", MAX_RECENTS + 1));
        s.note_opened(&again);
        assert_eq!(s.recents[0], again);
        assert_eq!(
            s.recents.iter().filter(|p| **p == again).count(),
            1,
            "reopening must promote, not duplicate"
        );

        s.note_removed(&again);
        assert!(!s.recents.contains(&again));
    }
}
