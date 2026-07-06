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

## Status — series complete (8/8)

| # | Algorithm / file vs paper | Verdict | Doc |
| --- | --- | --- | --- |
| 1 | `twm.rs` `score_candidate` vs Maher & Beauchamp 1994 | **Faithful** — core math + bandwidth cap (= paper Step 2); 3 numerical guards documented & kept | [01](faithfulness-audit-01-twm.md) |
| 2 | `spectral.rs` `cspe` vs Short & Garcia 2006 | **Faithful** — caller contract verified; doc-only fixes (stale Hamming→Hann) | [02](faithfulness-audit-02-cspe.md) |
| 3 | `spectral.rs` `jacobsen` vs Candan 2015 | **REAL BUG → FIXED** — bespoke (−1)^m + missing c_N≈2 biased every Discovery peak ~−2.5·δ bins; fixed per Candan Eq 1+12 (regression test) | [03](faithfulness-audit-03-jacobsen.md) |
| 4 | `peaks.rs` (`extract_peaks`, `mask_peaks`) | **Fixed cites** — `mask_peaks` Gómez citation fabricated → reclassified validated-bespoke (ADR 0002); phantom "Miron 2014" removed; Cano 40→30 dB documented | [04](faithfulness-audit-04-peaks.md) |
| 5 | `metrics.rs` (gatekeeper metrics) | **Fixed cites** — `ninos2` misattributed → relabeled ours (N/N_eff); nhwrsf lineage + band de-hardcoded; A/B built (`sparsity_ab`) | [05](faithfulness-audit-05-metrics.md) |
| 6 | `models.rs` `get_expected_beta` vs Rigaud 2013 | **Faithful form** — treble constants = paper's universal fit exactly; bass = ours by design; σ_B "Fig. 3" attribution corrected | [06](faithfulness-audit-06-rigaud.md) |
| 7 | `mat.rs` re-check vs Hodgkinson DAFx-09 | **Faithful** — all Eq/§ citations verified except phantom "§7" (→ Conclusion §4); OUR-constants documented | [07](faithfulness-audit-07-mat.md) |
| 8 | Goertzel + phase-vocoder tracking in `engine.rs` | **Faithful** — recurrence textbook-correct; phase-offset constraint documented; NEYMAN_PEARSON_K re-derived exact | [08](faithfulness-audit-08-goertzel.md) |

**Tally:** 1 real bug fixed (jacobsen), 5 citation defects corrected, 2
validated-bespoke reclassifications (`mask_peaks`, `ninos2`); the load-bearing
core ports (TWM/M&B, CSPE, MAT, Goertzel) confirmed faithful. The jacobsen fix
re-baselined discovery to **discrete 76/87, refined 77/87** — see
[ADR 0006](../adr/0006-discovery-refinement-validation.md).
