# Faithfulness audit 11 — `algorithms/whittaker.rs` vs Eilers 2003

**Series:** Prompt B′ wave-2 faithfulness audits (queue table in `README.md`),
item 4 of 5.
**Date:** 2026-07-15.
**Source of truth:** P. H. C. Eilers (2003), "A Perfect Smoother," Analytical
Chemistry 75(14), 3631–3636 — primary source read in full
(`resources/curve/eilers2003.pdf`). Whittaker 1923 cited as the method's
origin only (Eilers records that Whittaker used third-order differences; the
d = 2 endorsement is Eilers').
**Scope:** the whole module — `BandedSystem`/`BandedCholesky` (the shared
solver), `system`/`smooth`, `cv`/`cv_masked`, `LAMBDA_GRID_DECADES`,
`smooth_auto`.

## Paper specification (as read)

- **Eq 1**: Q = |y − z|² + λ|Dz|²; **Eq 5**: Δ²z_i = z_i − 2z_{i−1} + z_{i−2}
  (second-order penalty "works fine in many cases"; his Figs 2/4/10 use
  d = 2).
- **Eq 7**: S = Σ w_i(y_i − z_i)² with w ∈ {0, 1} for missing data;
  **Eq 8**: (W + λD′_d D_d)z = Wy. Missing points are "automatically and
  smoothly interpolated".
- **Some Polynomial Properties**: for large λ with d = 2, z approaches the
  *weighted least-squares straight line* through y (the polynomial of degree
  d−1 minimizing Σw_i(y_i−z_i)²).
- **Eq 9**: s_cv = √(Σ(y_i − ŷ_{−i})²/m) — the (unweighted) LOO score.
- **Eq 10**: z = ŷ = (W + λD′_d D_d)⁻¹Wy = **H**y — the hat/smoother matrix.
- **Eq 11**: y_i − ŷ_{−i} = (y_i − ŷ_i)/(1 − h_ii) — the fast LOO residual
  identity (credited to Hastie & Tibshirani / "well-known in the regression
  literature").
- **Search practice**: "the logarithm of λ was varied in steps of 0.5 on a
  linear grid to search for the minimum of s_cv. There is little point in
  trying to find a minimum with sophisticated search algorithms." (Fig 10's
  CV profile spans 10⁻²…10⁸.)
- **Caveat**: CV assumes independent errors; serially correlated errors bias
  λ low (the ADR 0009 chain-noise item measured exactly this and found
  r ≈ 0.15, mild).

## Verdict summary

| # | Item | Classification |
| --- | ---- | -------------- |
| 1 | Objective + d = 2 penalty + (W + λDᵀD)z = Wy | (a) faithful (Eqs 5, 7, 8) |
| 2 | Missing-data interpolation (w = 0) | (a) faithful (Eq 7 mechanism; test pins it) |
| 3 | λ-limit doc claims (λ→0 data; λ→∞ weighted LS line) | (a) faithful (his "Some Polynomial Properties" verbatim; tests pin both) |
| 4 | Fast LOO residual cited as "Eq. 10" | (c) **mis-citation → FIXED** (identity is Eq 11; Eq 10 is the hat matrix) |
| 5 | h_ii = [(W+λDᵀD)⁻¹]_ii·w_i | (a) faithful (H = M⁻¹W with diagonal W) |
| 6 | w-weighted CV *score* | (c → b) **undocumented deviation → now documented** (Eilers' Eq 9 scores unweighted; identity exactness for general W derived + test extended) |
| 7 | `cv_masked` | (b) OURS, documented (ADR 0007; pseudo-points shape the smoother but are excluded from the score — verified in code) |
| 8 | λ grid (half-decades, 10⁻²…10⁸) | (a) — matches Eilers' own stated practice; provenance line added |
| 9 | `BandedSystem`/`BandedCholesky` | (b) ours/textbook numerics, no paper claim (Eilers uses Matlab sparse chol); validated against a dense solve |
| 10 | `smooth_auto` < 3-points guard | (b) ours, inert input guard |

## Findings

**1–3. The smoother core is faithful.** The objective, the
second-difference penalty rows ([1, −2, 1] per Eq 5), the weighted normal
equations (Eq 8), zero-weight interpolation, and both λ-limit claims match
the paper exactly — including the precise form of the λ→∞ statement (the
*weighted least-squares line*, his polynomial-properties result), which the
in-code comment uses to justify smoothing the residual-from-prior (design
note §5). Tests pin λ→0, λ→∞ (curvature → 0 and a pure line passing
free), and gap interpolation.

**4–5. The fast-CV citation was off by one — FIXED.** Two sites (module
doc, `cv` doc) cited "Eq. 10" for the LOO residual (y_i − ẑ_i)/(1 − h_ii).
Eq 10 defines H; the identity is **Eq 11**. Both now cite Eq 10 for the
hat matrix and Eq 11 for the identity. The code's h_ii = [M⁻¹]_ii·w_i is
the correct diagonal of H = M⁻¹W.

**6. The weighted CV score is ours — now documented, derived, and
test-pinned.** Eilers' Eq 9 sums *unweighted* squared LOO residuals over
non-missing points (his W is 0/1). Our `cv` multiplies each squared LOO
residual by w_i — the natural choice when w carries heterogeneous precision
(the shrinkage/pseudo-observation weights of the curve engines), scoring
prediction error in the same metric the smoother minimizes; at 0/1 weights
it reduces to Eilers' score (up to the argmin-inert √·/m). Two supporting
facts now on record: (i) the Eq-11 identity itself is **exact for a general
diagonal W** — deleting point i is the rank-one update M − w_i·e_i·e_iᵀ,
and Sherman–Morrison gives y_i − ẑ^{(−i)}_i = (y_i − ẑ_i)/(1 − h_ii)
exactly (derivation checked by hand for this audit); (ii)
`test_loocv_vs_brute_force` previously pinned the identity only at uniform
weights — **extended** with a heterogeneous-weights + missing-point case
(passes; the fast and brute-force scores agree to 1e-6 relative).

**7. `cv_masked` verified.** The masked points still enter W (hence ẑ and
h_ii) but are skipped in the score — exactly the documented ADR-0007
semantics (pseudo-observations are prior, not data). No change.

**8. The λ grid is Eilers' own practice.** Half-decade log steps are his
stated search method and Fig 10 spans the same 10⁻²…10⁸; a provenance line
was added to `LAMBDA_GRID_DECADES` (previously the rationale was stated
without noting the paper does the same).

**9. The banded solver makes no paper claim, correctly.** Eilers solves
with Matlab's sparse Cholesky; our `BandedSystem`/`BandedCholesky` is
standard banded-Cholesky numerics validated against a dense
partial-pivoting reference (`test_banded_vs_dense`), shared with engine
(d) as documented. The `add_row` duplicate-column factor-2 subtlety was
re-checked (folding both symmetric cross terms onto the diagonal —
correct).

**Caveat recorded, already handled elsewhere:** Eilers warns CV assumes
independent errors (serial correlation biases λ low). ADR 0009 measured
the chain-noise correlation at r ≈ 0.15 (lag 1 and lag 12) and judged
LOO-CV adequate; the 1-SE rule (ESL §7.10, applied by the callers in
`curves.rs`) additionally biases *away* from undersmoothing. Consistent;
nothing to change.

## Fixes applied (2026-07-15, same session)

1. Module doc + `cv` doc: Eq-10 → Eq-11 for the LOO identity (Eq 10 kept
   for the hat matrix).
2. `cv` doc: the w-weighted score documented as ours; general-W exactness
   of the identity recorded (Sherman–Morrison).
3. `LAMBDA_GRID_DECADES` doc: Eilers-practice provenance added.
4. `test_loocv_vs_brute_force` extended with a heterogeneous-weights +
   missing-point brute-force check (behavior-neutral test addition;
   6/6 module tests pass).

## Audit series status

Wave-2 item 4 complete. Next: item 5, `algorithms/curves.rs` assembly
audit (citation checks + OURS/ADR labeling + constants sweep).
