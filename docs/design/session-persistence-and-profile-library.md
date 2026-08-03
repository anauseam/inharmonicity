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

**(b) Repeats are the raw material of review.** They are kept to *catch
outliers, not to average*: ADR 0009 measured σ_lnB(n) = max(19.3·n⁻³, 0.0035),
so bass/mid repeat spread is ~0.4 % and averaging helps essentially only where
the curve already absorbs the noise through inverse-variance ln-B shrinkage, and
where the strobe displays n = 1 whose target is identically B-immune (R4).

This was originally written as **automatic** disagreement detection — "two
captures differing by more than σ_lnB predicts identify a bad one". **That is
refuted** (§5.3): the repeats survive as what the inspector *shows a human*, key
by key, which is what shipped.

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
is for (§5.2, shipped 2026-08-01). §5.3 records the second attempt at an
automatic criterion, and why it also failed.

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

### 5.2 The measurement inspector *(tuner-gui, 2026-08-01)*

The surface §4 assumes. A settings panel — reviewing is not tuning — showing
every retained measurement of one key with its epoch, provenance, partial count
and B, the curve's verdict on that key, and three remedies: **drop one entry**
(any entry, not just the tail — `InharmonicityProfile::remove`, since a repeat
that looks wrong later is rarely the newest), **re-measure** (the ordinary
manual path: select, arm, capture — no second capture route), or leave it.

Two couplings are load-bearing and easy to get wrong:

- **A dropped entry keeps its dump, and an undone one does not.** The asymmetry
  is deliberate. Undo means *this capture should not have been taken* — a
  mis-strike, the wrong key selected, a phone ringing — so its audio has no
  later use and leaving debris from an unwanted action would surprise. A drop
  means only *this measurement is not trusted*, which is a claim about the
  estimate, not the recording: piano #2's deep bass is the standing case, where
  the audio was genuine and only the cached analysis was wrong. Keeping the
  audio is also what makes §4's "a poisoned profile is rebuildable offline"
  true for the entries a user actually rejects. Disk is bounded by the
  retention policy (TODO.md), not by deleting on rejection.
- **A drop consumes one undo of that key.** Undo pops the key's *tail*; leaving
  the history intact would let a later undo discard a different measurement
  than the one it was recorded for.

**The curve is the key picker.** The panel selects a key by clicking the curve
plot rather than from a list of note names: the plot already shows which keys
are measured (dots), which are doubted (✗) and which are gaps, so choosing what
to review is the same act as reading the curve, and every key is reachable —
including unmeasured ones, which a measured-keys list cannot offer a
re-measure. `CurvePlot` gained an opt-in `on_select` for this rather than a
second widget: the gallery thumbnails pass nothing and stay inert, so there is
one rendering of a curve in the app, not two that can drift.

**A flagged key carries its remedies where the user already is.** The strobe
panel's ✗ offers *Re-measure* (the ordinary capture flow, so it belongs on the
measuring surface) and *Review measurements* (which opens this panel on that
key, because dropping an entry means choosing between a key's repeats and that
needs the list). Sending the user to hunt through settings for either would be
the slow path.

The red-✗ marks on the curve plot, the keyboard and the strobe panel are the
same verdict rendered in three places, resolved once in `advisory.rs`. Which
flags earn a ✗ is measured, and is argued in the strobe design note §5.6.

### 5.3 Rejected: the σ_lnB repeat-disagreement detector

Schema v1 retains repeats partly to support an automatic check — flag a capture
that disagrees with its key's other captures by more than
σ_lnB(n) = max(19.3·n⁻³, 0.0035) predicts (ADR 0009). **Measured on piano #2's
595 repeats through `regenerate_partials`, it does not work**, in two
independent ways:

- **It fires on well-behaved data.** A 3σ pairwise rule flags **29 of 88 keys**;
  8σ still flags 8. Nothing in that set is a bad capture — these are the very
  repeats ADR 0009's σ was fitted to. The model describes the *median* key of an
  n-bin, while individual keys sit up to 3.5× (bass), 7.9× (mid) and 23× (treble)
  above their model σ, so the statistic has no calibrated tail.
- **It is weakly correlated with consequence.** Expressed as movement of the
  strobe target (worst partial n ≤ 8), bass repeats disagree by a median
  **0.10 ¢** and mid by 0.39 ¢ — below anything the readouts resolve — while the
  treble's median **197 ¢** is the known information floor that
  `curve_b_fallback` already marks and the ln-B shrinkage already absorbs. Of
  the 13 mid keys whose repeats do move the target past 1 ¢, the 3σ rule catches
  **4**; it misses F5 (8.2 ¢, z = 2.0) and A#5 (6.6 ¢, z = 0.3) while firing on
  a dozen keys that move by ≤ 0.1 ¢.

So there is no operating point: loose enough to catch the real movers and it
flags a third of the compass; tight enough to stay quiet and it only reports
disagreements a glance would already catch. A detector thresholded in *target
cents* rather than in σ_lnB units is the shape that could work, and is not
designed here — the inspector shows the repeats and the human decides, which is
§4's position anyway.

**What repeats are actually worth, then.** The same measurement that refutes the
detector points at the real use: those per-key departures from σ_lnB(n) — 3.5×,
7.9×, 23× — are not noise in the comparison, they are the *precision of that
key's B*, and the curve currently guesses it from partial count alone. Feeding a
measured σ_m into the shrinkage weight w = σ_p²/(σ_p² + σ_m²) corrects how much
the curve trusts each key, which is a systematic error rather than the ~0.05 ¢
averaging would buy. Tracked in TODO.md; it needs a small-sample estimator,
since k = 2–3 makes a raw sample SD nearly worthless.

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

Closed since: the **per-key inspector** and **flagged-key ✗ styling** (§5.2,
§5.6 of the strobe note) — what makes §4's "the human is the gate" true in
practice rather than only in principle — and the **repeat-disagreement
detector**, measured and rejected (§5.3).

Open, both in TODO.md: **export/import from an arbitrary path**; and **dump
retention** — nothing prunes `diagnostics/`, which grows without limit, and the
inspector now deletes dumps one at a time without ever showing how many there
are.
