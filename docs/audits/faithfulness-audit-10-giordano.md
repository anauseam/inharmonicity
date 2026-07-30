# Faithfulness audit 10 — `algorithms/giordano.rs` vs Giordano 2015 (+ Sethares 1993, Plomp–Levelt 1965)

**Series:** Prompt B′ wave-2 faithfulness audits (queue table in `README.md`),
item 3 of 5.
**Date:** 2026-07-15.
**Sources of truth:** N. Giordano (2015), "Explaining the Railsback stretch in
terms of the inharmonicity of piano tones and sensory dissonance," JASA
138(4):2359–2366 — primary source read in full
(`resources/worker/2359_1_online.pdf`). W. A. Sethares (1993), "Local
consonance and the relationship between timbre and scale," JASA
94(3):1218–1228 — §I read (`resources/curve/consonance.pdf`, scanned, read
visually). Plomp–Levelt 1965 (`resources/curve/Plomp_Levelt_Tonal_1965.pdf`)
— bibliographically verified only (see finding 8).
**Scope:** the whole module — constants `B1…S2`, `pure_tone_dissonance`,
`normalize_power`/`dissonance`, `octave_scan` (+ coincidence bracket,
`SCAN_STEP_CENTS`, `SCAN_MARGIN_CENTS`, `max_pair_rank`),
`coincident_pairs`, `pair_width_sensitivity`, `strong_cross_pairs`.

## Paper specification (as read)

- **Giordano Eq 3**: d₂(f₁,f₂) = e^{−b₁s|f₂−f₁|} − e^{−b₂s|f₂−f₁|}.
- **Giordano Eq 4**: s = s*/[s₁·min(f₁,f₂) + s₂]; constants in the following
  text: b₁ = 3.5, b₂ = 5.75, s* = 0.24, s₁ = 0.021, s₂ = 19.
- **Giordano Eq 5**: D_total = ½·Σ_{i=1}^{n₁} Σ_{j=1}^{n₂} B_ij·d₂(f_{1,i},
  f_{2,j}) — a **cross-pair** sum, with the explicit parenthetical: "here we
  omit the 'self-dissonance' of a tone, which is included by some authors
  but is not important for our work since we will look to find the condition
  in which the dissonance of two tones is a minimum."
- **Giordano Eq 6**: B_ij = a_{1,i}·a_{2,j} (the "amplitude product" model;
  Eq 7 is the alternative minimum-loudness model). §VI.C: "in all of our
  calculations we assume that the two notes have equal total power…
  normalizing for each note the sum of the power contained in each of the
  partials."
- **Giordano §VI.C shift rule**: shift the upper note's fundamental by df₁
  and every partial n by n·df₁; endnote 24: accurate to order α (≈ 5·10⁻⁵).
- **Giordano §VI.C convergence**: A0–A1/A1–A2 need "at least 16 partials of
  the lower note and 8 of the higher member" for the asymptotic stretch;
  fewer "gives a significantly smaller predicted stretch"; A2–A3 converges
  with 6/3.
- **Sethares 1993 Eq 1**: d(x) = e^{−ax} − e^{−bx}, gradient fit to the
  Plomp–Levelt curves gives a = 3.5, b = 5.75. **Eqs 2–4**:
  d = v₁₂(e^{−as(f₂−f₁)} − e^{−bs(f₂−f₁)}), s = d*/(s₁f₁ + s₂) with f₁ < f₂,
  v₁₂ = v₁v₂; d* = 0.24 (derived from the Eq-1 model), s₁ = 0.021, s₂ = 19
  (least-squares fit). His **Eq 5** (single-timbre D_F) and **Eq 6**
  (two-tone D_F(α) = D_F + D_αF + cross sum) are where intra-note terms
  live.

## Verdict summary

| # | Item | Classification |
| --- | ---- | -------------- |
| 1 | `B1…S2` constants | (a) exact (Giordano Eqs 3–4 text = Sethares Eqs 1–4) — cite said "Eqs. 4–6" → **FIXED** |
| 2 | `pure_tone_dissonance` | (a) faithful, exact (Eqs 3–4; min-frequency convention matches) — same cite fix |
| 3 | `dissonance` cross-pair sum | (a) faithful — **RECLASSIFIED**: the old doc mislabeled cross-pairs-only as OUR deviation; it is Giordano's own Eq 5 → **FIXED** |
| 4 | `dissonance` drops Eq 5's ½ | (b) inert constant scale, now documented |
| 5 | `normalize_power` (∑a² = 1) | (a) faithful (§VI.C verbatim) |
| 6 | n·df shift rule | (a) faithful (§VI.C; first-order caveat = his endnote 24, matches our test note) |
| 7 | ≥ 8 coincident-pairs gate | (a-derived) §VI.C quotes re-verified against the PDF (16/8 asymptotic, fewer = smaller stretch, 6/3 at A2–A3) |
| 8 | Plomp–Levelt citation | bibliographic only — correct role (perceptual data source; the implemented parametrization is Sethares') |
| 9 | Coincidence bracket, ±10 ¢ margin, j ≤ 7, 0.5 ¢ step | (b) OURS, documented + derived (ADR 0008); labels verified |
| 10 | `pair_width_sensitivity` (Form 2) | (b) OURS, derived — derivation re-verified independently |
| 11 | `strong_cross_pairs` | (b) OURS, demoted diagnostic, labeled |

## Findings

**1–2. The roughness kernel is exact, but cited the wrong equations —
FIXED.** Code constants B1 = 3.5, B2 = 5.75, X_STAR = 0.24, S1 = 0.021,
S2 = 19.0 match Giordano's post-Eq-4 text to all digits, and trace to
Sethares 1993 Eqs 1–4 (a = 3.5/b = 5.75 from his Eq-1 gradient fit;
d* = 0.24 derived from that model; s₁/s₂ his least-squares interpolation) —
lineage now recorded in-code. `pure_tone_dissonance` implements Eqs **3–4**
(min-frequency convention identical to Sethares' f₁ < f₂ ordering); both
doc-comments had cited "Eqs. 4–6" — Eqs 5–6 are the *sum and weights*, not
the kernel. Fixed at both sites. (The module-header "Eqs. 3–6" for the
whole layer was already correct.)

**3–4. Cross-pairs-only is the PAPER's construction, not ours —
reclassified.** The old `dissonance` doc-comment presented the cross-pair
restriction as our deviation from "Giordano's full two-tone sum," justified
by the design note's scan-invariance argument. Reading Eq 5: Giordano's sum
runs tone 1's partials against tone 2's — cross pairs only — and the
self-dissonance omission is his, stated in the parenthetical after Eq 6
with the same rationale (immaterial to a two-tone minimum). The "full
two-tone sum" (D_F + D_αF + cross) exists only in Sethares' 1993 Eq 6. The
implementation was faithful all along; only its provenance label undersold
it. One true deviation remains: the code drops Eq 5's ½ prefactor — a
constant scale, inert for the argmin and for downstream weight ratios
(Form-2 weights are consistently un-halved) — now documented. Both fixes
applied.

**5. Equal-total-power normalization faithful.** ∑a² = 1 per note is
§VI.C's "equal total power" verbatim; correctly attributed in-code. The
level-invariance test (`test_dissonance_level_invariant`) pins exactly the
property the paper's normalization exists to provide.

**6. Shift rule faithful.** `f + n·df` is §VI.C's n × df₁ rule; the
in-code note that it is "the first-order retuning of a stiff string" and
the mistuning-independence test's tolerance discussion match the paper's
endnote 24 (accurate to order α).

**7. The ≥ 8 gate's §VI.C basis re-verified.** The quoted convergence
sentences exist as cited (p. 2364–2365): 16-lower/8-upper for A0–A1 and
A1–A2, "significantly smaller predicted stretch" below it, 6/3 for A2–A3.
The min(⌊N_low/2⌋, N_up) ≥ 8 reduction and compass-wide adoption of the
bass-derived floor are ours, documented (ADR 0008 Decision 2 — unchanged).

**8. Plomp–Levelt 1965.** The module implements no P&L-specific claim —
the perceptual data enter only through Sethares' parametrization (as in
Giordano's own Fig. 3). Reference [2] is bibliographically correct (JASA
38:548–560, 1965 — confirmed against both papers' reference lists) and
correctly scoped ("the *quantity* … is Plomp–Levelt sensory dissonance in
the Sethares parametrization"). The P&L PDF is retained in
`resources/curve/` for reference; no deeper audit is warranted until code
makes a P&L-specific claim.

**9–11. The OURS components say so.** The coincidence bracket (scan
window from the 2j:j beatless widths), the ±10 ¢ margin ("the one
remaining soft constant"), j ≤ 7 derived from `RHO_FIT_KAPPA_MAX`, the
0.5 ¢ step, and `strong_cross_pairs`' demotion are all labeled ours with
their derivations (ADR 0008). `pair_width_sensitivity` re-derived
independently: d₂'(0) = (b₂−b₁)·s exactly (the two exponentials' slopes at
the origin), Δf = f̄(2^{ε/1200} − 1) ≈ f̄·(ln 2/1200)·ε, giving
∂D/∂ε = a_p·a_q·(b₂−b₁)·s(f̄)·f̄·ln 2/1200 — matches the code and the
finite-difference test; zero free parameters as claimed.

## Fixes applied (2026-07-15, same session)

1. `B1` const doc: "(Giordano Eqs. 4–6)" → Eqs 3–4 + Sethares 1993 Eqs 1–4
   lineage.
2. `pure_tone_dissonance` doc: "Eqs. 4–6" → "Eqs. 3–4".
3. `dissonance` doc: cross-pairs-only re-attributed to Giordano's Eq 5
   (self-dissonance omission is the paper's own); Sethares Eq-6 noted as
   where intra-note terms live; the dropped ½ prefactor documented as an
   inert constant scale.

All comment-only; no behavior change (wave-2 verification recorded in
audit 12).

## Audit series status

Wave-2 item 3 complete. Next: item 4, `algorithms/whittaker.rs` vs Eilers
2003 (`resources/curve/eilers2003.pdf`).
