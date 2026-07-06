# Post-remediation MOBO re-sweep — ARCHIVE

These are the TWM MOBO Pareto fronts (`twm_pareto_arm1..7.json`) and the Optuna study
DB (`twm_mobo.db`) generated at the repo root by the **corrected** `scripts/optimize_twm.py`,
archived 2026-06-30 to keep the repo root clean now that the TWM parameter experiment
is concluded.

**Distinct lineage from `mobo_run{1,2,3}_backup/`.** Those three are the *pre-remediation*
runs the adversarial review found broken (objA = rejected K=88 separability, degenerate
objB, single seed, no structural arm). This run4 is the **first run under the corrected
methodology**: production-K3 **bass** FL (objA) vs **treble** FL (objB), floor gate,
3-seed pooling, population 128, co-tuned structural arms (6–7), fingerprint assertion.
See `docs/design/mobo-methodology.md` and ADR 0006.

**Status / how to read these.** The shipped `TwmConfig::default()` remains the conservative
`p=0.5, q=3.88, r=1.426, ρ=0.298, λ=18` (74/87 on the one real upright). These fronts were
*not* used to re-select constants — the structural terms (nonpeak/smoothness) helped
synthetic but hurt real bass and were rejected, and any bass-side selection remains gated
on a second instrument (n=1 cannot select). They are retained as the evidence base for the
re-sweep, not as a source to select from.

To regenerate fresh fronts, re-run `scripts/optimize_twm.py` (writes back to the repo root).
