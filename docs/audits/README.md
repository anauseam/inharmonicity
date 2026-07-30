# Faithfulness audits

Each load-bearing algorithm in the pipeline is meant to be a **faithful port of a
cited published method**, not a bespoke assembly (the rule in
[`../internals/04-algorithms-and-models.md`](../internals/04-algorithms-and-models.md),
"Analytical vs Ad-Hoc"). This folder is the line-by-line audit of every such
algorithm against its source paper — the record of *what has been confirmed
correct, what was fixed, and what was reclassified*.

Each audit classifies every deviation as **(a) faithful**, **(b) deliberate
documented adaptation**, or **(c) undocumented deviation / bespoke addition**
(flagged, then documented or removed). The running status table also lives at the
bottom of audit 01.

## Status — wave 1 complete (8/8); wave 2 complete (5/5); wave 3 (1/1)

| # | Algorithm / file vs paper | Verdict | Doc |
| --- | --- | --- | --- |
| 1 | `twm.rs` `score_candidate` vs Maher & Beauchamp 1994 | **Faithful** — core math + bandwidth cap (= paper Step 2); 3 numerical guards documented & kept | [01](faithfulness-audit-01-twm.md) |
| 2 | `spectral.rs` `cspe` vs Short & Garcia 2006 | **Faithful** — caller contract verified; doc-only fixes (stale Hamming→Hann) | [02](faithfulness-audit-02-cspe.md) |
| 3 | `spectral.rs` `jacobsen` vs Candan 2015 | **REAL BUG → FIXED** — bespoke (−1)^m + missing c_N≈2 biased every Discovery peak ~−2.5·δ bins; fixed per Candan Eq 1+12 (regression test) | [03](faithfulness-audit-03-jacobsen.md) |
| 4 | `peaks.rs` (`extract_peaks`, `mask_peaks`) | **Fixed cites** — `mask_peaks` Gómez citation fabricated → reclassified validated-bespoke (ADR 0002); phantom "Miron 2014" removed; Cano 40→30 dB documented | [04](faithfulness-audit-04-peaks.md) |
| 5 | `metrics.rs` (gatekeeper metrics) | **Fixed cites** — `ninos2` misattributed → relabeled ours (N/N_eff); nhwrsf lineage + band de-hardcoded; A/B built (`sparsity_ab`) | [05](faithfulness-audit-05-metrics.md) |
| 6 | `models.rs` `get_expected_beta` (Discovery B prior) vs Rigaud 2013 | **Faithful form** — treble constants = paper's universal fit exactly; bass = ours by design; σ_B "Fig. 3" attribution corrected | [06](faithfulness-audit-06-b-prior.md) |
| 7 | `mat.rs` re-check vs Hodgkinson DAFx-09 | **Faithful** — all Eq/§ citations verified except phantom "§7" (→ Conclusion §4); OUR-constants documented | [07](faithfulness-audit-07-mat.md) |
| 8 | Goertzel + phase-vocoder tracking in `engine.rs` | **Faithful** — recurrence textbook-correct; phase-offset constraint documented; NEYMAN_PEARSON_K re-derived exact | [08](faithfulness-audit-08-goertzel.md) |

**Tally:** 1 real bug fixed (jacobsen), 5 citation defects corrected, 2
validated-bespoke reclassifications (`mask_peaks`, `ninos2`); the load-bearing
core ports (TWM/M&B, CSPE, MAT, Goertzel) confirmed faithful. The jacobsen fix
re-baselined discovery to **discrete 76/87, refined 77/87** — see
[ADR 0006](../adr/0006-discovery-refinement-validation.md).

## Wave 2 — complete (Prompt B′, 2026-07-15)

The 2026-07 tuning-curve implementation introduced four new paper-based
modules; audit 05 had also left `ninos2` with a family-level citation. All
five items audited against the primary sources (all PDFs in
`resources/curve/`, `resources/moba/`, `resources/worker/`,
`resources/gatekeeper/`):

| # | Subject | Verdict | Doc |
| --- | --- | --- | --- |
| 5-add. | `metrics.rs` `ninos2` ℓ²/ℓ¹ citation pinning | **Pinned** — H&R *define* the measure (Table I + Thm 4.1); ours = N·(ℓ²/ℓ¹)², satisfies all six criteria (D4/P2 from the ×N factor); **H&R's own Thm A.4/Table III D3 entry shown erroneous** (contradicts their Thm A.19; derivation on record); Hoyer §3.1 + Bell & Dean 1970 cites added | addendum in [05](faithfulness-audit-05-metrics.md) |
| 9 | `algorithms/rigaud.rs` vs Rigaud 2013 | **Faithful** — Eqs 7–9/20/29–31 + treble pair + A&S erf all exact; 1 mis-citation fixed (§III.B.3 → §III.A.3.b); paper's s_B sign typo noted in-code | [09](faithfulness-audit-09-rigaud.md) |
| 10 | `algorithms/giordano.rs` vs Giordano 2015 (+ Sethares 1993, Plomp–Levelt 1965) | **Faithful** — Eqs 3–6 exact incl. all five Sethares constants; cross-pairs-only **reclassified as the paper's own Eq 5** (doc had undersold it as ours); "Eqs 4–6" cites → 3–4; dropped ½ prefactor documented | [10](faithfulness-audit-10-giordano.md) |
| 11 | `algorithms/whittaker.rs` vs Eilers 2003 | **Faithful** — Eqs 5/7/8/10/11 exact; LOO-identity cite fixed Eq 10 → Eq 11; w-weighted CV *score* documented as ours (identity exact for general W — derived + test-pinned) | [11](faithfulness-audit-11-whittaker.md) |
| 12 | `algorithms/curves.rs` — assembly audit | **Clean** — every Eq/§ claim verified (incl. §IV.C.2 `min` typo confirmed via Fig 9), every OURS component ADR-linked, constants sweep clean; 2 Eq-10→10/11 cites + 1 stale GCV comment fixed | [12](faithfulness-audit-12-curves.md) |

**Wave-2 tally:** 6 mis-citations fixed, 1 reclassification *toward*
faithful, 2 undocumented deviations documented, 3 paper errata recorded
(Rigaud ×2, Hurley & Rickard ×1), 1 stale doc fixed, 1 test added, **zero
math defects** — all fixes comment-level; 67/67 lib tests, lib clippy-clean,
engine output unchanged by construction. Audit 06 was renamed
`faithfulness-audit-06-b-prior.md` (2026-07-15) so the "rigaud" name could
go to wave 2's audit of `algorithms/rigaud.rs`.

## Wave 3 — the coarse readout (Prompt P step 1, 2026-07-26)

The first audit of code written *after* waves 1–2: the OS-CFAR gate behind the
coarse spectral readout (shipped 2026-07-25, ADR 0011). Primary sources in
`resources/tracker/`.

| # | Subject | Verdict | Doc |
| --- | --- | --- | --- |
| 13 | `peaks.rs` `coarse_read` + `cfar_multiplier` vs Rohling 1983 | **Faithful core, one false attribution fixed** — Eq. 14 reproduces all 32 `N = 32` entries of the paper's Table II (new test) and Eq. 17's `√` is scoped to OS-CFAR by the paper itself; "Rohling's own choice is the median" was **false** (§III lists it as one option; §V recommends `k ≈ 3N/4`); the shipped quantile 0.25 **upgraded from ours-measured to derived** from §V's `(N − k)` interference criterion (`k/N ≤ 1 − W_lobe/s` = 0.25 at A0, measured); guard cells shown **inert** on three sets exactly as §V predicts and therefore **removed**; flank-floor mechanism ("finds the valleys") corrected — at 5-bin spacing 75 % of cells are inside a lobe and the selected cell is a *weak upper partial's* skirt | [13](faithfulness-audit-13-cfar.md) |

**Wave-3 tally:** 1 false attribution fixed, 1 constant promoted bespoke →
derived, 1 constant **removed** as measured-inert (`COARSE_CFAR_GUARD_BINS`), 1
wrong mechanism corrected (3 sites), 1 symbol drift fixed (`T_sq` → the paper's
`T_q`), 1 test added (Table II), **zero math defects** and **no value changed**.
Parity re-verified from both sides after the removal (100.0000 %, Δf = 0,
36,523 hops/size) and realized AWGN P_fa 0.00097 → 0.00102 against nominal 0.001
with the per-band structure unchanged.
