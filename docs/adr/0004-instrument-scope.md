# Pipeline Scope: Struck and Plucked Stiff-String Instruments

## Status

Accepted

## Context

The pipeline was designed around the acoustic piano, but the long-term goal is to
generalize it to other instruments. The open question was how far that generalization
should reach: all inharmonic instruments (including bells, bars, and membranes), all
instruments (including sustained-excitation families like bowed strings and winds), or
some principled subset.

An unbounded scope is not free. Every architectural decision in the pipeline encodes
physical assumptions, and supporting an instrument that violates those assumptions
means replacing subsystems, not parameterizing them. This ADR fixes the scope so that
future design work (instrument descriptors, the dynamic discovery algorithm, the
tuning curve solver) has a stable boundary to build against.

## Analysis

The pipeline encodes three independent layers of physical assumption. Each layer
generalizes differently, and the scope boundary falls where all three remain valid.

### Layer 1: The excitation model (Gatekeeper)

The Gatekeeper's 5-state machine assumes a percussive excitation followed by free
harmonic decay: NHWRSF detects the strike, NINOS2 identifies the "Golden Window" of
stable decay, and the 1.5-second capture assumes the note rings unattended while the
worker measures it.

- **Holds for:** every struck or plucked string — piano, harpsichord, clavichord,
  harp, hammered dulcimer, guitar family.
- **Fails for:** sustained excitation (bowed strings, winds, organ, voice). Vibrato,
  bow noise, and continuous re-excitation destroy the stable-decay-window concept,
  and the capture-and-measure workflow itself does not map onto instruments that are
  not tuned note-by-note. Supporting these means replacing the Gatekeeper's physics,
  not adjusting its thresholds.

### Layer 2: The partial model (Engine + Worker)

The Engine's `KeyProfile` and the Worker's MAT solver assume the stiff-string
dispersion law:

$$f_n = n f_0 \sqrt{1 + B n^2}$$

- **Holds for:** all struck/plucked strings. This is the same physics across the
  family — only the magnitude of $B$ changes (guitar ~10⁻⁵–10⁻⁴, piano ~10⁻⁵–10⁻²).
  Ideal-harmonic strings are the degenerate $B = 0$ case. Within this family,
  generalization is nearly free: parameterize the note table, range, and $B$-prior
  curve, and let the Worker's measured per-key $B$ override the prior.
- **Fails for:** modal percussion. Marimba bars (~1 : 4 : 10 after undercutting),
  carillon bells (hum/prime/tierce/...), and membranes are not a one-parameter
  stretch family. TWM scoring itself would survive (`predicted_partials` is already
  just an array and can hold arbitrary modal templates), but MAT's pairwise algebra
  is derived from the $\sqrt{1+Bn^2}$ form, and "the $B$ coefficient" — the
  application's central deliverable — stops being a meaningful quantity. Supporting
  modal instruments is a different offline estimator and a different product.

### Layer 3: The task model (tuning workflow)

Tuning curves, unisons, stretch octaves, and per-key profiles are concepts for
fixed-pitch, multi-string instruments tuned key by key. They transfer cleanly to
harpsichord, harp, and hammered dulcimer. They are irrelevant to instruments that the
engine could technically track (a guitar's six strings need no inharmonicity-aware
stretch curve) and undefined for instruments without fixed per-note tuning.

## Decision

The pipeline's scope is **struck and plucked stiff-string instruments**: instruments
where all three assumption layers hold simultaneously.

- **In scope:** acoustic piano (grand and upright), harpsichord, clavichord, harp,
  hammered dulcimer, and other fixed-pitch struck/plucked string instruments.
  Guitar-family instruments are physically supported but are not a design target.
- **Out of scope (permanently, absent a separate product decision):**
  sustained-excitation instruments — bowed strings, winds, organ, voice. These
  belong to the problem regime targeted by general pitch trackers (YIN, pYIN, BaNa),
  and entering it surrenders the analytic guarantees (decay-window stability,
  per-partial phase tracking, $B$ measurement) that differentiate this pipeline.
- **Out of scope (deferred, architecturally anticipated):** modal percussion
  (bells, bars, membranes). The Engine's template representation does not preclude
  arbitrary modal series, but MAT and the $B$-centric measurement workflow would
  need wholesale replacement. Any future support is a separate ADR.

## Consequences

- The future instrument-descriptor abstraction needs to parameterize exactly three
  things: the note table (names, count, ET reference frequencies), the expected-$B$
  prior curve (currently the hardcoded Rigaud-based `get_expected_beta`), and the
  capture/range constants. The Gatekeeper requires no per-instrument changes within
  scope.
- The partial model should eventually sit behind a small abstraction
  ($n \mapsto f_n$ given per-instrument parameters) with the stiff-string law as the
  first and only implementation. This documents the modal-percussion door without
  building anything behind it.
- The Worker's measured-$B$ pathway (MAT → `InharmonicityProfile` → Engine profiles)
  is the mechanism that makes the engine instrument-adaptive within scope: the prior
  curve only matters until the user measures their actual instrument.
- General-purpose pitch tracking (speech, sustained tones, MIR-style contour
  estimation) is explicitly a non-goal. Algorithm evaluations should be judged
  against the in-scope instrument family only.
