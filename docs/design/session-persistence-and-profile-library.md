# Session persistence & the profile library — design note

**Status:** Implemented 2026-07-31. Covers what persists between runs: the
profile schema (`tuner-core`), and where profiles and dumps live plus when they
are written (`tuner-gui`). Cross-references: ADR 0006 (the provenance rule this
extends), ADR 0009 (the σ model behind repeat retention), `docs/internals/01`
(crate boundaries), `06-capture-sets.md` (the capture sets, which are *not*
what §3 moves).

The two halves are separable and marked as such: **§1–2 are `tuner-core`** and
**§3–5 are `tuner-gui`**. When the crates split, §3–5 travel with the frontend.

Standing project constraints apply: `tuner-core` stays headless, the six
crossings are not widened, commit only when asked.

---

## 0. What was wrong

Three gaps, one theme — nothing survived a restart.

1. **Nothing was auto-saved.** `to_file` ran only from the *Save Profile*
   button, so closing the app, a crash, or the CPAL/ALSA shutdown path discarded
   every measurement since the last manual save. A full-compass pass is hours.
2. **A saved profile was not loaded at launch.** `TunerApp` started from
   `InharmonicityProfile::default()`. (The core-side seeding in
   `AudioPipeline::new` is a *different* mechanism — discovery templates, gated
   behind `APPLY_MEASURED_B_TO_DISCOVERY = false` for a measured reason,
   ADR 0006. Not conflated, not touched.)
3. **Calibration was lost every run** — the thresholds lived only in
   `atomics.config.*`.

---

## 1. Schema v1 — a key holds a list *(tuner-core)*

`measurements: BTreeMap<u8, Vec<KeyMeasurement>>`. Captures append.

Two independent requirements land on this one shape.

**(a) An untrusted measurement must never overwrite a trusted one.** Auto mode
re-arms and captures unattended, and ADR 0006 item 3 warns that an octave
false-lock makes MAT confidently measure the wrong series and file it under the
wrong key. That damage was memory-only until a manual save; autosave would have
made it permanent. Appending plus a resolution rule fixes it structurally,
rather than by adding a gate.

**(b) Repeats are the raw material of a bad-capture detector.** They are kept to
*catch outliers, not to average*: ADR 0009 measured
σ_lnB(n) = max(19.3·n⁻³, 0.0035), so bass/mid repeat spread is ~0.4 % and
averaging helps essentially only where the curve already absorbs the noise
through inverse-variance ln-B shrinkage, and where the strobe displays n = 1
whose target is identically B-immune (R4). What repeats *do* buy is
**disagreement detection** — two captures differing by more than σ_lnB predicts
identify a bad one. That comparison must be made **within** a provenance class;
mixing trusted and untrusted captures corrupts the σ it rests on.

**The active-entry rule.** `InharmonicityProfile::active` returns the **newest
trusted entry, else the newest** — ADR 0006 item 3 expressed over a list. Every
consumer reads through it: `CurveInput::from_profile`, the crossing-#4 template
push, and the strobe's raw-B source. Verified behaviour-preserving on the real
88-key profile, which migrates to 88 single-entry lists and still yields
`measured_count = 88`.

Bounded at `MAX_MEASUREMENTS_PER_KEY = 8`, evicting the oldest entry that is not
the active one — the file is rewritten on every capture, and a few per key is
all any σ comparison needs.

### 1.1 Rejected: a reserved per-string field

A `course: Option<u8>` was briefly added on the argument that `key_index` is not
a sufficient identity on every instrument this tuner targets — a fretted note is
producible on several strings of different gauge, hence different B, and a piano
note is 1–3 strings — and that reserving it avoided a second migration.

**The migration argument was false.** An additive optional field with
`#[serde(default)]` costs nothing to add later; old files simply load with
`None`. That is exactly how `captured_in_auto` was added. So the field bought
nothing and cost something real: an unused field in a *persisted* type is not
free like an unused private one, because it is written into every user's file as
`"course": null` and becomes a shape we would have to keep honouring.

Removed. The underlying observation — key index is not a sufficient identity on
fretted or multi-course instruments — is recorded in TODO.md, which is where an
unbuilt need belongs.

---

## 2. Per-instrument settings *(tuner-core)*

The split is by **physical units**, not by convenience:

- The noise floor is an **absolute RMS in the room's own units**, moving with
  mic, gain, room and HVAC ⇒ **rig state**, remeasured at launch as before.
- NHWRSF is normalized by Σ|X| and NINOS² is a dimensionless sparsity ratio
  (N·(ℓ²/ℓ¹)², audit-05) ⇒ both are **level-independent** and describe
  *instrument character* — attack sharpness, spectral sparsity ⇒ they live in
  `ProfileSettings` and travel with the instrument between machines.
- Manual recalibrate buttons stay regardless: level-independent is not
  noise-independent.

**Engine and reference mode also persist**, so reopening an instrument
reproduces the targets it was actually tuned to. Without it, tuning instrument B
with a different engine and reopening A would silently shift every one of A's
targets. Reference mode is the sharper case: the curve engines are piano models
(Rigaud's B prior is a piano fit, Giordano dissonance, Railsback), so a guitar
profile reopening in `Curve` mode would be wrong every time — `reference_mode`
decides whether `engine` is consulted at all.

This moved `EngineChoice` and `ReferenceMode` from `tuner-gui` into
`tuner-core::models`, since the profile that persists them lives there, and
replaced `EngineChoice::resolve(bundle)` with `CurveBundle::curve(choice)` —
keeping the bundle's resolution with the bundle rather than creating a
`models → worker` cycle.

---

## 3. Where files live *(tuner-gui)*

`PROFILE_PATH` was a bare relative name, so launching from a different directory
silently opened a different — or empty — profile. Survivable while saving was a
deliberate button press; not survivable with autosave.

| What | Where | Why |
| --- | --- | --- |
| Profiles | `data_dir()/profiles/` | User documents: small, and roaming them across machines is a feature. |
| App settings | `config_dir()/settings.json` | Config, by definition. |
| Capture dumps | `data_local_dir()/diagnostics/` | Large (raw audio per capture). On Windows `data_dir` is the *roaming* profile, which a domain login would sync over the network; `data_local_dir` is machine-local. On Linux and macOS the two are the same path. |

`directories::ProjectDirs` rather than hand-rolled XDG: the XDG variables are
unset on macOS and absent on Windows, where a hand-rolled fallback picks a
non-native path or none at all, and pre-built binaries are a planned deliverable.

**Dumps moved too, and that was a correction.** The first pass kept them
CWD-relative on the argument that the offline harnesses depend on it. That
conflated two different paths: the *app writing* dumps and the *harnesses
reading* capture sets. The harnesses take a root argument (the two that
hardcoded it now do as well), and the capture sets in
`docs/internals/06-capture-sets.md` stay exactly where they are. Meanwhile a
released binary has no useful CWD — a macOS `.app` launched from Finder gets
`/`, a Windows exe gets its install directory — so dumps would have landed
somewhere unwritable, and the old code swallowed the failure silently. That
would have quietly voided the offline-rebuild argument §4 depends on. The dump
root is now supplied by the frontend (`Option<PathBuf>`, `None` = write none, so
an embedded host can opt out) and a create failure logs.

**Identity is the load-bearing half of §3.** Autosave plus resume-at-launch
create a failure the manual flow did not have: tune instrument A, walk to
instrument B, and an auto-loaded A quietly absorbs B's measurements into A's
file — last-wins per key, irreversibly interleaved, with a one-deep `.bak` that
does not cover it. The guard is not a warning dialog. It is that the profile
**names its instrument**, that the name is **always on screen**, and that
starting a new one is a **visible action**. `InstrumentIdentity` carries name,
family, make, model, serial, form, owner and notes — the serial being the only
field that identifies an instrument unambiguously.

---

## 4. When it is written *(tuner-gui)*

Save on every measurement **and after every undo**, so the file always mirrors
in-memory state.

**Why that is safe without an acceptance gate.** A measurement is *evidence
about the instrument*; the remedy for a bad one is to re-measure that key; and
every capture also writes a full diagnostics dump, so a poisoned profile is
rebuildable offline through `regenerate_partials`.

**There is no "accepted measurement" and none was invented.** The Worker emits
`WorkerOutput::Measurement` unconditionally. The only quality signal, MAT's
`b_confidence`, is deliberately absent from `KeyMeasurement` because ADR 0006
item 4 demoted it to a diagnostic — it measures self-consistency, not accuracy —
and it was **not** resurrected as an autosave gate. With no trustworthy
automatic criterion the human is the gate, which is what the per-key inspector
is for (TODO.md, still unbuilt — the main gap this note leaves open).

Three mechanics:

- **Atomic write.** Temp file *beside* the target, then `rename` — a sibling
  deliberately, since `rename` is only atomic within one filesystem. Writing in
  place means a crash mid-write truncates the profile, which is exactly the loss
  autosave exists to prevent.
- **A `.bak` before the first write of a session**, giving a rollback point that
  does not depend on the in-memory undo stack.
- **Interaction-rate edits coalesce.** Text inputs emit per character and
  sliders per step, and the profile is a single ~140 KB document; writing per
  event would rewrite the whole file eight times to type a name. Identity fields
  and the two threshold sliders wait for a quiet delay, flushed by the tick loop
  and on library-close and exit. Measurements and undo never take this path.

**Undo stays session-scoped**, and `Load Profile` still clears it. Undo
*deletes the undone capture's diagnostics dump*, so a restored cross-session
entry would be a promise the files can no longer keep; 100 full
`KeyMeasurement`s would roughly double a file now rewritten on every capture;
and after a restart nobody remembers what they meant to undo. The three
mechanisms divide by timescale: **undo** for the mistake just made, the
**inspector** for "this key looks wrong" later, the **`.bak`** for catastrophe.

Undo also got simpler: a capture appends, so undoing is popping the entry back
off (`undo_last`), and the history stores keys rather than displaced values.

---

## 5. The library browser *(tuner-gui)*

A **settings panel**, not a measurement-time control — managing instruments is a
settings task. What stays on the measuring surface is the *name* of the open
instrument, beside the main title: knowing which profile is being written to is
what makes resume-at-launch plus autosave safe (§3), and that is a different
question from managing the collection.

New / open / resume-last, sortable by recent / name / make, searchable across
every identifying field including the serial, with duplicate and delete per row
(never the open one). Not a native file dialog: an OS picker would need `rfd`,
a real system dependency on Linux, and buys only open-from-anywhere — deferred
in TODO.md until that need is real.

### 5.1 What the field does (survey, 2026-07-31)

Four apps, one convergent answer. **PianoMeter** (the most modern) saves tuning
files **automatically**, identifies them by manufacturer / model / serial /
type / owner entered *after the fact*, and loads via a list sortable by name,
manufacturer and **date accessed** with a search bar. **Verituner** ships the
three entry verbs exactly — new tuning, open a saved tuning, resume the last one
open — and saves its A440 offset *in* the tuning file while a reusable Style
lives app-wide. **TuneLab Pro** keeps files in a `TuneLab` folder with
user-made category sub-folders, for repeat tunings. **CyberTuner** keeps
per-piano records with iCloud sync.

Carried into this note: automatic saving is the norm rather than a risk (§4);
identity is rich and entered late, so no field may be required to save (§3); a
browsable sortable searchable list beats a picker (§5); and "date accessed" is a
first-class sort order, hence `last_opened` tracked separately from `modified`.

Sources: PianoMeter support + iOS change log (pianometer.com/support,
pianometer.com/2024/04/02/pianometer-ios-change-log); Verituner features
(veritune.com/features.html); TuneLab "Managing Tuning Files"
(tunelab-world.com/managefiles.html); CyberTuner support
(cybertuner.com/irctsupport).

---

## 6. Consequences and what stays open

- Every capture costs one ~140 KB serialize-and-write, at human cadence,
  bounded by `MAX_MEASUREMENTS_PER_KEY`.
- Offline tooling reading profile JSON goes through the new shape;
  `diagnose_engine --profile` (hence `scripts/test_engine_all.py --profile`) was
  updated, and the v0 loader keeps every pre-existing file readable.
- The legacy `./tuning_profile.json` is **copied**, not moved, into the library
  on first launch when the library is empty — the original stays where the
  harnesses expect it.

The dump root is passed to the Worker at construction, so changing it needs a
restart. When a setting for it exists, the mechanism is `WorkerJob::SetDumpDir`
on crossing #6 — a new variant on a channel explicitly designed to take them,
not a new crossing, since that leg is a crossbeam channel and can carry a
`PathBuf`.

Open, all in TODO.md: the **per-key inspector** and **flagged-key ✗ styling**
(what makes §4's "the human is the gate" true in practice rather than only in
principle); the **repeat-disagreement detector** over the σ_lnB model once a
session has produced real repeats; **export/import from an arbitrary path**; and
**dump retention** — nothing prunes `diagnostics/`, which grows without limit.
