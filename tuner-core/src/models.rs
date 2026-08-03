//! # Domain Data Models
//!
//! Domain types for the tuner: notes and the 88-key lookup tables ([`NOTES`],
//! [`Note`], [`NOTE_MAP`]), captured measurements ([`Partial`], [`KeyMeasurement`],
//! [`InharmonicityProfile`] with its [`InstrumentIdentity`] and
//! [`ProfileSettings`]), the two persisted tuning selections ([`EngineChoice`],
//! [`ReferenceMode`]), and the discovery templates ([`KeyProfile`]).
//!
//! It also holds the small body of *domain-specific* math that produces those types —
//! the Rigaud inharmonicity prior ([`get_expected_beta`]), the Railsback stretch curve
//! ([`railsback_stretch_curve`]), and the stiff-string partial law in
//! [`KeyProfile::new`].

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Highest representable partial frequency — the Nyquist limit.
///
/// Derived from [`crate::audio::SAMPLE_RATE`]: the one spot where `models` reaches up
/// into `audio` (which already depends on `models`), forming a small `models ↔ audio`
/// cycle. Accepted for now — the crate already has intra-crate cycles. A future refactor
/// extracts the shared DSP/stream constants into a leaf module (tracked in `TODO.md`).
const NYQUIST_HZ: f32 = crate::audio::SAMPLE_RATE as f32 / 2.0;

/// Maximum number of partials modeled per key.
pub const MAX_PARTIALS: usize = 128;

/// Calculates the deviation from a target frequency in cents.
///
/// Cents are a logarithmic unit of pitch measurement where:
/// - 100 cents = 1 semitone
/// - 1200 cents = 1 octave
/// - Positive values indicate sharpness, negative values indicate flatness
///
/// # Arguments
/// * `freq` - Measured frequency in Hz
/// * `target_freq` - Target frequency in Hz
///
/// # Returns
/// * Cent deviation (positive = sharp, negative = flat)
pub fn calculate_cents_deviation(freq: f32, target_freq: f32) -> f32 {
    1200.0 * (freq / target_freq).log2()
}

/// One spectral peak: a local magnitude maximum with a sub-bin-refined
/// frequency, produced by `algorithms::peaks::extract_peaks` and consumed by
/// `twm` / `discovery`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SpectralPeak {
    /// True frequency in Hz (sub-bin interpolated via the Jacobsen estimator
    /// (Candan 2015)).
    pub frequency: f32,
    /// Linear magnitude at this peak.
    pub magnitude: f32,
}

/// A single measured partial (overtone) of a piano note.
///
/// The fundamental is `number = 1`. Overtones are `number = 2, 3, …`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Partial {
    /// The partial number (n). 1 = fundamental, 2 = first overtone, etc.
    pub number: u32,
    /// The measured frequency of this partial in Hz.
    pub frequency: f32,
    /// Amplitude of this partial (for spectral envelope analysis).
    pub amplitude: f32,
}

/// Stores all measured partials for a single piano key, plus the computed
/// inharmonicity constant (B).
///
/// Created by the capture processing pipeline after the Gatekeeper triggers
/// a successful capture and the Worker runs partial extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyMeasurement {
    /// The 88-key piano index (0 = A0, 87 = C8).
    pub key_index: u8,
    /// Measured fundamental frequency (Hz).
    pub measured_f0: f32,
    /// All measured partials for this key (fundamental + overtones).
    pub partials: Vec<Partial>,
    /// The computed inharmonicity coefficient, or `None` if not yet calculated
    /// or if there were insufficient partials.
    pub calculated_b: Option<f32>,
    /// UTC timestamp of the most recent capture (ISO format).
    pub last_captured: String,
    /// Capture provenance: `true` when the key identity came from the
    /// auto-discovery path, `false` when the user named the key (manual
    /// mode). The tuning curve consumes **manual captures only** (ADR 0006
    /// Corrections item 3; tuning-curve design note §10.1). Legacy profile
    /// entries predate this field and deserialize as `true` (auto/untrusted)
    /// so pre-flag data can never feed the curve.
    #[serde(default = "default_captured_in_auto")]
    pub captured_in_auto: bool,
}

/// Serde default for [`KeyMeasurement::captured_in_auto`]: legacy entries
/// are untrusted (see the field doc).
fn default_captured_in_auto() -> bool {
    true
}

/// Filename a pre-library profile was written to, relative to the working
/// directory. Read-only: the frontend looks here once to import such a file,
/// and [`AudioPipeline::new`](crate::pipeline::AudioPipeline::new)'s gated
/// discovery-seeding path still reads it. Nothing writes it.
pub const PROFILE_PATH: &str = "tuning_profile.json";

/// Current [`InharmonicityProfile`] schema version. Version `0` — one
/// measurement per key, no identity or settings — still deserializes; see
/// [`InharmonicityProfile::from_file`].
pub(crate) const PROFILE_SCHEMA_VERSION: u32 = 1;

/// Measurements retained per key before the oldest is dropped.
///
/// Ours. Repeats exist to be compared against each other in the inspector,
/// which needs only a few per key; the bound keeps a file that is rewritten on
/// every capture from growing without limit.
pub(crate) const MAX_MEASUREMENTS_PER_KEY: usize = 8;

/// Default NHWRSF onset threshold — the flux a transient must exceed to be
/// declared a new note event.
pub(crate) const DEFAULT_NHWRSF_THRESHOLD: f32 = 0.9;

/// Default NINOS² sustain-stability threshold.
pub(crate) const DEFAULT_NINOS2_STABILITY_THRESHOLD: f32 = 10.0;

/// The family of instrument a profile describes.
///
/// Affects display vocabulary only; every instrument is measured against the
/// same 88-slot compass ([`midi_from_key`]).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum InstrumentKind {
    /// The current focus: the only family with a per-instrument stretch curve.
    #[default]
    Piano,
    /// Six-string guitar and relatives.
    Guitar,
    /// Bass guitar, upright bass.
    Bass,
    /// Harp, harpsichord, and other multi-course plucked instruments.
    Harp,
}

impl InstrumentKind {
    /// What this instrument's measured units are called in the UI.
    pub fn unit_plural(&self) -> &'static str {
        match self {
            InstrumentKind::Piano => "keys",
            _ => "strings",
        }
    }
}

impl std::fmt::Display for InstrumentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstrumentKind::Piano => f.write_str("Piano"),
            InstrumentKind::Guitar => f.write_str("Guitar"),
            InstrumentKind::Bass => f.write_str("Bass"),
            InstrumentKind::Harp => f.write_str("Harp"),
        }
    }
}

/// Who and what the profile is a profile *of*.
///
/// Every field but `name` is optional: a profile exists from its first capture
/// and may be identified later, so no field may be required to save one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstrumentIdentity {
    /// Display name. Auto-generated on creation, renameable.
    pub name: String,
    /// Instrument family — drives display vocabulary only.
    #[serde(default)]
    pub kind: InstrumentKind,
    /// Manufacturer.
    #[serde(default)]
    pub make: Option<String>,
    /// Model designation.
    #[serde(default)]
    pub model: Option<String>,
    /// Serial number — the one field that identifies an instrument uniquely.
    #[serde(default)]
    pub serial: Option<String>,
    /// Body form within the family: grand/upright/spinet, dreadnought, etc.
    #[serde(default)]
    pub form: Option<String>,
    /// The instrument's owner.
    #[serde(default)]
    pub owner: Option<String>,
    /// Free text.
    #[serde(default)]
    pub notes: Option<String>,
}

/// Which engine of the `CurveBundle` the plot and strobe display — strobe
/// design note §9/§13. Selection is **display-only** (D7): every engine is
/// already in the bundle, so switching never triggers a recompute. The (c) ρ
/// Low/High presets are not variants yet — they are not computed until (c)'s
/// calibration is factored out of the per-preset path (§14 step 6); the gallery
/// renders them as deferred placeholders.
///
/// Resolve it against a bundle with `CurveBundle::curve`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum EngineChoice {
    /// (a) Rigaud-pure prior curve.
    RigaudPure,
    /// (b) per-key measured B + Whittaker smoothing.
    PerKeySmoothed,
    /// (c) Giordano-calibrated, ρ Mean preset.
    GiordanoMean,
    /// (d) multi-interval BALANCED — the manual-mode default (D7).
    #[default]
    MultiBalanced,
    /// (d) multi-interval PURE 12ths preset.
    MultiPureTwelfths,
}

impl EngineChoice {
    /// Every selectable engine, in gallery order.
    pub const ALL: [EngineChoice; 5] = [
        EngineChoice::RigaudPure,
        EngineChoice::PerKeySmoothed,
        EngineChoice::GiordanoMean,
        EngineChoice::MultiBalanced,
        EngineChoice::MultiPureTwelfths,
    ];

    /// Full display name (detail view / panel titles).
    pub fn label(&self) -> &'static str {
        match self {
            EngineChoice::RigaudPure => "(a) Rigaud prior",
            EngineChoice::PerKeySmoothed => "(b) Per-key + Whittaker",
            EngineChoice::GiordanoMean => "(c) Giordano · ρ Mean",
            EngineChoice::MultiBalanced => "(d) Multi-interval · Balanced",
            EngineChoice::MultiPureTwelfths => "(d) Multi-interval · Pure 12ths",
        }
    }

    /// Short name for gallery thumbnails (the section header names the class).
    pub fn short_label(&self) -> &'static str {
        match self {
            EngineChoice::RigaudPure => "Rigaud prior",
            EngineChoice::PerKeySmoothed => "Per-key smoothed",
            EngineChoice::GiordanoMean => "ρ Mean",
            EngineChoice::MultiBalanced => "Balanced",
            EngineChoice::MultiPureTwelfths => "Pure 12ths",
        }
    }
}

/// Which target function the app measures against — every readout shares it:
/// the strobe band, its cents readout, and the cent meter.
///
/// Orthogonal to reference *pitch* (design §11's A440 / [`TuningCurve::d_g`],
/// which shifts the whole curve): this selects *which* target function, not
/// where it is anchored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ReferenceMode {
    /// The instrument's stretched `target_f1` (per-partial targets for the
    /// strobe). The default, and the only mode that uses measured `B`.
    #[default]
    Curve,
    /// Pure equal temperament, **fundamental only** — the n = 1 target is
    /// B-immune (design R4), so no per-string inharmonicity is needed and a
    /// correctly-pitched string shows no false beat. The instrument-agnostic
    /// mode: it makes the app usable on a non-piano (e.g. a guitar string).
    Et,
}

impl ReferenceMode {
    /// The other mode — the toggle's target.
    pub fn toggled(self) -> Self {
        match self {
            ReferenceMode::Curve => ReferenceMode::Et,
            ReferenceMode::Et => ReferenceMode::Curve,
        }
    }

    /// Short label for the toggle button.
    pub fn label(self) -> &'static str {
        match self {
            ReferenceMode::Curve => "Ref: Curve",
            ReferenceMode::Et => "Ref: ET",
        }
    }
}

/// Per-instrument settings that persist with the profile.
///
/// The two thresholds are level-independent quantities (NHWRSF is normalized
/// by Σ|X|; NINOS² is a dimensionless ratio), so they characterise the
/// instrument rather than the rig — unlike the silence floor, an absolute RMS,
/// which is measured afresh each session and is deliberately absent here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileSettings {
    /// NHWRSF onset threshold for this instrument.
    #[serde(default = "default_nhwrsf_threshold")]
    pub nhwrsf_threshold: f32,
    /// NINOS² sustain-stability threshold for this instrument.
    #[serde(default = "default_ninos2_threshold")]
    pub ninos2_stability_threshold: f32,
    /// Which curve engine this instrument was last tuned with. Consulted only
    /// when `reference_mode` is [`ReferenceMode::Curve`].
    #[serde(default)]
    pub engine: EngineChoice,
    /// Curve or plain ET. Per instrument because the curve engines are piano
    /// models — a guitar profile that reopened in `Curve` mode would be wrong
    /// every time.
    #[serde(default)]
    pub reference_mode: ReferenceMode,
}

/// Serde default for [`ProfileSettings::nhwrsf_threshold`].
fn default_nhwrsf_threshold() -> f32 {
    DEFAULT_NHWRSF_THRESHOLD
}

/// Serde default for [`ProfileSettings::ninos2_stability_threshold`].
fn default_ninos2_threshold() -> f32 {
    DEFAULT_NINOS2_STABILITY_THRESHOLD
}

impl Default for ProfileSettings {
    fn default() -> Self {
        Self {
            nhwrsf_threshold: DEFAULT_NHWRSF_THRESHOLD,
            ninos2_stability_threshold: DEFAULT_NINOS2_STABILITY_THRESHOLD,
            engine: EngineChoice::default(),
            reference_mode: ReferenceMode::default(),
        }
    }
}

/// The complete inharmonicity profile for one instrument.
///
/// The top-level serializable object saved to and loaded from a JSON file: who
/// the instrument is, its settings, and every measurement taken of it. A key
/// holds a **list** of measurements, newest last — repeats are retained so a
/// suspect capture can be compared against the others rather than silently
/// overwriting the good one, and so an untrusted (auto-mode) capture can be
/// kept for review without ever displacing a trusted one. [`Self::active`]
/// resolves the list to the single measurement consumers read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InharmonicityProfile {
    /// Schema version, for migration. See [`PROFILE_SCHEMA_VERSION`].
    #[serde(default)]
    pub version: u32,
    /// The instrument this profile describes.
    #[serde(default)]
    pub identity: InstrumentIdentity,
    /// Persisted per-instrument settings.
    #[serde(default)]
    pub settings: ProfileSettings,
    /// Unix seconds at which the profile was created.
    #[serde(default)]
    pub created: u64,
    /// Unix seconds of the last write.
    #[serde(default)]
    pub modified: u64,
    /// Unix seconds at which the profile was last opened — the sort order a
    /// working tuner reaches for most ("date accessed", strobe design §12).
    #[serde(default)]
    pub last_opened: u64,
    /// Maps a key index (0–87) to every measurement taken of it, oldest first.
    /// Capped at [`MAX_MEASUREMENTS_PER_KEY`].
    #[serde(default)]
    pub measurements: BTreeMap<u8, Vec<KeyMeasurement>>,
}

impl Default for InharmonicityProfile {
    fn default() -> Self {
        Self {
            version: PROFILE_SCHEMA_VERSION,
            identity: InstrumentIdentity::default(),
            settings: ProfileSettings::default(),
            created: 0,
            modified: 0,
            last_opened: 0,
            measurements: BTreeMap::new(),
        }
    }
}

/// The pre-versioning (v0) profile shape: one measurement per key, no identity
/// and no settings. Read only by [`InharmonicityProfile::from_file`]'s fallback
/// so a profile written before the v1 bump still opens.
#[derive(Deserialize)]
struct ProfileV0 {
    measurements: BTreeMap<u8, KeyMeasurement>,
}

/// Unix seconds now, or 0 if the clock is before the epoch.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

impl InharmonicityProfile {
    /// A new, empty profile for `name`, stamped with the current time.
    pub fn new(name: impl Into<String>) -> Self {
        let now = unix_now();
        Self {
            identity: InstrumentIdentity {
                name: name.into(),
                ..InstrumentIdentity::default()
            },
            created: now,
            modified: now,
            last_opened: now,
            ..Self::default()
        }
    }

    /// The one measurement consumers read for `key`, or `None` if unmeasured.
    ///
    /// **Newest trusted, else newest** — the ADR 0006 item 3 provenance rule
    /// over a list. An auto-mode entry never displaces a manual one however
    /// recent it is, because a discovery false-lock makes MAT confidently
    /// measure the wrong series under the wrong key; it is retained, not read.
    pub fn active(&self, key: u8) -> Option<&KeyMeasurement> {
        let entries = self.measurements.get(&key)?;
        entries
            .iter()
            .rev()
            .find(|m| !m.captured_in_auto)
            .or_else(|| entries.last())
    }

    /// [`Self::active`] for in-place edits — the inspector's handle on the
    /// entry a key currently presents.
    pub fn active_mut(&mut self, key: u8) -> Option<&mut KeyMeasurement> {
        let entries = self.measurements.get_mut(&key)?;
        let pos = entries
            .iter()
            .rposition(|m| !m.captured_in_auto)
            .or_else(|| entries.len().checked_sub(1))?;
        entries.get_mut(pos)
    }

    /// Every key that carries a measurement, paired with its active entry.
    pub fn active_entries(&self) -> impl Iterator<Item = (u8, &KeyMeasurement)> {
        self.measurements
            .keys()
            .filter_map(|&k| self.active(k).map(|m| (k, m)))
    }

    /// Appends a measurement to its key, evicting the oldest entry that is not
    /// the active one once [`MAX_MEASUREMENTS_PER_KEY`] is reached.
    pub fn record(&mut self, measurement: KeyMeasurement) {
        let key = measurement.key_index;
        let entries = self.measurements.entry(key).or_default();
        entries.push(measurement);
        while entries.len() > MAX_MEASUREMENTS_PER_KEY {
            // The active entry is the newest trusted one, so the oldest
            // droppable entry is the first that is not it.
            let active_pos = entries
                .iter()
                .rposition(|m| !m.captured_in_auto)
                .unwrap_or(entries.len() - 1);
            let drop_at = if active_pos == 0 { 1 } else { 0 };
            entries.remove(drop_at);
        }
    }

    /// Removes the most recently appended measurement for `key`, returning it.
    /// Leaves the key absent entirely if that was its only measurement — the
    /// shape an undo of a first capture must restore.
    pub fn undo_last(&mut self, key: u8) -> Option<KeyMeasurement> {
        let entries = self.measurements.get_mut(&key)?;
        let popped = entries.pop();
        if entries.is_empty() {
            self.measurements.remove(&key);
        }
        popped
    }

    /// Removes the measurement at `index` in `key`'s list, returning it, or
    /// `None` if the key or index does not exist.
    ///
    /// The reviewing counterpart to [`Self::undo_last`], which can only pop the
    /// tail: a repeat that looks wrong later is rarely the newest one. Holds the
    /// same invariant — a key left with no measurements disappears entirely, so
    /// it reads as unmeasured rather than as an empty list.
    pub fn remove(&mut self, key: u8, index: usize) -> Option<KeyMeasurement> {
        let entries = self.measurements.get_mut(&key)?;
        if index >= entries.len() {
            return None;
        }
        let removed = entries.remove(index);
        if entries.is_empty() {
            self.measurements.remove(&key);
        }
        Some(removed)
    }

    /// Saves the profile to a JSON file, **atomically**: the bytes go to a
    /// temporary file beside the target and are renamed over it, so a crash
    /// mid-write leaves the previous profile intact rather than truncating it.
    /// The temp file is a sibling deliberately — `rename` is only atomic within
    /// one filesystem. Stamps `modified`.
    pub fn to_file(&mut self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        let path = path.as_ref();
        self.modified = unix_now();
        let json_string = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;

        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = std::path::PathBuf::from(tmp);
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        {
            use std::io::Write;
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(json_string.as_bytes())?;
            file.sync_all()?;
        }
        std::fs::rename(&tmp, path)
    }

    /// Loads a profile from a JSON file, migrating the v0 shape if needed.
    ///
    /// A v0 file has one measurement per key and no identity: each becomes a
    /// single-entry list, and the profile is named after the file it came from.
    /// The migration is in-memory only — the caller decides whether to write it
    /// back, so opening an old profile read-only never rewrites it.
    pub fn from_file(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let data = std::fs::read_to_string(path)?;
        if let Ok(profile) = serde_json::from_str::<Self>(&data) {
            return Ok(profile);
        }
        let legacy: ProfileV0 = serde_json::from_str(&data).map_err(std::io::Error::other)?;
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Imported profile".to_string());
        let mut profile = Self::new(name);
        for (key, measurement) in legacy.measurements {
            profile.measurements.insert(key, vec![measurement]);
        }
        Ok(profile)
    }
}

/// Per-key input to the tuning-curve engines, derived from a
/// [`KeyMeasurement`] that passed the trust rules (see [`CurveInput`]).
///
/// All quantities are `f64` — curve math is cold-path and precision-bound
/// (0.01 ¢ targets), unlike the `f32` DSP hot path.
#[derive(Debug, Clone)]
pub struct CurveKeyData {
    /// Measured inharmonicity coefficient B (raw — smoothing/fallback
    /// decisions belong to the engines; strobe targets use this value
    /// always, per the design note's strobe/curve B split, §5 D3).
    pub b: f64,
    /// Flexible-string fundamental F_0 (Hz), derived from the partial list
    /// and B via Rigaud Eq. 20 (`algorithms::rigaud::f0_from_partials`)
    /// — **not** `measured_f0`, which is the Goertzel seed. The audible first
    /// partial is f_1 = F_0√(1+B) (design note §1 convention rule).
    pub f0: f64,
    /// Measured partials as `(n, frequency_hz, amplitude)` — the Giordano
    /// layer's input.
    pub partials: Vec<(u32, f64, f64)>,
}

/// Trust-filtered, engine-ready view of an [`InharmonicityProfile`]:
/// one optional [`CurveKeyData`] per key of the 88-key compass.
///
/// [`CurveInput::from_profile`] enforces the provenance rule — the curve
/// consumes **manual-mode captures only** (ADR 0006 item 3) — plus basic
/// validity (B finite and positive, ≥ 2 partials, Eq.-20 F₀ solvable).
#[derive(Debug, Clone)]
pub struct CurveInput {
    /// Index 0 = A0 … 87 = C8; `None` where no trusted measurement exists.
    /// Always length 88 — the engines index it directly.
    pub keys: Vec<Option<CurveKeyData>>,
}

impl Default for CurveInput {
    /// An empty 88-key input (no trusted measurements). A derived `Default`
    /// would produce an empty `Vec` and break the length-88 invariant the
    /// engines index against.
    fn default() -> Self {
        Self {
            keys: (0..88).map(|_| None).collect(),
        }
    }
}

impl CurveInput {
    /// Number of keys carrying trusted measurements.
    pub fn measured_count(&self) -> usize {
        self.keys.iter().filter(|k| k.is_some()).count()
    }
}

/// Per-key status flags on a computed [`TuningCurve`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CurveKeyFlags {
    /// A trusted measurement fed the curve at this key.
    pub measured: bool,
    /// Curve-side B is prior-dominated: the precision-weighted blend of
    /// measured B toward the B_ξ fit (ADR 0009; design note §8, defaults
    /// #13.3) gave the measurement less than half the weight — the treble
    /// information floor — or the key carries no measurement at all.
    /// Strobe targets still use the raw measured B.
    pub curve_b_fallback: bool,
    /// This key's measured B was excluded by the negative-stretch validity
    /// detector (design note §2): its octave pair implied d(m+12) < d(m)
    /// and this key was the larger prior-deviator. Recapture recommended.
    pub excluded: bool,
    /// Engine (c) only: this key's octave pair failed the Giordano
    /// sufficiency gate (edge-hit or < 8 strong cross pairs) and did not
    /// contribute a ρ point.
    pub giordano_excluded: bool,
    /// The **final** curve still violates d(m+12) ≥ d(m) at a pair
    /// involving this key (flagged, never clamped — design note §2).
    pub negative_stretch: bool,
}

/// A computed tuning curve: the target deviation from equal temperament,
/// in cents, for each of the 88 keys, defined on the **audible first
/// partial** f_1 (Rigaud Eq. 4; design note §1).
///
/// **Derived data — never persisted.** The curve is recomputed from the
/// profile on load (design note §9; the stale-`analysis.json` incident is
/// the standing proof). Deliberately does not implement `Serialize`.
#[derive(Debug, Clone)]
pub struct TuningCurve {
    /// d(m): cents deviation of the target f_1 from ET, per key,
    /// normalized so `cents[48]` (A4) is 0. The reference-pitch offset is
    /// carried separately in [`d_g`](Self::d_g).
    pub cents: [f32; 88],
    /// Global deviation d_g in cents (Rigaud Eq. 32's role): a vertical
    /// offset of the whole curve for non-440 reference pitches
    /// (d_g = 1200log₂(ref/440)). Default 0.
    pub d_g: f32,
    /// Per-key status flags.
    pub flags: [CurveKeyFlags; 88],
}

impl TuningCurve {
    /// Target audible first partial for a key:
    /// f_1^*(m) = f_{ET}(m) · 2^((d(m) + d_g)/1200).
    pub fn target_f1(&self, key_index: u8) -> f32 {
        let et = NOTES[key_index as usize].frequency;
        et * ((self.cents[key_index as usize] + self.d_g) / 1200.0).exp2()
    }

    /// Per-partial strobe reference frequencies (design note §7):
    /// f_n^*(m) = n f_0^*(m)√(1 + B_{raw} n²) with
    /// f_0^* = f_1^*/√(1 + B_{raw}).
    ///
    /// `b_raw` **must be the key's own measured B** — targets must match
    /// the physical string, or a correctly tuned partial shows a false beat
    /// (design note §5, D3). Any smoothed/fitted B is curve-input only.
    /// Fills `out` with partials n = 1, 2, … until Nyquist or capacity;
    /// returns the count.
    pub fn strobe_partials(&self, key_index: u8, b_raw: f32, out: &mut [f32]) -> usize {
        let f1 = self.target_f1(key_index);
        let f0 = f1 / (1.0 + b_raw).sqrt();
        let mut count = 0;
        for (i, slot) in out.iter_mut().enumerate() {
            let n = (i + 1) as f32;
            let f_n = n * f0 * (1.0 + b_raw * n * n).sqrt();
            if f_n >= NYQUIST_HZ {
                break;
            }
            *slot = f_n;
            count += 1;
        }
        count
    }
}

/// Represents a single musical note with its name and frequency.
#[derive(Debug, Clone)]
pub struct Note {
    /// Note name (e.g., "A4", "C#3", "Bb2")
    pub name: String,
    /// Frequency in Hz
    pub frequency: f32,
}

/// Statically computed notes for a standard 88-key piano (A0 to C8).
///
/// This lazy static contains all 88 piano keys with their corresponding
/// frequencies calculated using equal temperament tuning with A4 = 440 Hz.
/// The notes are computed once at startup for optimal performance.
pub static NOTES: Lazy<Vec<Note>> = Lazy::new(|| {
    const NOTE_NAMES: [&str; 12] = [
        "A", "A#", "B", "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#",
    ];
    let mut notes = Vec::with_capacity(88);

    for i in 0..88 {
        // A4 is the 49th key, which is index 48 in a 0-indexed loop.
        let frequency = 440.0 * 2.0_f32.powf((i as f32 - 48.0) / 12.0);

        let note_index = i % 12;
        let octave = (i + 9) / 12;
        let name = format!("{}{}", NOTE_NAMES[note_index], octave);

        notes.push(Note { name, frequency });
    }
    notes
});

/// Static map for quick note name to key index lookups.
///
/// This provides O(log n) lookup time for converting note names
/// (like "A4", "C#3") to their corresponding piano key indices.
pub static NOTE_MAP: Lazy<BTreeMap<String, u8>> = Lazy::new(|| {
    NOTES
        .iter()
        .enumerate()
        .map(|(i, note)| (note.name.clone(), i as u8))
        .collect()
});

/// Finds the closest musical note to a given frequency.
///
/// This function searches through all 88 piano keys to find the one
/// with the frequency closest to the input frequency. It's used for
/// automatic note detection in the tuner.
///
/// # Arguments
/// * `freq` - Input frequency in Hz
///
/// # Returns
/// * `(note_name, target_frequency)` - Closest note name and its target frequency
pub fn find_nearest_note(freq: f32) -> (String, f32) {
    let closest = NOTES
        .iter()
        .min_by(|a, b| {
            let diff_a = (a.frequency - freq).abs();
            let diff_b = (b.frequency - freq).abs();
            diff_a.partial_cmp(&diff_b).unwrap()
        })
        .unwrap(); // This is safe as NOTES is never empty.

    (closest.name.clone(), closest.frequency)
}

/// Finds a note's name and frequency by its 88-key piano index.
///
/// This function provides direct access to note information using
/// the piano key index (0-87, where 0 is A0 and 87 is C8).
///
/// # Arguments
/// * `key_index` - Piano key index (0-87)
///
/// # Returns
/// * `(note_name, frequency)` - Note name and frequency
pub fn find_nearest_note_by_index(key_index: u8) -> (String, f32) {
    let note = &NOTES[key_index as usize];
    (note.name.clone(), note.frequency)
}

/// Returns the 88-key piano index (0–87) of the note closest to `freq`.
///
/// Unlike [`find_nearest_note()`], this avoids a `String` allocation and is
/// suitable for use on the DSP hot path or in pipeline output types.
///
/// # Arguments
/// * `freq` - Input frequency in Hz
///
/// # Returns
/// * Piano key index (0 = A0, 87 = C8)
pub fn find_nearest_note_index(freq: f32) -> u8 {
    NOTES
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            let diff_a = (a.frequency - freq).abs();
            let diff_b = (b.frequency - freq).abs();
            diff_a.partial_cmp(&diff_b).unwrap()
        })
        .map(|(i, _)| i as u8)
        .unwrap() // Safe: NOTES is never empty.
}

/// Gets the 88-key piano index from a note name.
///
/// This function converts note names like "A4" or "C#3" to their
/// corresponding piano key indices for use in the GUI.
///
/// # Arguments
/// * `name` - Note name (e.g., "A4", "C#3", "Bb2")
///
/// # Returns
/// * Piano key index (0-87), defaults to 0 if note not found
pub fn get_key_index_from_name(name: &str) -> u8 {
    *NOTE_MAP.get(name).unwrap_or(&0)
}

/// MIDI note number of piano key 0 (A0). The 88-key compass spans MIDI
/// 21 (A0) … 108 (C8); key 48 (A4) is MIDI 69 = 440 Hz.
pub const MIDI_KEY_0: u8 = 21;

/// MIDI note number for an 88-key piano index (`key + 21`; A0 → 21, C8 → 108).
///
/// The single canonical key↔MIDI converter (MIDI Tuning Standard: A4 = 69 =
/// 440 Hz). External note sources — a MIDI keyboard, a DAW — map through here
/// so the whole codebase agrees on the numbering; the synth and curve layers
/// stay in native key-index units. Inverse of [`key_from_midi`].
pub fn midi_from_key(key_index: u8) -> u8 {
    key_index + MIDI_KEY_0
}

/// Piano key index (0 = A0 … 87 = C8) for a MIDI note number, or `None` when
/// the note is outside the 88-key compass (MIDI 21–108). Inverse of
/// [`midi_from_key`].
pub fn key_from_midi(note: u8) -> Option<u8> {
    note.checked_sub(MIDI_KEY_0).filter(|&k| k < 88)
}

/// Returns the expected physical inharmonicity coefficient (beta) for a given piano key.
///
/// Implements Rigaud's dual-exponential whole-compass model (Eqs. 7–8):
/// B(m) = e^(s_B·m + y_B) + e^(s_T·m + y_T), the sum of the bass- and
/// treble-bridge log-linear asymptotes, with m the MIDI note number. Here,
/// re-indexed to 1-indexed keys via m = n + 20 (A0: n = 1 ↔ m = 21):
///
///   B(n) = exp(-0.066n - 9.211) + exp(0.0926n - 11.788)
///
/// Constant provenance (faithfulness-audit-06):
/// * **Treble pair = the paper's universal fit**, verified exact:
///   (s_T, y_T) = (9.26e-2, −13.64) ⇒ 0.0926·(n+20) − 13.64 = 0.0926n − 11.788.
///   The paper fixes these across all pianos (after Young 1952).
/// * **Bass pair = OURS** — the paper defines (s_B, y_B) as *piano-specific*
///   free parameters (no universal value exists); ours (−6.6e-2, −7.891 in
///   MIDI domain) is a typical medium-piano default. Known domain limit: the
///   real upright's measured bass B runs 7–25× this default (ADR 0006) —
///   inherent to any fixed bass choice, which is why measured-B seeding
///   exists (gated off pending validation on a second instrument).
///
/// # Reference
/// 1. Rigaud, F., David, B., & Daudet, L. (2013). "A parametric model and estimation techniques
///    for the inharmonicity and tuning of the piano". JASA 133(5), pp. 3107-3118.
///    DOI: 10.1121/1.4802644 (Eqs. 7-8; treble universality §IV.)
pub fn get_expected_beta(key_index: u8) -> f32 {
    // Rigaud model uses a 1-indexed key number (A0 = 1).
    // key_index is 0-indexed (A0 = 0), so we offset by 1.
    let n = key_index as f32 + 1.0;
    (-0.066 * n - 9.211).exp() + (0.0926 * n - 11.788).exp()
}

/// Precomputed per-key discovery template: the predicted stiff-string partial
/// series the matcher scores observed peaks against.
#[derive(Debug, Clone)]
pub struct KeyProfile {
    /// Equal-temperament fundamental this template is centered on (Hz).
    pub f0_et: f32,
    /// Inharmonicity coefficient (B) used to stretch the partials.
    pub beta: f32,
    /// Predicted partial frequencies (Hz); valid entries are `[0..valid_partial_count]`.
    pub predicted_partials: [f32; MAX_PARTIALS],
    /// Number of partials that fall below Nyquist.
    pub valid_partial_count: usize,
}

impl KeyProfile {
    /// Builds a template from a fundamental and inharmonicity coefficient via the
    /// stiff-string law `f_n = n·f0·√(1 + B·n²)`, dropping partials above Nyquist.
    pub fn new(f0_et: f32, beta: f32) -> Self {
        let mut predicted_partials = [0.0; MAX_PARTIALS];
        let mut valid_partial_count = 0;

        for n in 1..=MAX_PARTIALS {
            let n_f32 = n as f32;
            let f_n = n_f32 * f0_et * (1.0 + beta * n_f32 * n_f32).sqrt();
            if f_n < NYQUIST_HZ {
                predicted_partials[n - 1] = f_n;
                valid_partial_count += 1;
            } else {
                break;
            }
        }

        Self {
            f0_et,
            beta,
            predicted_partials,
            valid_partial_count,
        }
    }

    /// The Rigaud-prior template for a key: equal-temperament center, expected B.
    pub fn prior(key_index: u8) -> Self {
        Self::new(
            NOTES[key_index as usize].frequency,
            get_expected_beta(key_index),
        )
    }

    /// Builds a template from a measured key, using its measured inharmonicity B in
    /// place of the prior. Returns `None` when B is absent or non-physical (so the
    /// caller keeps the prior).
    ///
    /// The template is centered on equal temperament, not the measured `f0`: B is the
    /// tuning-invariant string-shape parameter, whereas a stored `f0` goes stale as the
    /// string is tuned. Stage-B refinement absorbs the live pitch offset.
    pub fn from_measurement(m: &KeyMeasurement) -> Option<Self> {
        let beta = m.calculated_b?;
        if !beta.is_finite() || beta <= 0.0 {
            return None;
        }
        let f0_et = NOTES.get(m.key_index as usize)?.frequency;
        Some(Self::new(f0_et, beta))
    }
}

/// Builds the full 88-key prior template table.
pub fn build_default_profiles() -> Box<[KeyProfile; 88]> {
    let v: Vec<KeyProfile> = (0..88).map(|k| KeyProfile::prior(k as u8)).collect();
    Box::new(v.try_into().unwrap_or_else(|_| unreachable!()))
}

/// Abramowitz & Stegun 7.1.26 error-function approximation (|err| < 1.5e-7).
///
/// `f32` twin of `algorithms::rigaud::erf`, kept separate on purpose:
/// this one feeds the discovery-side Railsback curve, which stays
/// bit-identical against the pre-tuning-curve baselines; the curve layer
/// needs the `f64` precision.
fn erf(x: f32) -> f32 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = ((((1.061_405_4 * t - 1.453_152_1) * t + 1.421_413_8) * t - 0.284_496_72) * t
        + 0.254_829_6)
        * t;
    sign * (1.0 - poly * (-x * x).exp())
}

/// Canonical Railsback stretch (cents vs equal temperament) for each of the 88
/// keys, from the Rigaud (2011/2013) inharmonicity-coupled octave-stretch model
/// with mean parameters (type-octave K≈4.51, m0≈64, α≈24; A4 anchored at 440).
///
/// Discovery currently scores templates at raw ET, which handicaps every note by
/// its stretch (worst in the extreme treble). Centering the per-key template on
/// this expected tuned pitch removes that systematic handicap; Stage-B refinement
/// then absorbs the residual per-instrument / pitch-raise offset. This is the same
/// model the synthetic generator uses, so the engine reference and the synthetic
/// dataset stay consistent.
///
/// # Reference
/// Rigaud, F., David, B., & Daudet, L. (2011). "A parametric model of piano
/// tuning". Proc. DAFx-11. (Eqs. 8, 12–14.)
pub fn railsback_stretch_curve() -> [f32; 88] {
    let b: [f32; 88] = core::array::from_fn(|k| get_expected_beta(k as u8));
    let et = |k: usize| -> f32 { NOTES[k].frequency };
    // Type-octave amount ρ(key), mean fit (decreasing bass→treble).
    let rho = |key: usize| -> f32 {
        let m = key as f32 + 21.0; // MIDI index
        (4.51 / 2.0) * (1.0 - erf((m - 64.0) / 24.0)) + 1.0
    };

    let mut f0 = [0.0f32; 88]; // flexible-string fundamentals
    let mut f1 = [0.0f32; 88]; // measured first partials = f0·√(1+B)
    // Anchor A4 (key 48): f1=440 ⇒ f0 = 440/√(1+B).
    f0[48] = 440.0 / (1.0 + b[48]).sqrt();
    for a in [60usize, 72, 84] {
        let r = rho(a);
        f0[a] = 2.0 * f0[a - 12] * ((1.0 + b[a - 12] * 4.0 * r * r) / (1.0 + b[a] * r * r)).sqrt();
    }
    for a in [36usize, 24, 12, 0] {
        let r = rho(a + 12);
        f0[a] =
            f0[a + 12] / (2.0 * ((1.0 + b[a] * 4.0 * r * r) / (1.0 + b[a + 12] * r * r)).sqrt());
    }
    for a in [0usize, 12, 24, 36, 48, 60, 72, 84] {
        f1[a] = f0[a] * (1.0 + b[a]).sqrt();
    }
    // Semitone fill inside each A–A octave (Eq. 12–14).
    let mut last_lambda = 0.0f32;
    for a in [0usize, 12, 24, 36, 48, 60, 72] {
        let b_sum: f32 = (1..=12).map(|p| b[a + p]).sum();
        let lambda = 24.0 * (f1[a + 12] / (2.0 * f1[a])).ln() / b_sum.max(1e-9);
        last_lambda = lambda;
        for p in 1..12 {
            f1[a + p] = f1[a + p - 1] * (2.0 + lambda * b[a + p]).powf(1.0 / 12.0);
        }
    }
    for k in 85..88 {
        f1[k] = f1[k - 1] * (2.0 + last_lambda * b[k]).powf(1.0 / 12.0);
    }

    core::array::from_fn(|k| 1200.0 * (f1[k] / et(k)).log2())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midi_mapping_round_trips() {
        assert_eq!(midi_from_key(0), 21); // A0
        assert_eq!(midi_from_key(48), 69); // A4 = 440 Hz (MIDI Tuning Standard)
        assert_eq!(midi_from_key(87), 108); // C8
        for k in 0u8..88 {
            assert_eq!(key_from_midi(midi_from_key(k)), Some(k));
        }
        assert_eq!(key_from_midi(69), Some(48));
        assert_eq!(key_from_midi(20), None); // below A0
        assert_eq!(key_from_midi(109), None); // above C8
    }

    /// A measurement of `key`, tagged with its provenance and a marker `f0`
    /// so the tests can tell entries apart.
    fn m(key: u8, f0: f32, captured_in_auto: bool) -> KeyMeasurement {
        KeyMeasurement {
            key_index: key,
            measured_f0: f0,
            partials: Vec::new(),
            calculated_b: Some(1e-4),
            last_captured: format!("{f0}"),
            captured_in_auto,
        }
    }

    /// The active-entry rule (ADR 0006 item 3 over a list): an auto-mode
    /// capture is retained for review but never displaces a manual one,
    /// however recent it is.
    #[test]
    fn untrusted_never_displaces_trusted() {
        let mut p = InharmonicityProfile::default();
        p.record(m(5, 100.0, false)); // manual
        p.record(m(5, 200.0, true)); // auto, arrives later
        assert_eq!(p.active(5).unwrap().measured_f0, 100.0);
        assert_eq!(p.measurements[&5].len(), 2, "the auto entry is retained");

        // A newer *manual* capture does take over.
        p.record(m(5, 300.0, false));
        assert_eq!(p.active(5).unwrap().measured_f0, 300.0);

        // With no manual entry at all, the newest untrusted one is active.
        let mut q = InharmonicityProfile::default();
        q.record(m(7, 10.0, true));
        q.record(m(7, 20.0, true));
        assert_eq!(q.active(7).unwrap().measured_f0, 20.0);
        assert!(q.active(9).is_none());
    }

    /// Eviction bounds the file without ever dropping the entry consumers read.
    #[test]
    fn eviction_never_drops_the_active_entry() {
        let mut p = InharmonicityProfile::default();
        p.record(m(3, 1.0, false)); // the only trusted entry: stays active
        for i in 0..(MAX_MEASUREMENTS_PER_KEY as u32 * 2) {
            p.record(m(3, 100.0 + i as f32, true));
        }
        assert_eq!(p.measurements[&3].len(), MAX_MEASUREMENTS_PER_KEY);
        assert_eq!(
            p.active(3).unwrap().measured_f0,
            1.0,
            "the trusted entry survived eviction"
        );
    }

    /// Undo pops the appended entry, and a key that never had one disappears
    /// entirely — the shape the curve's `active_entries` expects.
    #[test]
    fn undo_last_restores_the_previous_shape() {
        let mut p = InharmonicityProfile::default();
        p.record(m(2, 1.0, false));
        p.record(m(2, 2.0, false));
        assert_eq!(p.undo_last(2).unwrap().measured_f0, 2.0);
        assert_eq!(p.active(2).unwrap().measured_f0, 1.0);
        assert_eq!(p.undo_last(2).unwrap().measured_f0, 1.0);
        assert!(
            !p.measurements.contains_key(&2),
            "a key with no measurements must not linger as an empty list"
        );
        assert!(p.undo_last(2).is_none());
    }

    /// `remove` reaches any entry, not just the tail, and holds `undo_last`'s
    /// invariant: emptying a key removes the key.
    #[test]
    fn remove_reaches_any_entry_and_empties_the_key() {
        let mut p = InharmonicityProfile::default();
        p.record(m(6, 1.0, false));
        p.record(m(6, 2.0, true));
        p.record(m(6, 3.0, true));

        // The middle entry — the one `undo_last` can never reach.
        assert_eq!(p.remove(6, 1).unwrap().measured_f0, 2.0);
        assert_eq!(p.measurements[&6].len(), 2);
        assert_eq!(
            p.active(6).unwrap().measured_f0,
            1.0,
            "trusted still active"
        );

        assert!(p.remove(6, 5).is_none(), "index past the end");
        assert!(p.remove(9, 0).is_none(), "unmeasured key");

        p.remove(6, 0).unwrap();
        p.remove(6, 0).unwrap();
        assert!(
            !p.measurements.contains_key(&6),
            "a key with no measurements must not linger as an empty list"
        );
        assert!(p.active(6).is_none());
    }

    /// A v0 profile (one measurement per key, no identity or settings) still
    /// opens, and lands on the current defaults.
    #[test]
    fn v0_profiles_migrate_on_load() {
        let dir = std::env::temp_dir().join(format!("inh-v0-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("old-upright.json");
        std::fs::write(
            &path,
            r#"{"measurements":{"4":{"key_index":4,"measured_f0":55.0,
               "partials":[],"calculated_b":0.0004,"last_captured":"1"}}}"#,
        )
        .unwrap();

        let loaded = InharmonicityProfile::from_file(&path).unwrap();
        assert_eq!(loaded.measurements[&4].len(), 1);
        assert_eq!(loaded.active(4).unwrap().measured_f0, 55.0);
        // Pre-flag entries are untrusted, so they can never feed the curve.
        assert!(loaded.active(4).unwrap().captured_in_auto);
        // Named after its file, and carrying today's defaults.
        assert_eq!(loaded.identity.name, "old-upright");
        assert_eq!(
            loaded.settings.nhwrsf_threshold, DEFAULT_NHWRSF_THRESHOLD,
            "a v0 file has no thresholds; it must adopt the defaults"
        );
        assert_eq!(loaded.settings.reference_mode, ReferenceMode::Curve);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Saving round-trips through the current schema, leaves no temp file
    /// behind, and stamps `modified`.
    #[test]
    fn save_is_atomic_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("inh-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("upright.json");

        let mut p = InharmonicityProfile::new("Front room upright");
        p.identity.serial = Some("H1234567".into());
        p.identity.kind = InstrumentKind::Guitar;
        p.settings.engine = EngineChoice::GiordanoMean;
        p.settings.reference_mode = ReferenceMode::Et;
        p.record(m(11, 61.7, false));
        p.to_file(&path).unwrap();

        assert!(
            !dir.join("upright.json.tmp").exists(),
            "temp file left behind"
        );
        assert!(p.modified > 0, "save must stamp `modified`");

        let back = InharmonicityProfile::from_file(&path).unwrap();
        assert_eq!(back.version, PROFILE_SCHEMA_VERSION);
        assert_eq!(back.identity.name, "Front room upright");
        assert_eq!(back.identity.serial.as_deref(), Some("H1234567"));
        assert_eq!(back.identity.kind, InstrumentKind::Guitar);
        assert_eq!(back.settings.engine, EngineChoice::GiordanoMean);
        assert_eq!(back.settings.reference_mode, ReferenceMode::Et);
        assert_eq!(back.active(11).unwrap().measured_f0, 61.7);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
