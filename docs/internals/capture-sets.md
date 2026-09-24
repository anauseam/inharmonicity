# Capture Sets — the validation data

Every empirical claim in this project rests on sets of recordings in the app's
capture format: our own captures, and external corpora converted to it. They
are kept on disk and **never committed** (`.gitignore`: `diagnostics/`,
`diagnostics_piano*/`) — large, regenerable in principle, instrument-specific.
This file records what each set is, what state the instrument was in, and the
rules for consuming them, because none of that is recoverable from the audio.
A set made from an external corpus is described by the report that prepared
it; this file holds its row.

The sets live in the working tree, gitignored; the harnesses that read them
take the directory as an argument (`cargo lab mat validate diagnostics_piano2`),
defaulting to `diagnostics`. The *app* writes new dumps elsewhere: a per-user
directory chosen by the frontend (a released binary has no useful working
directory), one subdirectory per instrument named for the opaque `identity.id`
its profile carries, so renaming an instrument moves nothing, each holding an
`instrument.json` naming whose captures they are. Dumps written before
2026-08-15 sit in that root directly. The layout is `library.rs`'s.

## The sets

| Directory | Instrument | Captures | Keys | Captured for |
| --- | --- | --- | --- | --- |
| `diagnostics/` | Steel-string guitar, standard tuning | 6 | 6 (E2 A2 D3 G3 B3 E4 = keys 19/24/29/34/38/43) | The guitar-strobe frequency audit; the cross-instrument check for anything piano-tuned |
| `diagnostics_piano_1/` | Upright piano #1 | 87 | 87 (one per key; **key 21 / F#2 is absent**) | The original discovery/TWM validation set — the "87 captures" every lock-accuracy number cites |
| `diagnostics_piano2/` | Upright piano #2 | 595 | 88 (≥ 5 repeats per key) | the repeat-capture noise decomposition (report 0009); the project's first two-instrument evidence base |
| the app's dump directory, profile `Piano2_extended` | Upright piano #2 in its **as-found** state before tuning | 555 | a full-compass open pass at ~4 repeats per key, plus eight complete isolation sets — C2, F2, C3, D3 (bichords) and A#3, A4, C5, C6 (trichords) — each with every solo and the open note repeated | the mute-isolation set (recorded 2026-08-15/16): the unison panel against per-string truth (report 0014) |

Piano **1** has an underscore before its index and piano **2** does not, which
is why `.gitignore` uses `diagnostics_piano*/`: an earlier
`diagnostics_piano_*/` silently failed to ignore 315 MB of piano-2 captures.

**The mute-isolation set is not interchangeable with the other three.** It is
the same note recorded once per string in isolation (the others damped with a
mute) and once open, because nothing in a capture of a multi-strung note can
separate a real unison from one string beating against itself (report 0012 §8,
report 0013 D3). What it established is report 0014's. Its data rules, on top
of the ones below:

- **The declaration is the set.** Which strings sounded is the operator's
  declaration, latched onto the capture when its audio begins and written to
  `analysis.json` as `metadata.sounding_strings` (`models::SoundingStrings`),
  which `cargo lab mat regen` carries through. It states both how many strings
  the key has (`on_key`) and which sounded, since without the count one string
  of two and one of three are the same record; `on_key` is declared, not
  derived, because where a piano's single/bi/trichord breaks fall is
  instrument-specific and this set is the first data recording them (on
  piano #2, D3 is still a bichord while A#3 is a trichord). **String 1 is the
  leftmost string as the tuner faces the instrument.** `null` means undeclared
  — what every capture in the first three sets carries and what ordinary
  tuning writes — and a `null` capture is not part of this set whatever
  directory it sits in. The app offers the declaration only in Manual mode,
  since it describes the strings of a named key.
- **Screen bass solos on within-key `B` agreement, not on partial count.** A
  muted bass string is quiet enough that MAT can lock onto something else
  entirely; the count alone over-rejects, while the failures announce
  themselves in `B`, disagreeing with their siblings by 30–90× where good
  captures agree to a fraction of a percent. Report 0014 §1 is the screen;
  unscreened, C2's f₀ repeatability reads 56.6 ¢ instead of 0.078 ¢.
- **A solo capture measures one string, not the note.** The profile retains
  it but never treats it as the key's measurement (`KeyMeasurement::is_trusted`
  disqualifies a declared non-open capture as it does an auto-mode one), so a
  key measured *only* in isolation reads as unmeasured: a hole in the curve,
  not a wrong point in it.
- **The profile is not the set.** Retention is bounded per key — 8 trusted
  entries plus a reserve of 4 that no consumer reads — so an isolation key
  keeps the open captures it has room for and *one row per solo
  configuration*, not every repeat. Eviction drops a profile entry, never a
  dump: read the set from the dumps, through `cargo lab mat regen`, never the
  cached `analysis.json`.
- Validation-only like the others, and doubly so until a second instrument's
  worth of it exists.

## Instrument state — read this before interpreting any number

**Both uprights are out of tune.** That is the point, not a defect: the app
targets the out-of-tune and pitch-raise regime, and captures of a freshly tuned
instrument would test the easy case only. It does constrain what the sets can
prove:

- There is **no trusted `B` reference** on either instrument — the standing
  gate behind `APPLY_MEASURED_B_TO_DISCOVERY` and the report 0006 measured-B
  pathway (report 0006, `pipeline.rs`).
- Lock-accuracy scores are *relative*: a config scoring 77/87 beats one
  scoring 74/87 **on this instrument**, and neither is an accuracy claim about
  pianos.

Measured deviation from equal temperament (median per key, from
`analysis.json`; keys whose seeds are known-bad excluded):

| Set | median | p10 … p90 | full range |
| --- | --- | --- | --- |
| piano #1 | +4.4 ¢ | −7.1 … +17.6 | −24 … +44 |
| piano #2 | −0.8 ¢ | −12.0 … +3.5 | (see defect below) |
| guitar | −2.7 ¢ | −8.9 … −0.8 | −9 … −1 |

**Do not read those as tuning quality.** A correctly tuned piano deviates from
ET by design — the Railsback stretch reaches tens of cents at both extremes —
so cents-vs-ET conflates intended stretch with mistuning. The table
characterises *the detuning regime the captures represent*, which is what
matters when choosing a search span or an unwrap range; the evidence for
actual tuning state is qualitative (report 0006): every capture set is of a
detuned piano.

## Consumption rules

**Validation only.** These captures may not select a configuration. With n = 1
instrument (or 2), a difference of a few keys is the McNemar-p ≈ 0.2 class of
evidence: report per-register counts and which keys moved, do not tune on
them, and do not recalibrate the synthetic generator to match them. The rules
for turning a measurement into a *decision* — what is pre-registered, what a
null must be chosen before, when a finding is only suggestive — are the
methods standard in [`reports/README.md`](../../reports/README.md), with the
three that bind a contributor restated in
[`CONTRIBUTING.md`](../../CONTRIBUTING.md). This file is the data; those are
the inference.

**Piano #2 must be consumed through `cargo lab mat regen`, never through raw
`analysis.json`.** The deep-bass entries were written before the
`worker::MAT_SEED_TOLERANCE` fix and carry rumble-seeded garbage: **35 of 595
entries** land beyond ±200 ¢, all in the deep bass, with A0 "measured" at
7–14.7 Hz against an ET 27.5 Hz. The *audio* is genuine; only the cached
analysis is wrong.

```bash
cargo lab mat regen diagnostics_piano2 > p2.json
python3 tuner-lab/scripts/audit_captures.py p2.json      # consumes the regen, not analysis.json
```

**Prefer independent truth to the cached fields.** `cargo lab strobe truth`
computes a zero-padded hi-res DFT truth per capture; that is the reference for
estimator accuracy work, not `measured_f0`.

**Every capture in `diagnostics/`, `diagnostics_piano_1/` and
`diagnostics_piano2/` is 1.5 s, and a measurement may not be made over more
than that.** The shipped path enforces it — the Worker analyses the first
`CAPTURE_ANALYSIS_SAMPLES` however long the record is, and `mat regen` bounds
itself identically — because report 0009's σ model, report 0010's concordance
and report 0011's profile were all measured at that length, and a longer
analysis window silently makes a new number incomparable with every one of
them. A session may still *record* longer (Settings → Advanced → Capture
Duration, off by default): the extra audio is for the questions a 1.5 s record
cannot answer — per-string decay τ, deep-bass resolution offline — and a
harness that uses it reads the file directly and states that it did.

**Release builds.** DSP must be exercised with `--release`; debug builds drop
audio and change availability figures.

**Which documents may quote these sets.** They are gitignored and per-user: a
reader outside this machine cannot open one, reproduce a tally, or tell when a
figure has gone stale, but a number quoted in prose reads like evidence either
way. The rule that governs code ([`style.md`](style.md), *No comment asserts
what a reader cannot open*) therefore governs prose too: **this file, the
reports and the audits may** quote the data, since describing and interpreting
it is their job; **README, ARCHITECTURE and the rest of `docs/internals/` may
not quote raw tallies** — capture totals, dump counts, per-set sizes — and
state the *result* with a cite to the record that holds it. Naming the
instrument is not a tally: "both validation uprights", or "instrument 2", stays
legible and true, and that both pianos are uprights is load-bearing for how far
a result generalizes. It is the counts that rot.

**The harnesses that read these sets**, and the on-disk format of a single
capture, are in [`tuner-lab/README.md`](../../tuner-lab/README.md).

## Where the numbers ended up

- report 0006 — discovery/TWM lock accuracy, the measured-B gate (piano #1).
- report 0009 — σ_lnB, ρ reproducibility, strike strength (piano #2 repeats).
- report 0010 — M-of-N lock rule, concordance across both pianos.
- report 0011 — the coarse readout: CFAR profile, P_fa calibration, n\* selection
  (both pianos **and** the guitar — the cross-instrument disagreement at n = 5
  is what fixed n\* = 4).
- report 0014 — the unison panel against isolation truth: the operating regime in
  beats, the false-beat positive control, the coupling bound, and the per-string
  `B` spread (the mute-isolation set).
