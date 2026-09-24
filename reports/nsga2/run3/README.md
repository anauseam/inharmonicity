# Pre-remediation NSGA-II runs — ARCHIVE ONLY, do NOT select from

`run1/`, `run2/`, and this `run3/` are NSGA-II
sweeps from **before the 2026-06-20/21 methodology remediation**. They are kept as the
**evidence base for the adversarial review** (the fronts here are exactly what the
review analyzed — e.g. the seed-42 `p≈0.8, λ≈1.5` synthetic overfit and the
near-degenerate objB with <0.001 spread). **Do not use any of these to select
constants.**

They were generated under the methodology the review found broken (see report 0006 and
the methodology in `reports/nsga2/README.md` §8):

- **objA = K=88 separability** — the regime report 0006 Finding #1 measured as the *worst*
  real setting; its optimum is degenerate (`floor_frac=0.22`).
- **objB = ordinal confidence** — near-degenerate (varies <0.001 across the front), so
  the search was effectively single-objective.
- **single seed (42)** — the optimum *location* is seed-noise.
- **no co-tuned structural arm**, **no floor gate**, **no fingerprint assertion**.

The corrected methodology (production K=3 bass-vs-treble objectives, floor gate,
3-seed pooling, population 128, co-tuned structural arms 6–7) lives in
`tuner-lab/scripts/optimize_twm.py`. Re-running it regenerates fresh fronts at the repo root.

`run3/` is the most recent pre-remediation run (the one the review used).
`twm_nsga2.db` here is `twm_nsga2_run1.db.bak`'s sibling — the run-1 db was moved into
`run1/`.
