# Discovery Refinement — Deltas for the MOBO Implementation Plan

Handoff brief for the agent revising `.agents/implementation_plan.md`. Decisions
behind these deltas are recorded in `docs/adr/0005-discovery-algorithm-class.md`
(algorithm class, split discovery, ratio question) and `docs/adr/0001-mobo-tuning.md`
(MOBO methodology). Quantitative rationale (mis-association bound, seeding math,
scale invariance) lives in `docs/design/discovery-search-analysis.md`.

## 1. `twm.rs` — extend the planned `TwmConfig` refactor

- Signature: `score_candidate(peaks, profile, scale: f32, cfg: &TwmConfig)`.
- `scale` multiplies predicted partials inline (`profile.predicted_partials[i] * scale`).
  Scaling preserves ordering, so the O(N+K) two-pointer sweeps are untouched. The
  dynamic bandwidth cutoff becomes `max_obs_freq + profile.f0_et * scale`. Zero
  allocation.
- `p` fast path: `if cfg.p == 0.5 { 1.0 / f.sqrt() } else { f.powf(-cfg.p) }`.
- Regression test (already in plan, extended): default `TwmConfig` + `scale = 1.0`
  must produce byte-identical scores to the current hardcoded path.

## 2. `engine.rs` — split discovery

- **Stage A (coarse):** existing 88-key scan at `scale = 1.0`, but collect the top-3
  candidates (fixed-size insertion array, zero allocation) instead of the single min.
- **Stage B (refine):** per candidate, a 9-point pre-grid at 20-cent spacing over
  ±80 cents, then golden-section in the best bracket (~8–10 evals → ~2-cent
  precision). The pre-grid is mandatory: error-vs-scale is piecewise (peak-to-partial
  nearest-neighbor associations switch discretely), so a pure unimodal line search is
  unsafe. Cost ≈ 60 extra `score_candidate` calls, discovery frames only.
- Winner = minimum refined error; `identified_key` = the winning candidate's own key
  (the ±80-cent basin clamp guarantees refinement re-ranks Stage A's candidates and
  cannot escape toward sub-harmonics, 1200 cents away). The 3-frame consistency gate
  is unchanged (key-based, stateless).
- **Seeding (the payoff):** `tracking_targets[i] = profile.predicted_partials[i] * s_win`
  replaces ET seeding. (ET seeds put higher partials of mistuned notes outside the
  Goertzel phase-unwrap range of ±21.5 Hz at the 1024-sample hop.)
- **Manual mode** (`target_note` bypass) also runs Stage B on the single target
  profile — currently the worst-seeded path, and the critical one for Pitch Raise.
- **Cent meter semantics unchanged:** `partial_cents` is still measured against the
  *unscaled* tuning-curve targets. Refinement is discovery-internal; a 50-cent-flat
  note must still read 50 cents flat.

## 3. Evaluator / orchestrator changes

- `mobo_evaluator` gains a `--refine` flag; discrete-vs-refined discovery becomes an
  ablation arm under the plan's existing Occam decision rule. Tune {q, r, ρ, λ, p}
  under whichever mode wins.
- **Sequencing constraint:** implement refinement BEFORE running the optimization.
  Tuning constants against the discrete-ET engine on a ±50-cent-detuned dataset bakes
  grid-compensation into the parameters and forces a full re-run later.
- Objective B caveat: under discrete mode on detuned frames, the margin partly
  measures distance-to-grid (an artifact), not candidate separability. Interpret the
  ablation accordingly.
- Keep the generator's ±50-cent detuning clamp strictly inside the ±80-cent
  refinement window.
- Note for Arm 4: `p = 1` is the ratio-error limit of the TWM term (each term becomes
  Δf/f). The "score ratios" question is settled empirically by this arm, not by fiat
  (ADR 0005, Decision 2).

## 4. Verification order

1. Pre-MOBO smoke test: the byte-identical regression test, then
   `scripts/test_engine_all.py` over `diagnostics/` with refinement on vs. off under
   default constants. This catches implementation bugs and gives early real-acoustic
   signal; it is NOT the formal verdict.
2. Formal verdict: the MOBO ablation on the fixed-seed synthetic dataset.
3. Afterwards: record the ablation results in a validation ADR (house pattern of
   ADR 0002), e.g. `0006-discovery-refinement-validation.md`.
