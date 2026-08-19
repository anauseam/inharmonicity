# Capture Sets — the validation data

Every empirical claim in this project rests on sets of real recordings.
They are kept on disk and **never committed** (`.gitignore`: `diagnostics/`,
`diagnostics_piano*/`) — they are large, regenerable-in-principle, and
instrument-specific. This file records what they are, what state the
instruments were in, and the rules for consuming them, because none of that is
recoverable from the audio.

These sets live **in the repository**, and the harnesses that read them take the
directory as an argument (`-- diagnostics_piano2`), defaulting to `diagnostics`.
That is separate from where the *app* writes new dumps, which is a per-user
directory chosen by the frontend — a released binary has no useful working
directory — with one subdirectory per instrument, named for the opaque
`identity.id` its profile carries, so renaming an instrument moves nothing. Each
such directory holds an `instrument.json` naming whose captures they are. Dumps
written before 2026-08-15 sit in that root directly. See
[`../design/session-persistence-and-profile-library.md`](../design/session-persistence-and-profile-library.md) §3.

## The sets

| Directory | Instrument | Captures | Keys | Captured for |
| --- | --- | --- | --- | --- |
| `diagnostics/` | Steel-string guitar, standard tuning | 6 | 6 (E2 A2 D3 G3 B3 E4 = keys 19/24/29/34/38/43) | The guitar-strobe frequency audit; the cross-instrument check for anything piano-tuned |
| `diagnostics_piano_1/` | Upright piano #1 | 87 | 87 (one per key; **key 21 / F#2 is absent**) | The original discovery/TWM validation set — the "87 captures" every lock-accuracy number cites |
| `diagnostics_piano2/` | Upright piano #2 | 595 | 88 (≥ 5 repeats per key) | Prompt H's repeat-capture noise decomposition (ADR 0009); the project's first two-instrument evidence base |

Note the naming inconsistency: piano **1** has an underscore before the index,
piano **2** does not. This is why `.gitignore` uses `diagnostics_piano*/` — an
earlier `diagnostics_piano_*/` silently failed to ignore 315 MB of piano-2
captures.

## The mute-isolation set — **recorded 2026-08-15/16 on piano #2**

A fourth set, distinct from the three above and **not interchangeable with
them**: the same note recorded once per string in isolation (the others damped
with a mute) and once open. It exists because nothing in the three sets can
separate a real unison from one string beating against itself — every capture
of a multi-strung note is a blend, and a reference DFT hits the same `2/T` wall
the estimator does (ADR 0012 §8, ADR 0013 D3).

What isolation buys, and nothing else does: the true split as a difference of
two independently measured f₀ (not resolution-bound); a **false-beat positive
control**, since a solo capture that still resolves two lines is a false beat by
construction; and per-string B, which is the shipped discriminator's own
untested premise.

**Where it is.** Not in the repository: it lives in the app's per-user dump
directory under the instrument's own `identity.id` (profile `Piano2_extended` —
the same physical instrument as `diagnostics_piano2/`, in its **as-found** state
before tuning). 555 captures: a full-compass open pass at ~4 repeats per key,
plus seven complete isolation sets — C2, F2, C3, D3 (bichords) and A#3, A4, C5,
C6 (trichords) — each with every solo and the open note repeated. Consume it
through `regenerate_partials`, never the cached `analysis.json`.

**What it established**, and none of the other sets could:

- **Per-string f₀ repeats to 0.04–0.16 ¢** from the bass through the upper mid,
  so a solo capture's f₀ is a usable tuning target — 6–25× finer than a
  sub-1-¢ tolerance.
- **Precision collapses above C5**, tracking partial count: repeat σ is 0.14 ¢
  (A0–B1, 32 partials), 0.27 ¢ (C4–B4, 17), 0.75 ¢ (C5–B5, 10) and **2.16 ¢
  (C6–B6, 5)**. At C6 the estimator is bistable — its open captures fall into
  two clusters 13 ¢ apart. Per-string work is not feasible above ~C5, and a
  longer analysis window cannot rescue it: the limit is how many partials exist
  above the floor, not observation time.
- **As-found splits ran 1.0–17.5 ¢**, rising with pitch — 1.0–3.9 ¢ below A3,
  10.6–17.5 ¢ from A4 up.
- **Per-string B agrees to 0.4 % at A#3 and 1.7–2.5 % at A4/C5, but 4.7–8.3 % in
  the bass bichords** — 10–20× ADR 0009's bass repeat noise. If that survives
  scrutiny, the two strings of a bass unison genuinely differ in B, which would
  violate the unison discriminator's own premise exactly where ADR 0013 measured
  `p̂ ≈ 0`.

**The mechanism.** Which strings sounded is the operator's declaration, set
before arming and stamped onto the capture by the DSP thread as it dispatches
(`models::SoundingStrings`, `PipelineAtomics::capture_strings`). It reaches disk
in `analysis.json`'s `metadata.sounding_strings` and rides through
`regenerate_partials`; `null` means undeclared, which is what every capture in
the three sets above carries and what ordinary tuning writes. **String 1 is the
leftmost string of the note as the tuner faces the instrument.**

The declaration is offered only when **String Isolation** is switched on in
Settings (off by default, persisted in the app-settings document). While it is
off the control is hidden *and* the standing declaration is retracted, so no
capture can inherit a mute pattern from a session that has ended. A declaration
carries two facts, and both are needed: how many strings the key is strung with,
and which of them sounded. Without the count, one string of two and one of three
are the same record, and neither "is this the open note" nor "do I have every
solo" can be answered.

**Consumption rules**, in addition to the ones below:

- **A capture with `sounding_strings: null` is not part of this set**, whatever
  directory it sits in. The declaration is the set.
- **Screen bass solos on partial count.** A muted bass string is quiet enough
  that MAT can lock onto something else entirely: 2 of 8 C2 solo attempts came
  back with 17–20 partials and a B 30–90× the plausible value, against 32
  partials in the good captures. A solo whose partial count falls far below its
  key's open captures is a failed capture, not a measurement.
- **A solo capture measures one string, not the note**, so the profile retains
  it but never treats it as the key's measurement: `KeyMeasurement::is_trusted`
  disqualifies a declared, non-open capture exactly as it disqualifies an
  auto-mode one, and the tuning curve and strobe read the key's open (or
  undeclared) capture instead. A key measured *only* in isolation resolves to
  its newest solo, and the two consumers then part company: the curve **skips
  the key** (it admits trusted entries only, so the key falls back to the
  prior), while the strobe still reads that solo's `B`. A pass that never takes
  an open capture therefore leaves a hole in the curve, not a wrong point in
  it.
- **`on_key` is declared, not derived.** Where a piano's single/bi/trichord
  breaks fall is instrument-specific, and this set is the first data that
  records them — on piano #2, D3 is still a bichord while A#3 is a trichord.
  Declaring `on_key = 1` also declares the sounding string, since a
  single-strung key admits only one; two and three stay explicit, because there
  a forgotten mute would record a solo as an open capture. Changing the count
  clears the sounding set for the same reason — a pattern held across the
  change would declare a solo nobody made.
- **The profile is not the set.** `MAX_MEASUREMENTS_PER_KEY` caps a key at 8
  retained measurements, so an isolation key with four configurations × four
  repeats loses most of them from the profile; the audio is unaffected. Read the
  set from the dumps.
- Validation-only like the others, and doubly so until a second instrument's
  worth of it exists.

## Instrument state — read this before interpreting any number

**Both pianos are out of tune.** That is not a defect in the data, it is the
point: this app targets the out-of-tune and pitch-raise regime, so captures of
a freshly-tuned instrument would test the easy case only. It does, however,
constrain what the sets can prove:

- There is **no trusted `B` reference** on either instrument. This is the
  standing gate behind `APPLY_MEASURED_B_TO_DISCOVERY` and the ADR 0006
  measured-B pathway — see ADR 0006 and `pipeline.rs`.
- Lock-accuracy scores are *relative* measures. A config scoring 77/87 is
  better than one scoring 74/87 **on this instrument**; neither number is an
  accuracy claim about pianos.

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
matters when choosing a search span or an unwrap range. The evidence for actual
tuning state is qualitative and lives in ADR 0006 and the strobe design note
§15 ("every capture we have is of a detuned piano").

## Consumption rules

**Validation only.** These captures may not select a configuration. With n = 1
instrument (or 2), a difference of a few keys is the McNemar-p ≈ 0.2 class of
evidence. Report per-register counts and which keys moved; do not tune on them,
and do not recalibrate the synthetic generator to match them.

**Piano #2 must be consumed through `regenerate_partials`, never through raw
`analysis.json`.** The deep-bass entries were written before the
`worker::MAT_SEED_TOLERANCE` fix and carry rumble-seeded garbage — measured:
**35 of 595 entries** land beyond ±200 ¢, all in the deep bass, with A0
"measured" at 7–14.7 Hz against an ET 27.5 Hz. The *audio* is genuine; only the
cached analysis is wrong. So:

```bash
cargo run --release --example regenerate_partials -- diagnostics_piano2 > p2.json
python3 scripts/audit_captures.py p2.json      # consumes the regen, not analysis.json
```

**Prefer independent truth to the cached fields.** `examples/pitch_ground_truth.rs`
computes a zero-padded hi-res DFT truth per capture; that is the reference for
estimator accuracy work, not `measured_f0`.

**Every capture in the three sets is 1.5 s, and a measurement may not be made
over more than that.** The shipped path enforces it — the Worker analyses the
first `CAPTURE_ANALYSIS_SAMPLES` however long the record is, and
`regenerate_partials` bounds itself identically — because ADR 0009's σ model,
ADR 0010's concordance and ADR 0011's profile were all measured at that length,
and a longer analysis window silently makes a new number incomparable with every
one of them. A session may still *record* longer (Settings → Advanced → Capture
Duration, off by default): the extra audio is for the questions a 1.5 s record
cannot answer — per-string decay τ, deep-bass resolution offline — and a harness
that uses it reads the file directly and states that it did.

**Release builds.** DSP must be exercised with `--release`; debug builds drop
audio and change availability figures.

**The harnesses that read these sets**, and the on-disk format of a single
capture, are documented in
[`tuner-core/examples/README.md`](../../tuner-core/examples/README.md).

## Where the numbers ended up

- ADR 0006 — discovery/TWM lock accuracy, the measured-B gate (piano #1).
- ADR 0009 — σ_lnB, ρ reproducibility, strike strength (piano #2 repeats).
- ADR 0010 — M-of-N lock rule, concordance across both pianos.
- ADR 0011 — the coarse readout: CFAR profile, P_fa calibration, n\* selection
  (both pianos **and** the guitar — the cross-instrument disagreement at n = 5
  is what fixed n\* = 4).
