# Faithfulness audit 06 — `models.rs::get_expected_beta` + σ_B constants vs Rigaud 2013

**Series:** Prompt B faithfulness audits (status table in `faithfulness-audit-01-twm.md`), item 6 of 8.
**Date:** 2026-07-04.
**Renamed 2026-07-15** from `faithfulness-audit-06-rigaud.md`: this audit covers
the *Discovery B prior* (`get_expected_beta`) against Rigaud 2013's model form;
the wave-2 audit of `algorithms/rigaud.rs` (the tuning-curve port of the same
paper) now owns the "rigaud" name (`faithfulness-audit-09-rigaud.md`).
**Source of truth:** Rigaud, F., David, B. & Daudet, L. (2013). "A parametric
model and estimation techniques for the inharmonicity and tuning of the
piano." JASA 133(5), 3107–3118 — primary source read
(`resources/moba/2013_a_parametric_model...pdf`).
**Scope:** `models.rs::get_expected_beta` (the Discovery B prior) and the
σ_B = 0.157/0.116 constants used by `mobo_evaluator.rs`,
`joint_b_refine_diagnostic.rs`, and `docs/design/mobo-methodology.md`.

## Paper specification

- **Model (Eqs 7–8):** log B along the compass has two linear asymptotes;
  B_ξ(m) = e^{b_B(m)} + e^{b_T(m)} with b_T(m) = s_T·m + y_T,
  b_B(m) = s_B·m + y_B, **m = MIDI note number, m ∈ [21, 108]** (A0 = 21).
  The additivity is explicitly a smoothing convenience, not physics.
- **Treble pair is universal** (after Young 1952): the paper fits
  **s_T ≃ 9.26·10⁻², y_T ≃ −13.64** across pianos (Young's own physics-based
  values: 9.44·10⁻² / −13.68) and fixes them.
- **Bass pair is piano-specific by design**: ξ = {s_B, y_B} is the free
  parameter set, estimated per piano (their per-piano results appear as
  curves in Figs 7/8/10; their algorithm-initialization example uses
  s_B = −8.9·10⁻², y_B = −7).
- **No scatter statistics**: the JASA paper contains no per-note B dispersion
  values; its Fig. 3 is an algorithm initialization/result spectrum figure,
  not a B-scatter plot. *(But the DAFx-11 precursor does — see the 2026-08-27
  addendum, which reverses finding 4.)*

## Verdict summary

| # | Item | Classification |
| --- | ---- | -------------- |
| 1 | Model form: dual-exponential additive, log-linear asymptotes | (a) faithful (Eqs 7–8) |
| 2 | Treble constants (0.0926, −11.788 in 1-indexed keys) | (a) **exactly** the paper's universal fit, correctly re-indexed |
| 3 | Bass constants (−0.066, −9.211 in 1-indexed keys) | (b) OURS by necessity — the paper defines them as piano-specific; provenance was undocumented |
| 4 | σ_B = 0.157/0.116 "[Rigaud Fig. 3]" | ~~(c) false attribution~~ → **(a) faithful**; the JASA paper dropped them, the DAFx-11 precursor has them verbatim (see Addendum 2026-08-27) |
| 5 | Bass-domain validity | documented limitation, not a deviation (ADR 0006: real upright bass B is 7–25× this prior) |

## Findings

**1–2. The curve is a faithful implementation with a verified index
conversion.** Code: B(n) = exp(−0.066·n − 9.211) + exp(0.0926·n − 11.788),
n = key_index + 1 (A0 = 1). Substituting m = n + 20 recovers the paper's
form exactly. Treble term: 0.0926·(n+20) − 13.64 = **0.0926·n − 11.788** —
i.e. our treble pair IS the paper's universal (s_T, y_T) = (9.26·10⁻²,
−13.64), re-indexed without error. This is the half of the prior the paper
declares portable across pianos, and it is the half our validation trusts
(the prior over-estimates only mildly in treble; ADR 0006).

**3. The bass pair is ours — necessarily.** In MIDI domain ours is
s_B = −6.6·10⁻², y_B = −7.891. The paper provides **no universal bass
values** (ξ is the per-piano free parameter; their worked example uses
−8.9·10⁻²/−7). Ours is a "typical medium piano" default of undocumented
origin — not a deviation from the paper (the paper *requires* choosing), but
the doc-comment implied all four constants came from the citation. Fixed:
the doc now states the treble/bass provenance split. The known consequence
is already on record: the real upright's measured bass B runs 7–25× this
default (ADR 0006, `validate_mat`) — a model-domain limitation of any fixed
bass choice, which is exactly why measured-B seeding was built (and gated).

**4. σ_B = 0.157/0.116 — false attribution, real constants.** Used as
per-note relative scatter ×(1 + σ·N(0,1)) in the MOBO synthetic generator
(`mobo_evaluator.rs:254`, "Rigaud Fig. 3 split"), as the ±n·σ log-grid
bounds in `joint_b_refine_diagnostic.rs`, and cited as "[Rigaud Fig. 3]" in
`mobo-methodology.md` §How-the-synthetic-is-built. **The paper's Fig. 3 is a
spectrum figure and the values appear nowhere in the paper.** They are OUR
synthetic-calibration constants (plausibly eyeballed from the cross-piano
spread in the paper's figures, but unverifiable). Consequences of the
correction: none behavioral — the harness experiments used σ as a scale
knob (the Prompt-3 diagnostic explicitly swept up to 20σ, so its refutation
did not hinge on σ's third decimal) — but the records must stop citing
Rigaud for them, and the split point (key ≤ 50 = A0–B4) is likewise ours.
Fixed in all three places; classified as ours-uncalibrated, to be re-fit
only if piano #2 data ever motivates it.

**5. Also checked:** the flattened single-equation doc-comment matches the
code's constants; C8 endpoint (B ≈ 2.6·10⁻²) is exactly the paper's own
treble line at m = 108; `KeyProfile`'s use of the prior (Nyquist-capped
partial table) is outside the paper's scope and already documented.

## Fixes applied (comments/records only — same session)

1. `get_expected_beta` doc-comment rewritten: paper's Eqs 7–8 named; MIDI →
   1-indexed conversion shown; **treble pair = paper's universal fit
   (verified), bass pair = OUR medium-piano default** (paper defines it as
   piano-specific); pointer to ADR 0006's bass-domain caveat.
2. `mobo_evaluator.rs` σ comment: "Rigaud Fig. 3 split" → ours, with the
   audit pointer.
3. `joint_b_refine_diagnostic.rs` σ helper comment: same correction.
4. `mobo-methodology.md` §synthetic: "[Rigaud Fig. 3]" → "(our calibration;
   mis-cited to Rigaud pre-audit — see faithfulness-audit-06)".

## Addendum — finding 4 reversed: σ_B is Rigaud's, from DAFx-11 (2026-08-27)

Finding 4 above is **wrong**, and the error is one of edition, not of reading.
This audit's source of truth was the 2013 JASA paper, whose Fig. 3 is indeed a
spectrum figure containing no scatter statistics. The values were taken from the
**DAFx-11 precursor** (`resources/curve/53_e.pdf`), §3.2, whose Fig. 3 *is* the
relative-deviation histogram:

> "We present on Figure 3 the histograms of the relative deviation between
> B\*(m) and B_θ(m) computed in A0-B4 (m ∈ [21, 71]) and C4-C8 ranges. In C4-C8
> range, the mean and the standard deviation are respectively equal to
> −4.2·10⁻³ and **1.16·10⁻¹**. In A0-B4 range we have respectively 4.6·10⁻³ and
> **1.57·10⁻¹**." — DAFx-11 §3.2

Both constants match to three digits, over the 5-piano corpus the original
`mobo_evaluator.rs` header already named. The split point corroborates it
independently: the code's `key <= 50` is MIDI 71 = B4, exactly the paper's
`m ∈ [21, 71]` boundary — not a value anyone would land on by eyeballing figures.
So σ_B and its split are **the paper's**, and the pre-audit citation was right;
it merely pointed at the wrong edition. Reclassified (a) faithful.

Two caveats to carry with the citation:

1. **The DAFx-11 paper is internally inconsistent about the label.** Its Fig. 3
   caption says "A0-B3" while §3.2's body says "A0-B4 (m ∈ [21, 71])"; the body's
   m-range is the unambiguous one and is what our code follows. The body's
   companion label "C4-C8 (m ∈ [72, 108])" is likewise off — m = 72 is C5 — while
   the *same* section earlier writes "C4-C8 (m ∈ [60, 108])" for the treble fit.
   Cite the m-ranges, not the note names.
2. **σ_B measures model-vs-data residual, not per-note physical scatter.** It is
   the dispersion of (B\*(m) − B_θ(m))/B_θ(m) after a 3-note bass fit, with treble
   outliers manually removed. Using it as a per-note draw — which the synthetic
   generator does — is the right order of magnitude for "how far a real note sits
   off the two-bridge curve," which is exactly the mismatch the MOBO synthetic
   wanted; it is not a claim about repeat-measurement noise. Our own measured
   repeat-noise figure is the separate σ_lnB(n) of ADR 0009.

**An unlooked-for corroboration of ADR 0009.** Rigaud's σ_B is the dispersion of
(B\*(m) − B_θ(m))/B_θ(m) — measured data against the fitted two-bridge curve.
That is the *same quantity* as `curves::sigma_prior`, the robust SD of
ln(B_meas/B_ξ) over low-noise keys (to first order ln(1+x) ≈ x, so the two are
comparable to within ~10 % at these magnitudes). Ours is self-calibrated per
instrument at 0.186 (upright #1) and 0.062 (upright #2), with
`SIGMA_PRIOR_DEFAULT = 0.12` picked as the midpoint for profiles too sparse to
calibrate. Rigaud's pooled 5-piano figures — 0.157 bass / 0.116 treble — fall
inside our two-instrument bracket, and the treble value lands on our interpolated
fallback almost exactly. So the ADR-0009 claim that σ_p is instrument-specific
and O(10 %) has an independent 5-piano anchor it did not have before, and the
`n = 2` fallback constant is better supported than "midpoint of two uprights."
One asymmetry stands unexploited: Rigaud's σ is **register-split** (bass scatter
~35 % larger), while `sigma_prior` returns one scalar per instrument. Splitting
it would raise w in the treble and lower it in the bass — a candidate refinement,
not taken here, and gated on the same evidence bar as anything else touching the
shrinkage.

No behavioral consequence, as before: nothing on the hot path or in the tuning
curve reads these constants; they scale synthetic B draws in two offline
harnesses. The four remediations listed in "Fixes applied" have been reverted to
cite `Rigaud DAFx-11 §3.2/Fig. 3`.

**Lesson for the series:** this audit's own header records the DAFx-11 paper as
the precursor, and audit 09 explicitly judged it "not needed; the JASA paper is
self-contained." That judgement is true of the *model* and false of everything
the journal version dropped — the σ statistics here, and the λ·B semitone
recursion that `models.rs::railsback_stretch_curve` still uses. When a port
cites a journal paper with a conference precursor, both editions are in scope.

## Addendum — the universal treble pair, checked on two uprights (2026-08-25)

The verdict above is that our treble constants **are** the paper's universal
pair, correctly re-indexed. This addendum asks the next question: is that pair
right for the pianos we have? It is the load-bearing borrowed assumption in the
top two octaves, and ADR 0009 analysis 7 shows no local measurement can check it
up there — so it was checked in the highest band where captures still resolve B.

**Why the paper's claim is physical rather than statistical.** §II.a: *"Down to
middle C (C4 note, m = 60), the values of B are roughly the same for all the
pianos … mainly due to the fact that string design in this range is
standardized, since it is not constrained by the limitation of the piano size."*
The size constraint lands on the **bass** bridge, which is precisely why
ξ = (s_B, y_B) is fitted per instrument and (s_T, y_T) is not. The paper's fit is
an L1 regression over 6 pianos in C4–C8 (4 real grands, 1 upright, 1 sampled
grand) and lands within 1.9 % of Young 1952's independent physics-based
derivation (s_T^[Yo52] ≃ 9.44e−2 vs 9.26e−2).

**Fitted on our two uprights**, over keys A4–G#6 where the treble half is ≥ 95 %
of B_ξ and captures still carry 3–14 partials. Instrument #2 stores several
repeats per key, and which one the fit reads matters: the curve reads the newest
(`active()`), while the per-key median over all repeats is the stable estimator.

| | slope s_T | slope SE | resid sd | vs Rigaud |
| --- | --- | --- | --- | --- |
| instrument #1 (one capture per key) | 0.0834 | 0.0054 | 0.183 | −1.7 SE |
| instrument #2, newest capture per key | 0.0903 | 0.0013 | 0.045 | −1.7 SE |
| instrument #2, median of repeats | **0.0911** | 0.0017 | | **−0.9 SE** |
| Rigaud (6 pianos) | 0.0926 | | | |
| Young 1952 (physics) | 0.0944 | | | |

Both uprights read the slope a little *shallow*, and neither difference is
significant on its own. The point estimate is snapshot-sensitive: a reading of
instrument #2 three days earlier gave +0.2 SE, and four re-captures in the band
moved it to −1.7. The direction is the one the estimator's known failure
predicts — partial count falls from 11 to 3 across the band (r = −0.93 with key),
and a capture with few partials biases B low, never high, so a shallow fitted
slope is what the bias would produce even on a piano that matched Rigaud exactly.
Binned by capture richness across both instruments, keys 48–87:
**≥ 10 partials → median B/B_universal = 1.02×** (log-SD 0.164); 7–9 → 0.93×
(0.32); 2–4 → 0.70× (1.19). Rich captures land on the model; poor ones skew low
and scatter an order of magnitude wider.

**What this bounds.** Propagated 27 semitones from the fit's centroid to C8, the
measured slope difference is worth **−0.9 to −1.4 ¢** depending on the estimator,
and the 2-SE prediction interval of the fitted line adds **±1.6–2.1 ¢**. Any
error in the borrowed asymptote is therefore worth **under ~3 ¢ at C8** — below
the repeat noise in our own top-octave pitch readings. This is not a precision
confirmation of Rigaud's value; it is consistency within the estimator's own bias.

**What it does not bound.** Every ≥ 10-partial capture on both instruments lives
at keys 48–71, and corr(key, #partials) = −0.79 pooled (−0.91 on instrument #1).
Above G#6 the model is *inferred by extrapolation, not measured*, and any
apparent departure there is perfectly confounded with the estimator's failure
direction. Two instruments do not settle it, because both go blind in the same
place for the same physical reason.

**Not covered: ρ.** This checks B only. The treble octave type ρ_φ is
`RhoPhi::TYPICAL`, and engine (c)'s calibration accepts zero ρ points above F4
(key 44) — worth up to 17 ¢ at A7, larger than the whole B uncertainty. ρ is a
preference rather than a measurable, so it is not an audit question; see
ARCHITECTURE.md, "What is still open".

## Audit series status

Item 6 complete. Running table: `faithfulness-audit-01-twm.md`. Remaining:
7 (`mat.rs` re-check — classify OUR constants: B ≥ 0 clamp, convergence
tolerances, `CONFIDENCE_EVIDENCE_PAIRS`, coherence band), 8 (Goertzel usage
in `engine.rs`).
