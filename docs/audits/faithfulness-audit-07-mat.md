# Faithfulness audit 07 — `mat.rs` re-check vs Hodgkinson et al. DAFx-09 (new lens)

**Series:** Prompt B faithfulness audits (status table in `faithfulness-audit-01-twm.md`), item 7 of 8.
**Date:** 2026-07-04.
**Source of truth:** Hodgkinson, Wang, Timoney & Lazzarini (2009). "Handling
Inharmonic Series with Median-Adjustive Trajectories." Proc. DAFx-09 —
primary source re-read (`resources/worker/MH_Inharmonic_Series_paper_94.pdf`).
**Scope:** this is the *re-check* pass. The algorithm itself was audited and
remediated 2026-06-25 (significance gate, band sizing, B_MAX, serial order,
per-partial Fo-array — all recorded in the module header and ADR 0006). The
new lens: (a) classify every OUR-constant, (b) verify every §/Eq citation
against the paper — four of six prior audits found citation defects.

## Citation verification (all §/Eq references checked against the paper)

| mat.rs claim | paper reality | verdict |
| --- | --- | --- |
| Eq 1: f_k = k·f₀·√(1+βk²) | Eq (1) | ✓ |
| Eq 6: f₀ = f_m/(m√(1+βm²)) | Eq (6) | ✓ |
| Eq 8 reduction in `compute_pair`: B = (K_n−K_m)/(K_m·n²−K_n·m²), K=(f/idx)² | Eq (8); the m² cancellation verified algebraically (numerator m²(K_n−K_m), denominator m²(k²K_m−m²K_n)) | ✓ **exact** |
| Eq 9: E = (K²−K)/2 | Eq (9) | ✓ |
| §2.2 significance gate = magnitude-spectrum average | §2.2, verbatim ("empirically determined most convenient to use the average of the magnitude spectrum") | ✓ |
| §2.2 stop when partials fade | §2.2 ("allows the latter to stop when no more significant partials are found") | ✓ (our 3-miss tolerance is a documented generous variant) |
| Fig 3 serial growth; bootstrap bands at f0_ET, 2f0_ET | Fig 3 + §2.2 text | ✓ (`run_serial` reproduces it: n=1,2 located before the first solve; b=0 until k≥2 ⇒ second band at 2f₀, as the paper) |
| Fo-array = K entries, one per partial ("page 3") | p. 3: "along with K f₀ estimates" | ✓ |
| §2.3 CSPE preferred; long windows (2¹⁴, 2¹⁵–2¹⁶ for string courses) | §2.3 (CSPE presented with RMS analysis, Fig 7; window guidance verbatim) | ✓ (Worker's 2¹⁶ = 65536 FFT sits exactly in the courses-of-strings recommendation) |
| §2.4 tight band ≈ f₀/16 full (4 bins in the F2 example) | §2.4, verbatim | ✓ |
| **"§7" for the multi-series / parallel-string limitation (3 sites + 2 in `mat_b_recovery.rs`)** | **The paper has no §7** (sections 1–5); the content is the **Conclusion, §4** ("the method has presently no command as to which series the trajectory will follow… one of the strings only") | ✗ **FIXED** — fifth citation defect of the series; content real, section number wrong |
| B_MAX doc: "real treble climbs further still (Fig. 10)" | Fig 10 tops out ≈ 2.8·10⁻³ at C#7 — it shows the steep treble *rise*, not values above the Rigaud 0.026 | ✗ misleading wording — **FIXED** (now quotes Fig 10's actual reach) |

Two staleness defects also found and **FIXED**:

- `detect_pitch_mat` arg doc said "`Simultaneous` is the shipped default" —
  `#[default]` is `Serial` and the Worker passes `Serial` (worker.rs:197).
- `extract_significant` doc said serial's band is "tight (paper-faithful)" —
  both orders currently use f₀/4 (the tight band was tested and reverted, as
  the `BAND_HALFWIDTH_F0_FRAC_SERIAL` doc itself records). The two comments
  predated that reversion.

## OUR-constant classification (the handoff's explicit ask)

| constant | value | classification |
| --- | --- | --- |
| `MAX_ITERATIONS` | 6 | (b) ours — the paper's serial procedure has no convergence loop at all; this (and both `*_REL_TOL`s) exist only for the `Simultaneous` order's Jacobi iteration. Unused by the shipped `Serial` path. Documented. |
| `F0_REL_TOL` / `B_REL_TOL` | 1e-4 / 1e-2 | (b) ours, same scope as above. |
| `B_MIN` / `B_MAX` pair filter | −1e-3 / 5e-2 | (b) ours — the paper has no B bounds; these drop mis-numbered pairs before the median. Rationale documented at both constants. |
| final `b.max(0.0)` clamp | — | (b) ours — physics (stiffness only raises partials); paper silent on negative medians. Documented at the call site; ADR-0006-recorded. |
| `SIM_MAX_PARTIALS` | 12 | (b) ours — empirically derived cap with the full mis-numbering mechanism documented (24 collapses bass B); inherent to the non-paper Simultaneous order. |
| `MAX_PARTIALS` | 32 | (b) ours — paper grows "as far as sufficient energy" (examples reach ~22–27); 32 is a buffer bound above that. Documented. |
| `SERIAL_MAX_CONSECUTIVE_MISSES` | 3 | (b) ours — generous variant of §2.2's stop rule (paper stops when no significant partial is found; ours steps over isolated gaps). Documented. |
| `BAND_HALFWIDTH_F0_FRAC_*` | 0.25 / 0.25 | (b) ours — documented empirical deviation from §2.4's ~f₀/16, with the A#0-279× fragility evidence and a revisit-on-in-tune-instrument note in place. |
| `BAND_HALFWIDTH_MIN_BINS` | 4.0 | (b) ours — resolvability floor; incidentally the paper's own §2.4 example band is exactly 4 bins. |
| `CONFIDENCE_EVIDENCE_PAIRS` | 10 | (c→resolved) ours, bespoke — part of the `confidence` scalar already demoted to runtime-diagnostic-only (ADR 0006 Corrections item 4: DAFx-09 outputs only (f₀,B); not persisted). Constant documented; no paper claim made. |
| coherence tolerance band | 0.5·\|median\|+5e-4 | (c→documented) ours, bespoke, same demoted-diagnostic scope; inline comment states its role. Arbitrary but inert outside the UI display. |
| `median_f32` upper-median on even counts | — | trivial ours (paper says "median" unqualified); deterministic, robustness-equivalent. Noted here, not worth a comment. |

## Verdict

The 06-25 remediation holds up under the re-check: **every equation and
method-section citation is accurate except the phantom "§7"** (now §4,
Conclusion — fixed at all five sites), and the OUR-constants are uniformly
(b)-class — deliberate, documented, with their empirical evidence recorded
in place. The confidence machinery remains bespoke-by-decision (runtime
diagnostic only, per ADR 0006). No behavioral findings; all fixes
comment-only; 28/28 tests pass.

Open items already tracked elsewhere (not new): tight §2.4 band + serial
confirmation + Simultaneous removal — all gated on the second, in-tune
instrument (mat.rs docs + ADR 0006).

## Audit series status

Item 7 complete. Running table: `faithfulness-audit-01-twm.md`. Remaining:
item 8 — Goertzel usage in `engine.rs` (textbook algorithm; verify the
tracking window/decision logic around it is documented as ours).
