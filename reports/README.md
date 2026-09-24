# `reports/` — the evidence bodies, and the standard they answer to

This directory holds the **measurement reports** behind the project's decisions.
It is not documentation. Documentation tells a contributor how the system works
([`ARCHITECTURE.md`](../ARCHITECTURE.md), [`docs/internals/`](../docs/internals/))
and what was decided (the *Decisions* section of `ARCHITECTURE.md`); this
directory is the **defence** — what a reader
who was not there would need in order to disagree with a conclusion.

The split exists because the two genres have opposite virtues. A decision record
is short and immutable; an evidence body is long, revisable at its edges, and
worth nothing if compressed. Ten of the original sixteen ADRs had invented a
status (`MEASURED`, `INVESTIGATION COMPLETE`, `Draft (living)`) because they were
carrying both jobs at once, and ran roughly four times the length of the six that
were not.

## Layout

| Path | What it is |
| --- | --- |
| `0000-template.md` | The form. Copy it. |
| `NNNN-<slug>.md` | A numbered report. The index below says what each one decided and where that decision lives now. |
| `audits/` | Faithfulness audits — each ported algorithm checked symbol-for-symbol against its source paper. Evidence about a port's fidelity, so it lives here, not in `docs/`. |
| `retiring/` | Staged for the documentation site: the reports and audits being written up as pages. A report leaves once its page is published; an audit also waits for its differential test. [`retiring/README.md`](retiring/README.md) is the queue. |
| `nsga2/` | The NSGA-II sweep behind report 0001. Its README is the methodology (harness, synthetic signal model, selection protocol, threats to validity); `run1/`–`run4/` hold the Pareto-front JSON report 0006 cites. The Optuna databases are local only. |

Not every report decided something. `tuning-curve-grounding.md` is the argument
for how the curve layer is shaped, never a decision, and 0014 is a body whose
head was withdrawn.

## The methods standard

This file is about **inference**: what has to be true before a number is allowed
to change the code. It is not about algorithm design
([`layering.md`](../docs/internals/layering.md)), naming and provenance
([`style.md`](../docs/internals/style.md)), or what the capture sets are and how to consume them
([`capture-sets.md`](../docs/internals/capture-sets.md)) — it is the layer above all three.

Nothing here is new. The project has followed these rules since report 0006 and has
never written them down, which means they have been re-derived (and occasionally
missed) once per investigation. This is the record.

The section numbers below run 1, 3, 4, 6, 9 because they are kept from the
file this standard came out of. The three that are missing bind a contributor
at the moment of writing rather than a reader, and are in
[`CONTRIBUTING.md`](../CONTRIBUTING.md): anchoring a threshold (2), the
two-instrument limit (5), and the threshold-dependent-question rule (7). The
eighth, a profile holding repeats, is a system property in
[`layering.md`](../docs/internals/layering.md).

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
| [report 0010](0010-m-of-n-lock-rule-replay.md) | the replay protocol's decisions, the support outcome gate, the concordance criterion, and *what M-of-N does not fix* |
| [report 0011](0011-coarse-spectral-readout.md) Context | a three-way comparison, fixed before the tracker was scored |
| [report 0006](0006-discovery-refinement-validation.md) | the protocol, including which keys were expected to stay failed |
| [`report 0010` appendix](0010-m-of-n-lock-rule-replay.md#appendix--the-design-notes-derivation-trail) | the two-instrument concordance criterion, written while the design was still "build nothing" |

**Everything outside the pre-registered set is exploratory.** Report it in full —
it is often the most interesting part — but it may not move the decision on its
own. It queues work: a follow-up prompt, a gate on a second instrument, a
measurement for the next session at the instrument.

**When an exploratory finding should move a decision**, say so explicitly and
confirm it on data the criterion was not fitted to. That is what the second
instrument is *for*.

A design note built to think with says so on its **Tags** line (`exploratory`),
as the [template](../docs/design/template.md) sets out, so none of its sketches is
read as a specification.

## 3. Choose the null before scoring against it

A piano's spectrum is dense enough that a coincidence-based attribution
"explains" almost anything. [report 0013](0013-bass-extra-lines-attribution.md)
§3 is the worked example: against a **permutation** null — shuffling offsets
within a register — no candidate family beat its own null on both instruments,
while a naive uniform redraw manufactured a +9-point excess out of nothing.

The null is part of the pre-registration, not something selected once the scores
are in.

## 4. Suggestive is not a finding

[report 0013](0013-bass-extra-lines-attribution.md) §4 measured the bass
flanking-pair symmetry at 0.45–0.59 against a null of 0.80–0.83 — the most
interesting number in the section — and recorded it as **"suggestive, not a
finding"** because 0.45 is a long way from the 0 a real symmetric sideband pair
would give.

Write the distinction into the document. A reader six months later cannot
recover the author's confidence from the number alone.

## 6. A model's numbers are provisional until the port reproduces them

When a design note's measurement phase ran in Python, its tables are the
*model's* output, not the app's. Porting it is a measurement in its own right.

[report 0012](0012-unison-line-estimator.md) is the case: porting the
unison-assist design note overturned three of its published figures — the
resolution law is a sharp transition rather than a smooth curve, weak-second-string
sensitivity is separation-limited rather than level-limited, and bass repeat
reproducibility is 1 % rather than 11 %, which removed one of the three legs an
earlier conclusion rested on. The specified χ² discriminator was built exactly as
written and **measured wrong**, rejecting 87 % of genuine tenor unisons.

So: the report carries the port's numbers, and the design note is deleted once the
report exists; git history keeps what was considered and rejected. Do not
re-derive from a superseded note.

## 9. What the record is for

A report carries the measurement behind a decision; the code applies it. An
audit ([`audits/README.md`](audits/README.md)) checks a port against its source
paper. This file governs the step in between — turning measurements into a
decision that is defensible later, by someone who was not there.

The practical test: **could a reader disagree with the conclusion using only
what is written down?** If the evidence for a decision is a number without a
population, a threshold without an anchor, or a comparison without a null, the
answer is no, and the record has failed regardless of whether the decision was
right.

Three rules that bind a contributor at the moment of writing are restated in
[`CONTRIBUTING.md`](../CONTRIBUTING.md) — anchoring a threshold, the
two-instrument limit, and the threshold-dependent-question rule — and the
profile-repeats invariant is a system property, in
[`layering.md`](../docs/internals/layering.md).

## The report index

Every numbered report, what was decided on it, and where that decision lives
now. This table is the **complete decision log** — the examiner's view. The
contributor's subset is the *Decisions* section of
[`ARCHITECTURE.md`](../ARCHITECTURE.md), which lists only the choices that shape
the system; a component's method choice sits with its method under *DSP
foundations*, and a constant's definition is its own record.

Numbers are report IDs and have gaps. 0014 never had a decision worth a head.
0016 was retired on 2026-09-06: its rule lives in `style.md` and its rename map
in `tuner-lab/README.md`, so the body held only a finished migration's proof.
0017 and 0018 were argued rather than measured, so they have an
`ARCHITECTURE.md` entry and no report. Nothing is renumbered, and a retired
number is never reused.

| # | report | what it decided | where the decision lives |
| --- | --- | --- | --- |
| 0001 | [`0001-nsga2-tuning.md`](0001-nsga2-tuning.md) | TWM's constants are tuned by NSGA-II over a synthetic set with known truth, and the front is selected on real captures | the method itself — [`nsga2/`](nsga2/README.md) |
| 0002 | [`0002-twm-peak-masking-validation.md`](0002-twm-peak-masking-validation.md) | a −30 dB relative peak mask replaces the geometric gate | `peaks.rs` `GLOBAL_THRESHOLD_RATIO` |
| 0003 | [`0003-gatekeeper-rejection-of-sfm.md`](retiring/0003-gatekeeper-rejection-of-sfm.md) | spectral flatness is removed from the Gatekeeper | report only |
| 0004 | [`0004-instrument-scope.md`](retiring/0004-instrument-scope.md) | scope is struck and plucked stiff-string instruments | `ARCHITECTURE.md` § Decisions |
| 0005 | [`0005-discovery-algorithm-class.md`](0005-discovery-algorithm-class.md) | discovery is peak-domain TWM scoring, coarse-to-fine; MAT stays Worker-side | `ARCHITECTURE.md` § DSP foundations |
| 0006 | [`0006-discovery-refinement-validation.md`](0006-discovery-refinement-validation.md) | the conservative TWM constants ship; the measured-*B* pathway is gated off | `twm.rs` `TwmConfig::default`, `pipeline.rs` `APPLY_MEASURED_B_TO_DISCOVERY` |
| 0007 | [`0007-tuning-curve-regularization-geometry.md`](0007-tuning-curve-regularization-geometry.md) | prior-mean reversion, ℓ = 12 keys; minimum-norm chain gauge | `curves.rs` `REVERSION_LENGTH_KEYS`, `per_key_smoothed` |
| 0008 | [`0008-giordano-layer-fidelity-derived-weights.md`](0008-giordano-layer-fidelity-derived-weights.md) | coincidence-bracket scan, the §VI.C gate, LOO-CV + 1-SE, derived interval weights | `curves.rs` `GIORDANO_MIN_COINCIDENT_PAIRS`, `select_rho_reg_weight`; `giordano.rs` `SCAN_MARGIN_CENTS` |
| 0009 | [`0009-repeat-capture-noise-decomposition.md`](0009-repeat-capture-noise-decomposition.md) | inverse-variance ln-*B* shrinkage replaces the hard partial-count switch | `curves.rs` `SIGMA_LNB_COEFF`, `sigma_ln_b` |
| 0010 | [`0010-m-of-n-lock-rule-replay.md`](0010-m-of-n-lock-rule-replay.md) | the acquisition lock is M-of-N binary integration at (7, 8) | `ARCHITECTURE.md` § DSP foundations; `engine.rs` `record_stable_winner` |
| 0011 | [`0011-coarse-spectral-readout.md`](0011-coarse-spectral-readout.md) | a bounded OS-CFAR readout replaces the tracker fallback; *n\** = 4 below key 16; an 8-hop debounce | `peaks.rs` coarse block; `curves.rs` `COARSE_READ_FUNDAMENTAL_KEY`; `app/strobe.rs` `BAND_UNWRAP_MARGIN_HZ` |
| 0012 | [`0012-unison-line-estimator.md`](0012-unison-line-estimator.md) | strings resolve as spectral lines: zoom DFT, OS-CFAR, a Rohling-derived record floor, 1024 window; the χ² test is not shipped | `strobe/unison.rs`; `peaks.rs` `UNISON_*` |
| 0013 | [`0013-bass-extra-lines-attribution.md`](0013-bass-extra-lines-attribution.md) | the bass stays in unison assist; the discriminator withholds the claim per key | `strobe/unison.rs` module doc |
| 0014 | [`0014-unison-panel-against-isolation-truth.md`](0014-unison-panel-against-isolation-truth.md) | nothing — the panel ships unchanged and the fate decision waits | report only |
| 0015 | [`0015-ambient-sigma-gates-measured.md`](retiring/0015-ambient-sigma-gates-measured.md) | nothing moves; the per-bin startup floor is rejected on its own guard | report only |
| 0016 | *retired 2026-09-06* | the harness split into tests, benches and `tuner-lab` | the three-homes rule in `style.md`; the rename map in `tuner-lab/README.md` |
| 0017 | — | two FFTs per hop | `ARCHITECTURE.md` § Decisions |
| 0018 | — | one owner for `CaptureState` | `ARCHITECTURE.md` § Decisions |
| 0019 | [`0019-dc-blocker-corner.md`](retiring/0019-dc-blocker-corner.md) | the DC blocker's 35 Hz corner stays; change the order, not α | `audio.rs` `DC_BLOCK_ALPHA` |

Standalone, with no number: [`tuning-curve-grounding.md`](tuning-curve-grounding.md),
[`nsga2/`](nsga2/README.md) (the sweep's method and its runs), and
[`audits/`](audits/README.md).
