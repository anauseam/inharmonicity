# Evidence and Methodology — how a measurement becomes a decision

This file is about **inference**: what has to be true before a number is allowed
to change the code. It is not about algorithm design
([`04`](04-algorithms-and-models.md)), naming and provenance
([`05`](05-style.md)), or what the capture sets are and how to consume them
([`06`](06-capture-sets.md)) — it is the layer above all three.

Nothing here is new. The project has followed these rules since ADR 0006 and has
never written them down, which means they have been re-derived (and occasionally
missed) once per investigation. This is the record.

## 1. Confirmatory and exploratory are different claims

**Pre-register the decision rule before the measurement runs.** Write down what
would make you keep a thing, change it, or remove it — with thresholds and
directions — and only then measure.

The failure this prevents is not dishonesty, it is drift: an investigation turns
up something interesting, the interesting thing becomes the criterion, and the
criterion now justifies a conclusion it was fitted to. Everyone involved can be
acting in good faith.

Already practised throughout:

| Where | What was pre-registered |
| --- | --- |
| [ADR 0010](../adr/0010-m-of-n-lock-rule-replay.md) | the replay protocol's decisions, the support outcome gate, the concordance criterion, and *what M-of-N does not fix* |
| [ADR 0011](../adr/0011-coarse-spectral-readout.md) Context | a three-way comparison, fixed before the tracker was scored |
| [ADR 0006](../adr/0006-discovery-refinement-validation.md) | the protocol, including which keys were expected to stay failed |
| [`sequential-detection-design.md`](../design/sequential-detection-design.md) | the two-instrument concordance criterion, written while the design was still "build nothing" |

**Everything outside the pre-registered set is exploratory.** Report it in full —
it is often the most interesting part — but it may not move the decision on its
own. It queues work: a follow-up prompt, a gate on a second instrument, a
measurement for the next session at the instrument.

**When an exploratory finding should move a decision**, say so explicitly and
confirm it on data the criterion was not fitted to. That is what the second
instrument is *for*.

Design notes carry their status in the title — `(exploratory, build nothing)` on
the sequential-detection note, `(exploratory)` on the temporal-integration one.
Keep doing that.

## 2. Anchor a threshold to a measured quantity

A pre-registered bar is only honest if the number in it came from somewhere. In
order of preference:

1. **A prior measurement on this project's own data** — "≤ 5 %, because ADR 0012
   §5 measured 4 % on this instrument".
2. **A derivation from a published criterion** — `UNISON_MIN_BINS = 25` is
   Rohling §V solved for record length, not a hand-picked floor.
3. **The estimator's own scatter** — "within σ and the known bias", where both
   were measured.

An invented percentage is none of these. If no anchor exists, the bar is a
**product judgment** and must be labelled as one, with a human making the call
rather than an analysis appearing to derive it. That is not a defect; pretending
otherwise is.

This is the inference-side twin of [`04`](04-algorithms-and-models.md)'s
Topological Scrutiny Test, which bans fragile magic numbers in the *code*. The
same reasoning applies to the numbers in an *argument*.

## 3. Choose the null before scoring against it

A piano's spectrum is dense enough that a coincidence-based attribution
"explains" almost anything. [ADR 0013](../adr/0013-bass-extra-lines-attribution.md)
§3 is the worked example: against a **permutation** null — shuffling offsets
within a register — no candidate family beat its own null on both instruments,
while a naive uniform redraw manufactured a +9-point excess out of nothing.

The null is part of the pre-registration, not something selected once the scores
are in.

## 4. Suggestive is not a finding

[ADR 0013](../adr/0013-bass-extra-lines-attribution.md) §4 measured the bass
flanking-pair symmetry at 0.45–0.59 against a null of 0.80–0.83 — the most
interesting number in the section — and recorded it as **"suggestive, not a
finding"** because 0.45 is a long way from the 0 a real symmetric sideband pair
would give.

Write the distinction into the document. A reader six months later cannot
recover the author's confidence from the number alone.

## 5. Two instruments cannot select a configuration

The full rule, with the capture sets it applies to, is in
[`06`](06-capture-sets.md) — "Validation only". The short form: with n = 1 or 2
instruments, a difference of a few keys is the McNemar-p ≈ 0.2 class of
evidence. Report per-register counts and which keys moved. Never tune on the
validation instruments, and never recalibrate the synthetic generator to match
them.

Lock-accuracy scores are **relative**. 77/87 beats 74/87 *on that instrument*;
neither is an accuracy claim about pianos.

## 6. A model's numbers are provisional until the port reproduces them

When a design note's measurement phase ran in Python, its tables are the
*model's* output, not the app's. Porting it is a measurement in its own right.

[ADR 0012](../adr/0012-unison-line-estimator.md) is the case: porting
`unison-assist-design.md` overturned three of its published figures — the
resolution law is a sharp transition rather than a smooth curve, weak-second-string
sensitivity is separation-limited rather than level-limited, and bass repeat
reproducibility is 1 % rather than 11 %, which removed one of the three legs an
earlier conclusion rested on. The specified χ² discriminator was built exactly as
written and **measured wrong**, rejecting 87 % of genuine tenor unisons.

So: the ADR carries the port's numbers, and the design note is kept as the record
of what was considered and rejected. Do not re-derive from a superseded note.

## 7. A threshold-dependent question has no threshold-free answer

If the quantity you are measuring is defined by a threshold, the answer inherits
the threshold, and quoting a single number hides that.

The live example: "how long does the note stay above the noise floor?" has no
answer, because `noise_floor` is not measured — it is `silence_threshold`, a
config constant (`pipeline.rs`), which the D3 gate then scales by a
Neyman–Pearson factor. The decay stop that ends a capture is the same kind of
human-calibrated threshold.

The way out is one of:

- report the **trajectory** rather than a crossing time — no threshold needed;
- report crossings against **several named references** as a family, so the
  reader sees the spread the choice produces;
- use a **dimensionless** reference (−20/−40/−60 dB below a signal's own peak),
  which survives a gain change.

Naming the reference is not a caveat. It is the result.

## 8. A profile holds repeats; the product reads one and a statistic reads all

`InharmonicityProfile::active()` returns the **newest** trusted capture of a key,
and that is what the curve and the strobe consume. It is a deliberate choice, not
an omission: the piano is a changing object, and a key that was re-tuned or
re-strung should be represented by its current state, not by a median that
spans weeks. Whether to pool *within a session* is a design question that is
open; on the current evidence it would not move the curve (four re-captures in
A4–G#6 reproduced engine (d) to 0.01 ¢ at C8), because bass and mid repeat
noise is 0.14–0.48 % and the treble is shrinkage-dominated.

A **validation statistic** is a different consumer and must not inherit that
choice. Fitting the treble asymptote on instrument 2 gave +0.2 SE from Rigaud on
one day and −1.7 SE three days later, with the curve unchanged between them: the
only thing that moved was which capture was newest in four keys. Two rules
follow.

- **Pool repeats per key, by median, before fitting or comparing.** The
  estimator's failure is one-sided — a starved capture collapses `B` toward
  zero, never toward infinity — so a mean inherits the collapse and a median
  discards it. The pooled fit reads −0.9 SE and is stable across days.
- **Treat a nominal SE as optimistic when the input is one capture per key.**
  It counts scatter about the fitted line and nothing else: not which capture
  was active, not the collapse bias. A z-score computed from it overstates
  significance in both directions.

Anyone taking several captures of a key — which is every validation session —
should know that the profile keeps them (bounded per class, [`06`](06-capture-sets.md))
and the app reads one. The repeats exist for the statistics, not for the curve.

## 9. What the record is for

An ADR argues a decision; the code applies it. An audit
([`../audits/README.md`](../audits/README.md)) checks a port against its source
paper. This file governs the step in between — turning measurements into a
decision that is defensible later, by someone who was not there.

The practical test: **could a reader disagree with the conclusion using only
what is written down?** If the evidence for a decision is a number without a
population, a threshold without an anchor, or a comparison without a null, the
answer is no, and the record has failed regardless of whether the decision was
right.
