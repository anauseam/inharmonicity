//! # Profile session — the open instrument and when it is written
//!
//! Owns the profile currently being measured, the file it is persisted to, and
//! the small state machine around writing it: whether this session has taken
//! its rollback copy, and whether an interaction-rate edit is still waiting to
//! be flushed.
//!
//! Kept out of `app.rs` because it is the one part of the frontend with
//! invariants of its own rather than message routing, and it can be exercised
//! without a GUI. `app.rs` holds a [`ProfileSession`] and its handlers become
//! calls into it.
//!
//! The persistence rules — save on every measurement and every undo, atomic
//! write, a `.bak` per session, coalesced typing — are argued in
//! `docs/design/session-persistence-and-profile-library.md` §4.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tuner_core::models::{InharmonicityProfile, KeyMeasurement};

use crate::library::{self, AppSettings};

/// How long an interaction-rate edit waits for the interaction to stop before
/// it is written.
///
/// Text inputs emit per character and sliders per step, while the profile is a
/// single ~140 KB document — writing on each event would rewrite the whole file
/// eight times to type a name, and hundreds of times to drag a threshold. The
/// worst case this costs is the tail of a name typed in the last moment before
/// a crash; measurements and undo never take this path.
const EDIT_FLUSH_QUIET: Duration = Duration::from_millis(700);

/// Captures whose most recent measurement can still be undone in one session.
///
/// Bounded because undo is the short-timescale remedy only — "the mistake you
/// just made". A key that looks wrong later is the inspector's job; total loss
/// is the `.bak`'s.
const UNDO_HISTORY_DEPTH: usize = 100;

/// The instrument currently open, its file, and the write policy around it.
#[derive(Default)]
pub struct ProfileSession {
    /// The authoritative in-memory profile.
    profile: InharmonicityProfile,
    /// Where it is persisted. `None` only before the first profile exists.
    path: Option<PathBuf>,
    /// Whether this session has written its `.bak` for [`Self::path`]. Reset
    /// whenever a different profile is opened.
    backed_up: bool,
    /// An interaction-rate edit is pending a write; the clock restarts on each
    /// keystroke so the write lands once the user stops.
    dirty_since: Option<Instant>,
    /// Keys whose most recent measurement can still be undone, oldest first.
    ///
    /// Only the key is stored: a capture *appends* to that key's list, so
    /// undoing it is popping the entry back off — there is no displaced value
    /// to carry. Session-scoped and never persisted.
    undo_history: std::collections::VecDeque<u8>,
}

impl ProfileSession {
    /// The open profile.
    pub fn profile(&self) -> &InharmonicityProfile {
        &self.profile
    }

    /// The open profile, mutably. Callers that change it must follow with
    /// [`Self::persist`] or [`Self::touch`].
    pub fn profile_mut(&mut self) -> &mut InharmonicityProfile {
        &mut self.profile
    }

    /// Path of the open profile.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The key whose capture the next undo would revert, with the timestamp of
    /// the entry that would be discarded — repeat captures share a key, so the
    /// epoch is the only thing distinguishing which one. Matches the on-disk
    /// `key_<idx>_<note>_<epoch>` dump name.
    pub fn undo_target(&self) -> Option<(u8, Option<&str>)> {
        let &key = self.undo_history.back()?;
        let epoch = self
            .profile
            .measurements
            .get(&key)
            .and_then(|entries| entries.last())
            .map(|m| m.last_captured.as_str())
            .filter(|s| !s.is_empty());
        Some((key, epoch))
    }

    /// Resolves which instrument the session starts on, recording the choice in
    /// `settings`.
    ///
    /// In order: the profile open when the app last closed; else a one-time
    /// import of a pre-library `tuning_profile.json` from the working
    /// directory; else a fresh empty instrument, so autosave always has
    /// somewhere to write.
    pub fn open_at_startup(&mut self, settings: &mut AppSettings) {
        if let Some(path) = settings.last_profile.clone()
            && let Ok(profile) = InharmonicityProfile::from_file(&path)
        {
            self.adopt(profile, path, settings);
            return;
        }
        if let Some(path) = library::import_legacy_profile()
            && let Ok(profile) = InharmonicityProfile::from_file(&path)
        {
            self.adopt(profile, path, settings);
            return;
        }
        let name = library::default_profile_name();
        let path = library::unique_path_for(&name);
        self.adopt(InharmonicityProfile::new(name), path, settings);
    }

    /// Makes `profile` at `path` the open instrument, stamping `last_opened`
    /// and recording it as the one to resume next launch.
    ///
    /// Undo history is dropped: it indexes measurements of the instrument being
    /// closed, and replaying it against a different one would write a
    /// stranger's measurement into this profile.
    pub fn adopt(
        &mut self,
        mut profile: InharmonicityProfile,
        path: PathBuf,
        settings: &mut AppSettings,
    ) {
        profile.last_opened = unix_now();
        // Mint the identity on first open, so a profile written before the
        // field existed gets one and the `persist` below makes it durable.
        // Everything that must survive a rename — the dump directory above all
        // — keys off this, so it is minted exactly once and never rewritten.
        if profile.identity.id.is_empty() {
            profile.identity.id = uuid::Uuid::now_v7().to_string();
            eprintln!(
                "[SESSION] Minted instrument id {} for '{}'",
                profile.identity.id, profile.identity.name
            );
        }
        self.profile = profile;
        self.path = Some(path.clone());
        self.backed_up = false;
        self.dirty_since = None;
        self.undo_history.clear();

        settings.note_opened(&path);
        // Stamp `last_opened` on disk straight away, so the browser's Recent
        // order is right even if this session takes no captures.
        self.persist();
        eprintln!(
            "[SESSION] Opened '{}' ({} measured) from {}",
            self.profile.identity.name,
            self.profile.measurements.len(),
            path.display()
        );
    }

    /// Appends a capture and persists immediately.
    ///
    /// Appending rather than replacing is what keeps an unattended auto-mode
    /// capture from destroying a manual one: `InharmonicityProfile::active`
    /// prefers the newest *trusted* entry, so an auto capture adds evidence but
    /// never displaces a trusted measurement.
    pub fn record(&mut self, measurement: KeyMeasurement) {
        let key = measurement.key_index;
        self.undo_history.push_back(key);
        if self.undo_history.len() > UNDO_HISTORY_DEPTH {
            self.undo_history.pop_front();
        }
        self.profile.record(measurement);
        self.persist();
    }

    /// Reverts the most recent capture, returning the entry that was removed so
    /// the caller can delete its diagnostics dump. Persists immediately —
    /// otherwise an undo would leave the bad measurement on disk.
    pub fn undo(&mut self) -> Option<(u8, KeyMeasurement)> {
        let key = self.undo_history.pop_back()?;
        let removed = self.profile.undo_last(key);
        self.persist();
        removed.map(|m| (key, m))
    }

    /// Discards one retained measurement of `key` — the inspector's drop —
    /// returning it. Persists immediately, for the same reason undo does.
    ///
    /// One undo of that key is forgotten with it: undo pops the key's tail, so
    /// leaving the history intact would let a later undo discard a *different*
    /// measurement than the one it was recorded for.
    pub fn remove(&mut self, key: u8, index: usize) -> Option<KeyMeasurement> {
        let removed = self.profile.remove(key, index)?;
        if let Some(pos) = self.undo_history.iter().rposition(|&k| k == key) {
            self.undo_history.remove(pos);
        }
        self.persist();
        Some(removed)
    }

    /// Marks an interaction-rate edit for a coalesced write. The clock restarts
    /// on every call, so the write lands once the user stops typing or dragging
    /// rather than once per character or slider step.
    pub fn touch(&mut self) {
        self.dirty_since = Some(Instant::now());
    }

    /// Whether an edit is waiting to be written.
    pub fn is_dirty(&self) -> bool {
        self.dirty_since.is_some()
    }

    /// Writes a pending edit once the interaction has stopped. Called from the
    /// frontend's tick loop.
    pub fn flush_if_quiet(&mut self) {
        if self
            .dirty_since
            .is_some_and(|t| t.elapsed() >= EDIT_FLUSH_QUIET)
        {
            self.persist();
        }
    }

    /// Writes the profile, taking this session's `.bak` first if it has not
    /// been taken yet.
    ///
    /// The write itself is atomic (`InharmonicityProfile::to_file`); the `.bak`
    /// is a rollback point that does not depend on the in-memory undo stack.
    pub fn persist(&mut self) {
        let Some(path) = self.path.clone() else {
            eprintln!("[SESSION] No profile open; nothing to save.");
            return;
        };
        if !self.backed_up {
            // Only meaningful once the file exists — a brand-new profile has
            // nothing to roll back to, and its first write is not a risk.
            if path.is_file() {
                let mut bak = path.as_os_str().to_owned();
                bak.push(".bak");
                match std::fs::copy(&path, PathBuf::from(bak)) {
                    Ok(_) => eprintln!("[SESSION] Rollback point written."),
                    Err(e) => eprintln!("[SESSION] Could not write rollback point: {e}"),
                }
            }
            self.backed_up = true;
        }
        if let Err(e) = self.profile.to_file(&path) {
            eprintln!("[SESSION] Error saving profile to {}: {e}", path.display());
        }
        self.dirty_since = None;
    }

    /// Deletes a profile document, refusing the open one — autosave would then
    /// be writing to a file that no longer exists. Returns whether it went.
    pub fn delete(&mut self, path: &Path) -> bool {
        if self.path.as_deref() == Some(path) {
            eprintln!("[SESSION] Refusing to delete the open profile.");
            return false;
        }
        match std::fs::remove_file(path) {
            Ok(()) => {
                eprintln!("[SESSION] Deleted profile {}", path.display());
                true
            }
            Err(e) => {
                eprintln!("[SESSION] Could not delete {}: {e}", path.display());
                false
            }
        }
    }
}

/// Unix seconds now, or 0 if the clock is before the epoch.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measurement(key: u8, f0: f32) -> KeyMeasurement {
        KeyMeasurement {
            key_index: key,
            measured_f0: f0,
            partials: Vec::new(),
            calculated_b: Some(1e-4),
            last_captured: format!("{f0}"),
            captured_in_auto: false,
            sounding_strings: None,
        }
    }

    /// Nothing in this module may write the real `settings.json`: these tests
    /// run on a developer's machine, and clobbering the resume pointer there
    /// once already sent the app to a fresh profile on next launch. `adopt`
    /// therefore takes the settings to update, and the caller decides when —
    /// and whether — they reach disk.
    ///
    /// A session resuming an instrument that already exists on disk — the case
    /// the rollback point is for. (A brand-new profile deliberately gets none:
    /// there is nothing to roll back to.)
    fn session(tag: &str) -> (ProfileSession, PathBuf) {
        let dir = std::env::temp_dir().join(format!("inh-sess-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("upright.json");
        let mut existing = InharmonicityProfile::new("Test upright");
        existing.to_file(&path).unwrap();
        (
            ProfileSession {
                profile: existing,
                path: Some(path.clone()),
                ..ProfileSession::default()
            },
            path,
        )
    }

    /// A capture writes through immediately, and undo pops it back off and
    /// writes again — the file must never lag the in-memory state.
    #[test]
    fn capture_and_undo_both_reach_disk() {
        let (mut s, path) = session("undo");

        s.record(measurement(5, 100.0));
        let on_disk = InharmonicityProfile::from_file(&path).unwrap();
        assert_eq!(on_disk.active(5).unwrap().measured_f0, 100.0);

        let (key, removed) = s.undo().unwrap();
        assert_eq!(key, 5);
        assert_eq!(removed.measured_f0, 100.0);
        let on_disk = InharmonicityProfile::from_file(&path).unwrap();
        assert!(
            on_disk.active(5).is_none(),
            "undo must not leave the reverted capture on disk"
        );

        assert!(s.undo().is_none(), "history is empty");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    /// The `.bak` is taken once per session, before the first write, and is not
    /// re-taken on later writes — otherwise it would track the damage it exists
    /// to undo.
    #[test]
    fn backup_is_taken_once_per_session() {
        let (mut s, path) = session("bak");
        let bak = path.with_extension("json.bak");

        s.record(measurement(1, 10.0));
        assert!(bak.is_file(), "first write takes the rollback point");
        let after_first = std::fs::read_to_string(&bak).unwrap();

        s.record(measurement(2, 20.0));
        let after_second = std::fs::read_to_string(&bak).unwrap();
        assert_eq!(
            after_first, after_second,
            "the rollback point must not advance with later writes"
        );
        assert!(!after_second.contains("\"key_index\": 2"));

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    /// An inspector drop reaches any entry, writes through, and consumes one
    /// undo of that key — otherwise the next undo would discard a measurement
    /// it was never recorded for.
    #[test]
    fn a_drop_writes_through_and_consumes_one_undo() {
        let (mut s, path) = session("drop");
        s.record(measurement(4, 10.0));
        s.record(measurement(4, 20.0));
        assert_eq!(s.undo_history.len(), 2);

        // The older entry — the one undo cannot reach.
        assert_eq!(s.remove(4, 0).unwrap().measured_f0, 10.0);
        let on_disk = InharmonicityProfile::from_file(&path).unwrap();
        assert_eq!(on_disk.measurements[&4].len(), 1);
        assert_eq!(on_disk.active(4).unwrap().measured_f0, 20.0);
        assert_eq!(s.undo_history.len(), 1, "the drop consumed one undo");

        // The one remaining undo removes the one remaining entry, leaving the
        // history empty rather than pointing at a key that no longer exists.
        assert_eq!(s.undo().unwrap().1.measured_f0, 20.0);
        assert!(s.undo().is_none());
        assert!(s.remove(4, 0).is_none(), "nothing left to drop");

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    /// Undo history is bounded and drops on adopting a different instrument.
    #[test]
    fn undo_history_is_bounded_and_session_scoped() {
        let (mut s, path) = session("hist");
        for i in 0..(UNDO_HISTORY_DEPTH + 10) {
            s.record(measurement((i % 88) as u8, i as f32));
        }
        assert_eq!(s.undo_history.len(), UNDO_HISTORY_DEPTH);

        s.adopt(
            InharmonicityProfile::new("Another"),
            path.with_file_name("another.json"),
            &mut AppSettings::default(),
        );
        assert!(
            s.undo_history.is_empty(),
            "undo must not survive an instrument change"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    /// A brand-new instrument gets no rollback point: backing up an empty
    /// profile would be a rollback target worse than the thing it protects.
    #[test]
    fn a_new_profile_has_nothing_to_roll_back_to() {
        let dir = std::env::temp_dir().join(format!("inh-sess-new-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fresh.json");
        let mut s = ProfileSession {
            profile: InharmonicityProfile::new("Fresh"),
            path: Some(path.clone()),
            ..ProfileSession::default()
        };
        s.record(measurement(3, 30.0));
        assert!(path.is_file());
        assert!(!path.with_extension("json.bak").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A touched edit is not written until the quiet delay elapses, and
    /// `persist` clears the pending state.
    #[test]
    fn edits_coalesce_until_quiet() {
        let (mut s, path) = session("quiet");
        s.touch();
        assert!(s.is_dirty());
        s.flush_if_quiet();
        assert!(s.is_dirty(), "flushed before the quiet delay elapsed");
        s.persist();
        assert!(!s.is_dirty());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
