# Faithfulness audit 03 — `spectral.rs::jacobsen` vs Candan 2015 (+ Keyta & Dilaveroğlu 2025 assessment)

**Series:** Prompt B faithfulness audits (status table in `faithfulness-audit-01-twm.md`), item 3 of 8.
**Date:** 2026-07-02.
**Sources of truth (PDFs in `resources/engine/`):**

- Candan, Ç. (2015). "Fine resolution frequency estimation from three DFT
  samples: Case of windowed data." Signal Processing 114, 245–250 — the paper
  the code cites (`freq_estimation_with_windowing_Elsevier_SP_2015.pdf`).
- Keyta, B.M. & Dilaveroğlu, E. (2025). "An Exact Analysis of Fine Resolution
  Frequency Estimation Method from Three DFT Samples: Windowed Data Case."
  Elektronika ir Elektrotechnika 31(3), 30–37 — the generalization, assessed
  at user request (`03__ISSN_1392-1215__...pdf`).

**Scope:** `tuner-core/src/algorithms/spectral.rs::jacobsen` and its single
caller `peaks.rs::extract_peaks` (Discovery path, BASS_WINDOW_SIZE = 8192).
**No behavior changed in this audit** — the fix is queued pending user
approval because it has re-validation consequences (below).

## Paper specification (Candan 2015)

Two-stage estimation of f = (k_p + δ)/N: (1) peak bin k_p from the windowed
DFT magnitude; (2) fractional part from the three complex bins (Eq 1):

    δ̂ = c_N · Re{ (R[k_p−1] − R[k_p+1]) / (2R[k_p] − R[k_p−1] − R[k_p+1]) }

where R[·] are the **raw windowed DFT samples** — no phase manipulation of any
kind — and c_N is a **window-specific bias-correction factor**: c_N =
tan(π/N)/(π/N) ≈ 1 for the rectangular window (Candan 2011/2013), and for an
arbitrary window computed numerically from Eq 12, c_N = B₀²/(A₁B₀ + A₀B₁),
with A/B built from the window transform f_w and its derivative at 0, ±1
(Eq 10). For large δ Candan proposes an optional second iteration (Table 1);
Fig 3 shows the single-shot residual bias per window, with **Hann the
best-behaved** of all windows shown.

## THE FINDING — the implementation inverts and mis-scales the estimator on Hann data

**Classification: (c) undocumented deviation with observable consequence — a
real bug.** Two deviations from the cited paper compound:

1. **A bespoke `(−1)^m` "Hann phase correction"** (spectral.rs, `sign(m)`
   pre-multiplication of the three bins). Candan's estimator takes raw causal
   bins; no such correction appears in any of the cited papers. The doc-comment
   justifies it as correcting the [0, N−1] window's half-window time shift —
   but the estimator never wanted zero-phase bins. The causal bins'
   (−1)^l alternation between neighbours is **load-bearing**: it is what gives
   the formula its positive unit slope.
2. **The c_N bias-correction factor is absent entirely** (c_N = 1 implicitly)
   — for Hann, c_N ≈ 2 (Candan Fig 2; exactly 2 in the N→∞ limit).

**Analytic result** (continuum limit, Hann window, derivation verified
numerically): on raw causal bins the ratio evaluates to **exactly δ/2** —
whence c_N = 2. After the (−1)^m conversion to zero-phase form, the same
formula instead evaluates to **−1.5·δ/(1−δ²)**: wrong sign, wrong scale, plus
a δ³ distortion. The composite estimator error is ≈ −2.5·δ bins.

**Numeric verification** (exact port of the Rust code; real cosine, Hann over
[0, N−1], numpy rfft = realfft convention, N = 2048; errors in bins, mean over
random phases — `scratchpad/jacobsen_check.py`):

| true δ | A: code as written | B: raw bins, c=1 | C: raw bins × c_N (Eq 12) |
| ------ | ------------------ | ---------------- | ------------------------- |
| −0.45  | **+1.297**         | +0.225           | −0.00003                  |
| −0.15  | **+0.380**         | +0.075           | −0.00001                  |
| +0.15  | **−0.380**         | −0.075           | +0.00001                  |
| +0.45  | **−1.297**         | −0.225           | +0.00003                  |

Column A matches the analytic −1.5δ/(1−δ²) prediction exactly. Column C
(the faithful Candan method) is essentially exact at every δ — for Hann the
continuum relation is exactly linear, so the single c_N correction leaves no
usable residual (no second iteration needed; see K&D confirmation below).

**Why it never surfaced:** there is no `jacobsen` unit test (the only
Discovery-path spectral test is `test_cspe_super_resolution`, which tests the
*other* estimator), and TWM's multi-partial averaging is tolerant enough to
keep discovery mostly working on top of biased peaks.

**Impact:**

- Discovery peaks only (`extract_peaks` at 8192): every peak frequency is off
  by ≈ −2.5·δ bins — up to **±1.35 bins ≈ ±7.3 Hz** near bin edges, i.e.
  *worse than no interpolation at all* (bin-centre error is ≤ 0.5 bin ≈ 2.7 Hz).
- The Worker/MAT path is **unaffected** (it uses CSPE — audited faithful,
  audit 02).
- The 74/87 baseline, the MOBO-tuned TWM constants, and the
  `diagnostics/key_*/peaks.csv` frequencies were all produced **on top of the
  biased peaks**. Fixing the estimator changes the TWM error landscape, so the
  baseline must be re-measured and the pinned-constant conclusions re-checked
  (the bias was present in both the synthetic MOBO harness and the real
  captures, so the tuning was at least self-consistent — but the fixed
  estimator may shift both the baseline and the optimum).
  *[Correction 2026-07-05, Prompt A′: the parenthetical's synthetic half is
  wrong — `mobo_evaluator` synthesizes `SpectralPeak`s directly with an
  unbiased ~N(0, 0.2 Hz) jitter stand-in (`emit_partial_cluster`) and never
  calls `jacobsen`, so the bias lived ONLY in the real-capture layer. The
  tuning was synthetic-clean / real-selection-biased, not self-consistent;
  the fix ALIGNED the real peaks with the synthetic's assumption. Measured
  consequence: the arm-6 real-data side-finding (t1898, 77/87) did not
  survive the fix (75/87 vs the default's 77/87) — see ADR 0006 items 2/5,
  2026-07-05 entry.]*

## Resolution (2026-07-02, same day — fix APPLIED on user go-ahead)

All five steps of the proposed fix below are done:

- `jacobsen` now consumes raw bins × c_N per Candan Eq 1 + Eq 12
  (`candan_bias_correction`: 2.000332 @ 8192, 2.001329 @ 2048, asymptote 2.0
  otherwise); doc-comment rewritten (records the old bug and cites K&D).
- Regression test `test_jacobsen_bias` added (δ grid × both FFT sizes,
  |error| < 1e-3 bins); full lib suite 28/28.
- **Re-validation (fixed peaks, shipped default config):**
  - DISCRETE: **76/87** (bass 21/26, mid 33/33, treble 22/28) — was 74/87.
  - REFINED: **77/87** (bass 20/26, mid 33/33, treble 24/28) — was 74/87
    (bass 20, mid 32, treble 22).
  - The fix is a correctness change justified by the papers + synthetic
    verification — not a parameter selected on the captures — so the n=1 rule
    is not implicated by shipping it.
- **Consequence: every pre-fix number is stale** — the 74/87 baseline, the
  arm-6 t1898 77/87 (the shipped config now *ties* it), the 13-key
  failure-mode split, and the pitch-raise-reach figures were all measured on
  biased peaks. Re-derivation was done as **Prompt A′** (2026-07-05) — results
  in ADR 0006 items 2 & 5 (the t1898 candidate did not survive the fix).
- New failure sets (default config): discrete — 000/004/005/010/012 bass,
  080/081/082/084/085/086 treble; refined — 000/001/002/005/010/012 bass,
  080/082/086/087 treble. Mid register is now perfect in both modes.

## Proposed faithful fix (as queued at audit time; now applied — see Resolution)

Per Candan 2015 Eq 1 + Eq 12:

1. Drop the `(−1)^m` pre-multiplication — feed the raw complex bins.
2. Multiply δ by c_N computed for our exact window (Hann over [0, N−1]) and
   FFT size, via Eq 12 offline: **c_N = 2.000332 (N = 8192)**,
   c_N = 2.001329 (N = 2048, if ever used). A constant per window size with a
   derivation comment; no runtime cost.
3. Keep the boundary-bin and degenerate-denominator fallbacks (paper is
   silent; same adaptation class as CSPE's fallback — document as ours).
4. Add the missing regression test: synthetic Hann-windowed tones on a δ grid,
   assert |error| < 1e-3 bins (the audit's numeric shows ~3e-5 headroom).
5. Re-validation: re-run `scripts/validate_config.py` over the 87 captures to
   re-measure the baseline (fresh capture replays are unaffected — peaks.csv
   is regenerated by the engine; but any conclusions pinned to the old
   frequencies must be re-checked). Treat the result as a new baseline, not a
   tuning opportunity (n=1 rule stands).

## Keyta & Dilaveroğlu 2025 — usefulness assessment (user request)

What the paper contributes beyond Candan 2015:

1. **A generalized bias-correction factor (their Eq 20):** c_N chosen to
   minimize total squared bias over the whole interval |δ| ≤ 0.5 (least
   squares), rather than Candan's slope-at-δ=0 Taylor version. MATLAB given.
2. **An exact variance expression (Eqs 13–15, 25–27)** for arbitrary window
   and arbitrary δ — prior formulas were valid only near δ ≈ 0. Simplified
   forms + code for the cosine-sum family (Hann included). Their Fig 8 shows
   MSE ≈ variance across the whole δ range for the c_N-corrected estimator —
   i.e. **single-shot + c_N is practically unbiased; no second Candan
   iteration needed.**

**Verdict for our application:**

- **The bias side adds nothing measurable for us.** For Hann the estimator's
  δ-response is exactly linear in the continuum, so the two c_N definitions
  coincide to five decimals at our sizes (computed with both papers' own
  formulas): Eq 12 vs Eq 20 = 2.001329 vs 2.001314 (N=2048), 2.000332 vs
  2.000328 (N=8192). Difference ≈ 4e-6 bin worst case ≈ 20 µHz — far below
  anything the pipeline can use. **Recommendation: cite and implement Candan
  Eq 12 (the primary, simpler source); note K&D as confirming evidence.**
- **The variance expression is genuinely new capability, but not for now.**
  It would give a principled per-peak σ_f(SNR, δ) — usable for an SNR-based
  peak gate or uncertainty-weighted TWM. That is a *new method* (not in
  M&B), exactly the bolt-on class ADR 0006 gates on the second instrument;
  and the per-peak SNR estimate it needs is not currently computed. **File it
  as decision-support** (e.g., it quantifies when jacobsen's noise floor, not
  its bias, limits Discovery — relevant if a future Stage-A rank diagnostic
  implicates peak accuracy), **not as a hot-path change.**
- One incidental confirmation: K&D's estimator statement (their Eq 2) is the
  same raw-bins-times-c_N form — a second independent source contradicting
  our `(−1)^m` variant.

## Audit series status

Item 3 complete (this doc); fix queued on user approval. Running status table:
`faithfulness-audit-01-twm.md`.
