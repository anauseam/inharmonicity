# Faithfulness audit 06 — `models.rs::get_expected_beta` + σ_B constants vs Rigaud 2013

**Series:** Prompt B faithfulness audits (status table in `faithfulness-audit-01-twm.md`), item 6 of 8.
**Date:** 2026-07-04.
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
- **No scatter statistics**: the paper contains no per-note B dispersion
  values; its Fig. 3 is an algorithm initialization/result spectrum figure,
  not a B-scatter plot.

## Verdict summary

| # | Item | Classification |
| --- | ---- | -------------- |
| 1 | Model form: dual-exponential additive, log-linear asymptotes | (a) faithful (Eqs 7–8) |
| 2 | Treble constants (0.0926, −11.788 in 1-indexed keys) | (a) **exactly** the paper's universal fit, correctly re-indexed |
| 3 | Bass constants (−0.066, −9.211 in 1-indexed keys) | (b) OURS by necessity — the paper defines them as piano-specific; provenance was undocumented |
| 4 | σ_B = 0.157/0.116 "[Rigaud Fig. 3]" | (c) **false attribution** — not in the paper; the constants are ours |
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

## Audit series status

Item 6 complete. Running table: `faithfulness-audit-01-twm.md`. Remaining:
7 (`mat.rs` re-check — classify OUR constants: B ≥ 0 clamp, convergence
tolerances, `CONFIDENCE_EVIDENCE_PAIRS`, coherence band), 8 (Goertzel usage
in `engine.rs`).
