# ADR 0006 — Discovery Refinement & TWM Calibration: Validation

## Status

**Draft (living) — reverted from a premature "Accepted".** The conservative tuned
constants are adopted as `TwmConfig::default()` **provisionally**. Two rounds of
adversarial review — a prose/bank review (2026-06-19) and a two-agent MOBO methodology
review (2026-06-20, dataset-realism + optimization/inference; full audit in
`docs/design/mobo-methodology.md` §8) — found **five** load-bearing claims that did not
survive. Verdict: **trust-with-caveats** (the shipped constants stay a defensible
*provisional* default; the stronger conclusions are withdrawn). The five, corrected
throughout this ADR:

1. **"No register regressed" was false.** Against the stated discrete baseline
   (71/87), the shipped config is bass +1, mid **−1**, treble +3 (net +3). The mid
   regression (key 034 G3 → adjacent key 35) was omitted; it is a *refinement* cost
   (it appears when Stage B is enabled, M&B-discrete 33 → M&B-refine 32) and is
   constants-neutral, but the headline mixed the discrete and refined baselines so
   "no regression" silently assumed the refined one. See revised Decision.
2. **"Options exhausted / fundamental limit" was premature.** The Duan-non-peak and
   Emiya-smoothness rejections are *frozen-constant* tests (the structural coefficient
   bolted onto frozen q/r/ρ) — exactly the test Finding #3 says is invalid for
   error-landscape changes. These terms add new (and, for non-peak, un-normalized)
   error contributions whose scale interacts with q/r/ρ, so they need a co-tuned MOBO
   arm that was never run. The B-mismatch diagnostic (0005 revisit #3) is also still
   unmeasured. The rejections stand *as bolt-ons*; the stronger claim does not.
3. **"MOBO selected the constants" — it did not.** The search's own optimum is
   degenerate (`floor_frac=0.22`: objA is minimized by entering the error-collapse the
   ordinal-objB switch was meant to retire) and **seed-fragile** (the "p≈0.8, λ≈1.5
   synthetic overfit" of Finding #6 is a seed-42 artifact; seeds 1/7 give λ≈15). objA
   in refine mode is the **K=88** argmin — the very regime Finding #1 measured as the
   *worst* real setting. The shipped constants came from the **real-data plateau**, not
   the synthetic argmin (q=3.88 sits below every seed's synthetic optimum of 5.7–7.6).
4. **"+3/+4 beats M&B" is not statistically significant.** McNemar on the 6 discordant
   keys gives p≈0.22; 4 of 5 gains are manual-mode extreme treble; bass moved +1.
5. **The multi-objective framing is thin.** objB varies by <0.001 across the whole
   Pareto front, so NSGA-II was effectively single-objective on a (flawed) objA.

This ADR stays Draft until the review's **required experiments** run or are explicitly
closed: (1) **a second real instrument + in-tune captures** (the highest-value action —
n=1 cannot carry the selection); (2) a **co-tuned structural MOBO arm** (was impossible
in code — the serve protocol accepted only the 6 base params); (3) the
**B-residual diagnostic** (0005 revisit #3). The experimental terms remain in
`TwmConfig` as default-off.

**Post-review methodology remediation (2026-06-20/21, done — see
`mobo-methodology.md` §4/§5/§8):** the search methodology was rebuilt before the
re-sweep:

- **Objectives redesigned** — objA = production **K=3 bass** false-lock, objB =
  production **K=3 treble** false-lock (an *orthogonal*, non-degenerate tradeoff),
  replacing the rejected K=88 separability objA (claim #3) and the near-degenerate
  ordinal objB (claim #5). Error-collapse trials (`floor_frac > 0.05`) are rejected.
- **K-robustness diagnostic** `prod_fl_k{2,3,4,5}` added — addressing the worry that
  K=3 over-commits to an unproven K. The shipped constants are **K-robust**, not
  K=3-overfit: production FL is nearly flat across K (conservative: K2 0.266 / K3 0.263
  / K4 0.262 / K5 0.262, *slightly better* at larger K). K=3 stays **empirical** (best
  of {3,88}, n=1); a K-sweep is future work.
- **Co-tuned structural arms** (6, 7) wired — `nonpeak`/`smoothness` are now free
  params with q/r/ρ re-optimized around them (the valid test claim #2 / Finding #3
  required), so the re-sweep settles whether they help when co-tuned.
- **Search robustness** — 3-seed pooling (42/1/7) + population 50→128 (seed-fragility),
  NaN-safe comparisons, Optuna pinned, dataset fingerprint asserted before a run.

**Re-sweep complete (2026-06-23/24) — structural-term question settled.** The
corrected sweep ran (7 arms × 3 seeds, bass-vs-treble objectives). Result on the
co-tuned structural arms (6, 7): the synthetic and the real piano **disagree, and real
decides.** Co-tuned `nonpeak` *Pareto-dominated* the no-structural arm on **synthetic**
(overall FL 0.254→0.207) — looked like a win — but on the **real** piano it *hurts*
(same q/r/ρ: nonpeak 0.047 → 68/87 bass 14, vs nonpeak off → **77/87 bass 21**);
`smoothness` didn't help even on synthetic. **Both structural terms are therefore
rejected on real, now by the valid co-tuned test** (not the disputed bolt-on), and the
result is coherent with the oracle-B / B-limited-bass finding below: charging
predicted-but-absent partials crushes a true bass note whose template is already
mis-shaped by the wrong B. *Side-finding (candidate, not adopted):* the nonpeak-off
high-q/high-ρ config scored 77/87 (bass preserved) on the one piano — +3 vs the
provisional default — but n=1 (prior +3 was McNemar p≈0.22), so it is **queued for
second-instrument validation, not adopted.** The re-sweep fronts are archived in
`docs/mobo-archive/mobo_run4_backup/`; the pre-remediation fronts in
`docs/mobo-archive/mobo_run{1,2,3}_backup/` *(paths updated 2026-07-02 — formerly
repo-root `mobo_run*_backup/`; re-running `scripts/optimize_twm.py` regenerates
fresh fronts at the repo root, which are gitignored)*.

**🔑 Oracle-B diagnostic (2026-06-21) reframes the bass conclusion.** The synthetic
oracle-B ablation (`b_oracle_diagnostic`; `mobo-methodology.md` §8.2) shows bass
*separability* false-lock collapses **27% → 1.5%** when the true key is given its true
inharmonicity B. So the bass bottleneck is a **wrong-B template (the fixed Rigaud
prior), not a peak-domain scoring limit** — which recontextualizes the "Unified
conclusion" below: the deadzone / Duan / Emiya structural fixes failed on bass because
they tweak *scoring* while the actual problem is the template's *B shape*, which
constants and scoring terms cannot fix. **This triggers ADR 0005 revisit #3: a
second refinement dimension — joint (f₀, B) estimation in Stage B — is now the
highest-value bass lever, likely above any further constant tuning.** (Upper-bound
caveats: idealized/asymmetric oracle, separability not production, synthetic
understates the real one-directional B gap — see §8.2. Confirm on a real instrument.)

## Context

ADR 0001 specified MOBO tuning of the TWM constants; ADR 0005 specified split
discovery (Stage A discrete scan → Stage B basin-clamped scale refinement). This
ADR records what the empirical program actually found when those were built and
validated — including several course corrections, because the journey is the
evidence. The operational *how* (harness, synthetic signal model, objectives,
search config, selection protocol, and the threats-to-validity audit) lives in
[`docs/design/mobo-methodology.md`](../design/mobo-methodology.md).

The yardstick is the **original TWM** (default Maher & Beauchamp constants,
discrete, no refinement) on the one real instrument available: a 1–2-year-untuned
piano, 87 folder-labeled captures. Baseline: **71/87** correct locks.

## Findings (chronological — the corrections matter)

1. **K=88 was wrong.** Exhaustive refinement (every key) was adopted on the
   reasoning that it removes a magic number and matches canonical TWM. On the real
   captures it was the *worst* setting (61/87, bass 19→12): refining all candidates
   exposes the true key to dense-bass attractors and adjacent keys the discrete
   Stage-A filter otherwise excludes. **K is a robustness filter, not just a speed
   knob; small K (≈3) is correct.** K must be set/validated on *real* data — the
   synthetic under-represents the real attractor field (see also "what the synthetic
   can't price", below). Reverted to TOP_K=3.

2. **Objective B was gameable, then fixed.** The first objB (median-normalized
   margin) collapsed under error-scale *compression*: a low-λ regime drove the
   per-frame median toward the normalizer floor, inflating objB. MOBO exploited
   this. Replaced with an **ordinal** objB (mean fraction of the 87 impostors the
   true key out-scores) — rank-based, immune to scale inflation *and* compression.
   The re-run early-stopped cleanly with no degeneracy (floor 0.0%, tie 0.1%).

   ⚠️ **Corrected by the 2026-06-20 review (§8.3):** that "floor 0.0%" health check is
   **config-cherry-picked** — it holds at the *conservative* config but the
   error-collapse reappears at the *optimizer's own operating point*
   (`floor_frac=0.22` at p≈0.785, λ≈1.757). The ordinal objB didn't *remove* the
   collapse; it just stopped *seeing* it, so nothing steers the search away from it.
   The real defect is upstream in **objA** (it optimizes the K=88 regime and is
   minimized inside the collapse) — see the revised Finding #6 and §8.3.

3. **Cheap frozen-constant tests are not valid for error-landscape changes.** The
   "drop /N normalization" test (catastrophic, bass→0) and the "stretched
   reference" test (mild bass regression) were run with *default* constants frozen;
   for changes that alter the error scale or template positions, the old constants
   are mismatched, so the results don't evaluate the change's true potential. The
   "/N is the root cause" hypothesis (from the deep-research reviews) is **rejected
   as stated**: count-normalization is load-bearing (it makes different-partial-count
   candidates comparable); removing it just flips the bias from anti-treble to
   anti-bass.

4. **Refinement's value is real but detuning-gated.** On a bench with a realistic
   tuning-state distribution (tuned / service-drift / pitch-raise), false-lock by
   distance-from-ET shows refinement is neutral below ~35¢ and decisively better
   above it (discrete 90% vs refined ~36–68% at 55–78¢). This is why refinement
   looked useless on the real piano and the old synthetic — both lived mostly below
   the crossover. Refinement earns its keep for the **pitch-raise** use case.

5. **Pitch-raise recall is Stage-A-gated.** At K=3, a 70¢-off note ranks poorly at
   the (ET) Stage-A scan and drops out of the top-K before refinement sees it
   (refined 67.8% at 55–78¢ vs exhaustive 36.2%). Constants cannot fix this; it
   needs an architectural lever (larger K in the detuned regime, or a detuning-aware
   Stage A). Tracked as future work.

6. **The MOBO's synthetic optimum is a synthetic overfit; the robust Pareto point
   wins on real.** With p and λ free, MOBO (seed 42) converged on p≈0.8, λ≈1.5 (best
   *synthetic* separability, objA 0.205). On the real captures that regime only
   matches the conservative config by **trading bass for treble** (bass 15–16 vs 20)
   and is mode-fragile. The **conservative Pareto point** (p=0.5, λ=18, tuned
   q/r/ρ) — which the synthetic ranked *lower* — wins cleanly on real.

   ⚠️ **Corrected by the 2026-06-20 methodology review** (`mobo-methodology.md`
   §8.3/§8.11/§8.13): this finding is weaker than originally written. (a) The "p≈0.8,
   λ≈1.5 overfit" is **seed-specific** — at equal budget seeds 1/7 land at λ≈15,
   p≈0.65, so there is no single "the synthetic optimum"; the *value* of objA is
   seed-stable but the *location* is noise. (b) The synthetic optimum is also
   **degenerate** (`floor_frac=0.22`, the error-collapse regime), because objA in
   refine mode is the rejected K=88 argmin — so "best synthetic separability" is
   partly an artifact, not a real optimum. (c) The closing claim — "a single-objective
   optimizer would have returned only the overfit" — **does not hold**: objB varies by
   <0.001 across the whole Pareto front, so the search was effectively single-objective
   on objA anyway; what actually surfaced the robust point was a **wide band of
   near-objA-tied configs filtered by real-data validation**, which a single-objective
   top-N export would reproduce. The transferable lesson survives — *real-data
   validation over a plateau chose the constants* — but credit goes to the plateau +
   real gate, **not** to the multi-objective search or any synthetic optimum.

## Decision

Adopt the **conservative tuned constants** as `TwmConfig::default()`
(`p=0.5, q=3.88, r=1.426, ρ=0.298, λ=18`):

| param | M&B default | adopted | note |
| --- | --- | --- | --- |
| p (freq exponent) | 0.5 | 0.5 | unchanged — freeing it overfits |
| q (amplitude penalty) | 1.4 | ≈3.88 | **raised** (pinned near search ceiling) |
| r (reward) | 0.5 | ≈1.43 | **raised** (pinned near search ceiling) |
| ρ (reverse weight) | 0.33 | ≈0.30 | slightly lowered |
| λ (Duan ceiling) | 18 | 18 | unchanged — no analytic backing, but robust |

Discovery uses Stage A (K=3) → Stage B refinement (±80¢ window), per ADR 0005.

**Result — full decomposition against ONE baseline (the stated discrete original,
71/87).** Each register, no baseline-mixing:

| | total | bass (≤26) | mid (27–59) | treble (60–87) |
| --- | --- | --- | --- | --- |
| M&B discrete (original baseline) | 71 | 19 | **33** | 19 |
| M&B + refine | 70 | 19 | 32 | 19 |
| **Conservative + refine (shipped)** | **74** | **20** | **32** | **22** |

So shipped vs original is **+3 net = bass +1, mid −1, treble +3**. Honest reading of
the two effects, separated:

- **Refinement** (M&B-discrete→M&B-refine) is +0 net but costs **mid −1**: key 034
  (G3) refines into adjacent key 35. This is a Stage-B exposure cost, independent of
  the constants.
- **The constants** (M&B-refine→conservative-refine) are **+4 = bass +1, treble +3,
  mid 0** — no register regressed *by the constants*.

⚠️ **The +4 from the constants is concentrated in the manual-mode extreme treble.**
Apples-to-apples (both refined), the gained keys are 072 (A6), 082 (G7), 083 (G#7),
087 (C8), 004 (C#1), minus 070 (G6) — i.e. **four of five gains are A6–C8**, the
register this very ADR calls "near information-limited … manual mode is the accepted
fallback" (~2–5 partials, most exposed to the dense-treble attack archetype). The
**bass — the register the entire dense-attractor narrative is about — moved only +1.**
Selecting a config by one-piano pass-count where the margin lives in the flakiest
register is a real overfit risk: the deltas are deterministic (not run-noise), but
**one instrument cannot bound instrument-noise**, and best-of-N Pareto selection on a
single piano is itself a fitting procedure (see Threats in `mobo-methodology.md`).
**And it is not statistically significant:** the 2026-06-20 review ran a McNemar exact
test on the 6 discordant keys → **two-sided p≈0.22** (one-sided 0.11) — the headline
improvement is not distinguishable from a coin-flip on its own validation captures,
before instrument-to-instrument variance (which n=1 cannot estimate) is even
considered. A second instrument is required to give this any footing.

The transferable lesson survives but is narrower than first stated: the **amplitude
terms (q, r) want to be high for piano**, and the *bass* benefit (the design intent)
is a modest, single-key +1 — not the headline.

Residual failures (13) are octave / sub-harmonic confusions at the register
extremes — the class constant-tuning cannot fix.

## What the synthetic can / can't price

- **Can:** the metric constants, given a realistic spectrum/detuning distribution
  (the constants reweight existing structure).
- **Can't:** K and the refinement-window width (they govern exposure to the real
  attractor field, which the synthetic models least faithfully) — set these on real
  data. And it can't perfectly rank configs near the overfit frontier — hence
  decide final configs on real, never on synthetic hypervolume.

## Analytical findings

### Why "forgiveness"-type fixes fail (the deadzone derivation)

The forward per-term error is
`e_n = Δf_n·w_n·(1 + q·a_n/A_max) − r·(a_n/A_max)`, with `w_n = f_n^{−p}`.
The deadzone replaces the distance with `Δf̃_n = max(0, Δf_n − tol_n)`, so it can
only *reduce* error. Define a candidate's total deadzone reduction:

```text
R(X) = Σ_n min(Δf_n, tol_n) · w_n · (1 + q·a_n/A_max)
```

Locks are decided by the margin `E_I − E_T`; the deadzone shifts it by
`R(T) − R(I)`, so it helps *only if it forgives the true key more than the
impostor*. It does the opposite:

- **True key T:** partials align, every `Δf_n = ε` is a tiny residual (< tol_n), so
  `R(T) = Σ ε·w·(…) = E_pm,T` — its entire, already-small forward error. Bounded.
- **Impostor I:** predicts partials *near* peaks (forgiven) and in *gaps*
  (`Δf_n > tol_n`, capped reduction `tol_n`). So `R(I) ≈ N_gap · avg(tol·w)`, where
  `N_gap` = number of partials predicted where no peak exists.

`N_gap` is largest for **dense, low-f₀ candidates** (most predicted partials in
band → most gaps). So `R(I) − R(T)` is large and positive for dense-bass impostors,
and for the *borderline* attractors (already near-competitive via comb density) that
rebate exceeds the small original margin and **flips them into false locks** — the
measured monotonic, bass-driven degradation. This is the **same structural pathology
as the `/N` laundering and K=88 over-exposure: any tolerance/forgiveness mechanism
disproportionately rewards dense-spectrum candidates, because they have the most
surface area to be forgiven.**

### The deadzone never engaged its target (octaves)

For an octave-up candidate, predicted partial m sits at the note's harmonic 2m; the
inharmonic divergence is `Δ_oct(m) = 2m·f₀·[√(1+4Bm²) − √(1+Bm²)] ≈ 3·B·m³·f₀`
(small Bm²). The deadzone as **implemented** ([twm.rs](../../tuner-core/src/algorithms/twm.rs))
is `tol_n = c·B·n²·f_n/(2(1+Bn²)) ≈ (c/2)·B·n³·f₀` — note the factor of ½ from the
∂f_n/∂B propagation, which an earlier revision of this prose dropped. Their ratio is
therefore **`Δ_oct/tol = 6/c ≈ 43`** at c=0.14 — the octave divergence is ~43× the
deadzone, so it forgives ~2% of it. Octave discrimination is essentially untouched
(the earlier `3/c ≈ 21` understated this by 2×; the conclusion only strengthens).
(This corrects the "octave tension" worry carried earlier: the failure is the
bass-count rebate, not octave over-forgiveness.)

### Implication: the residual classes and their right tools

- **Octave/sub-harmonic confusion** — discriminating signal (inharmonic divergence)
  lives at *high* partials, which are down-weighted and noisy; both ways to chase it
  (deadzone, low-p) hurt. Near the **TWM-family scoring limit**; not constant-fixable.
- **Dense-bass attractor / pitch-raise sub-harmonic steals** — the `N_gap` channel.
  The principled fix is to **penalize** predicted-but-absent partials, not forgive
  them → Duan peak/non-peak likelihood (see `docs/design/duan-likelihood-design.md`).
- **Extreme treble (A7–C8)** — ~2–5 partials, information-limited; manual mode.

## Open / remaining optimizations (to complete before flipping to Accepted)

- [x] **Widen q/r search bounds** (q→[0,8], r→[0,3]) and re-tune — **done, no real
      gain.** q/r re-pinned at the new ceilings (q≈7.7, r≈2.97) and synthetic objA
      crept (0.258→0.242), but the real-capture result was *identical* to the old
      config (74/87, same per-register). Two conclusions: (a) **constant-tuning has
      hit its ceiling at 74/87** — the residuals need a different lever; (b) the
      config sits on a **broad q/r plateau** on real (q∈[3.9, 7.7] all give 74), so
      it's robust, not a sharp peak — good for shipping. The structural read: the
      optimizer drives the amplitude-weighted term to dominate and the bare
      `Δf·f^{-p}` term toward zero, i.e. for piano, amplitude-modulated matching
      discriminates and the un-weighted distance term contributes little. We do NOT
      widen further (saturated).
- [x] **n-kernel (forward-error B-deadzone)** — **done, rejected.** Implemented
      `tol_n = c·B·n²·f_n/(2(1+Bn²))` (the ∂f_n/∂B B-uncertainty; sane 3–8¢ widths).
      On real it hurt monotonically (c=0.14→66, 0.3→64, 0.5→62 vs 74 off),
      bass-driven. Reason is structural, not a tuning artifact: a deadzone forgives
      forward distance **symmetrically**, helping impostors as much as the true key,
      so it *lowers* discrimination — worst in the bass where dense candidates have
      the most partials to be forgiven on. The octave/sub-harmonic residuals need
      *sharper* discrimination, not more tolerance, so the deadzone is the wrong
      direction. (The opposite — up-weighting high-partial divergence — is low p,
      which we already saw hurts overall.) Caveat: this was a frozen-constant test
      (conservative q≈3.9); a co-tuned MOBO could differ, but the symmetric-forgiveness
      mechanism is unlikely to reverse. **Conclusion: octave discrimination is near
      the TWM-family scoring limit — the discriminating signal lives at down-weighted,
      noisy high partials.** Constant left in `TwmConfig` as default-off `b_deadzone`.
- [x] **Duan non-peak (count form)** — **done, rejected.** Implemented the
      principled inverse of the deadzone: a per-partial penalty (un-normalized count)
      for each predicted partial in the active band `[min_obs, max_obs]` with no peak
      within 2%. On real it **crushed bass monotonically** (c=0.05→bass 10, 0.2→5,
      1.0→0) — the structural-fail kill criterion. Root cause: **bass spectra are
      gappy *throughout*** (missing/weak/beating partials interleaved with real ones,
      not just below the fundamental), so the active-band gate can't spare them — a
      true bass note predicts many in-band partials with no peak and is charged as if
      hallucinating. **Meta-finding: the "predicted-but-absent" signal is fundamentally
      ambiguous in the bass — forgiving it (deadzone) lets dense impostors win,
      charging it (Duan count) crushes true bass notes; a count can't separate a true
      bass note's legitimate gaps from an impostor's hallucinations.** The only way
      forward would be a *detectability/amplitude-weighted* non-peak term (Emiya-style
      envelope), but the bass envelope is itself unreliable (few, jagged present
      partials) exactly where we'd need it — low expected payoff, high complexity.
      Constant left in `TwmConfig` as default-off `nonpeak_penalty`.
- [x] **Emiya amplitude-smoothness (the detectability-weighted non-peak)** — **done,
      rejected.** Penalized the amplitude *incoherence* of the MATCHED partials
      (Σ squared 2nd-differences of log-amplitude, gated to ≥3 matched) — designed to
      charge incoherent impostors while sparing a sparse-but-coherent bass note. On
      real it **also crushed bass monotonically** (s=0.3→66, 1.0→54, 3.0→35), and the
      effect was **λ-stable** (identical slope at λ=18 and λ=∞, so not a ceiling
      interaction). Root cause: the Emiya premise (true notes have smooth envelopes)
      is *violated by real piano bass* — beating (frame-to-frame amplitude
      modulation), soundboard dips, missing partials, and noisy single-frame FFT
      magnitudes make a true bass note's matched-partial envelope **jagged**, so the
      penalty charges the true note. It also leans on amplitude *fidelity* that the
      peak-detection/masking stage (deliberately out of scope) doesn't guarantee —
      re-coupling the two stages. Constant left in `TwmConfig` as default-off
      `smoothness_penalty`.
- [x] **Co-tuned structural MOBO arm** — **done (2026-06-23/24), both terms rejected on
      real.** Arms 6–7 freed `nonpeak`/`smoothness` with q/r/ρ co-tuned. `nonpeak`
      dominated on *synthetic* but **hurt bass on real even co-tuned** (77/87 bass 21 →
      68/87 bass 14); `smoothness` didn't help even on synthetic. The frozen-constant
      rejections thus hold — and for the right reason (real bass is B-limited, so
      charging absent partials crushes the mis-templated true note). See Status note
      and `mobo-methodology.md` §8.7.
- [~] **B-residual diagnostic** (0005 revisit #3) — **synthetic oracle-B done
      (2026-06-21):** giving the true key its true B drops bass separability FL
      **27%→1.5%** ⇒ bass is **B-limited, not peak-domain-limited** → joint (f₀,B) is
      the high-value bass lever. Real-data B-fit still pending (hard per Review 1; waits
      for the second instrument). See `mobo-methodology.md` §8.2.

  - **[NEW · 2026-06-27] Measured-B→discovery pathway built, and real-data validation
    REFUTES the synthetic promise (n=1).** The full pathway is now implemented: the
    Worker's measured per-key B (via the rewritten MAT) compiles to a discovery
    template (ET-centered, β-only — see below) and is handed to the live engine over
    the sanctioned crossing #4 (`ringbuf` SPSC; `02-cross-thread-communication.md` §4).
    Validation replayed the 87 real captures with each measured key seeded from
    `tuning_profile.json` (`test_engine_all.py --refine --profile`):

    | config | total | bass (≤26) |
    | --- | --- | --- |
    | Rigaud prior (baseline) | **74/87** | 6 fails |
    | measured B (all keys) | **73/87** | 8 fails |

    Net **−1**, and the oracle's predicted *bass* collapse did **not** appear — the
    **highest-ratio bass keys broke** (3/16/17 at **18–25× the prior**, s_win pinned
    toward the −80¢ basin edge), while the only clean gains were **treble** keys where
    the prior *over*-estimates B (70/81/84 at 0.3–0.8×). Cross-tab: fixed {6,34,70,81,84},
    broke {3,16,17,35,37,82}. This is coherent with the standing caveats, **not** a
    contradiction of the oracle ablation: (a) the synthetic oracle used true B only
    **~1.06×** prior in the bass — a 6% nudge — whereas the real MAT B is **7–25×**,
    so the real values are a far larger (and, on this out-of-tune upright with **no
    trusted B reference**, plausibly **over-estimated**) change; (b) the oracle was
    *asymmetric* (only the true key got perfect B) while production seeds **every**
    key, boosting dense bass impostors too (the §8.2(a) caveat made real).

  - **Decision: pathway SHIPPED but GATED OFF by default** — `pipeline::
    APPLY_MEASURED_B_TO_DISCOVERY = false`. The engine runs on the Rigaud prior; the
    Worker still measures/persists/displays B (only the *discovery-template* seeding is
    gated). This mirrors the ADR-0006 discipline of keeping unvalidated changes
    default-off. **Re-enable when a second instrument with a trusted B reference
    confirms the measured values** (the same standing gate). Flipping the one flag
    activates startup-load + live per-capture updates + the GUI `LoadProfile` sync.

  - **Template-construction choice (documented, holds regardless of the gate):**
    the template is **ET-centered with measured β only** — the measured `f0` is never
    a template input. β is a string-physical, tuning-invariant *shape* parameter and
    carries the entire oracle-B benefit; `f0` is the one quantity a tuner deliberately
    changes, so a stored measured `f0` is stale by construction (ET's worst-case error
    is bounded; a stale center's is not), and ADR 0005's coarse ET grid + ±80¢ basin
    clamp are defined around ET. Stage-B refinement (kept reference-free) absorbs the
    live detuning. Key identity stays by **key number**, never by frequency.

  - **Mechanism — why a "better" β *breaks* a key (β/f₀ orthogonality).** A template
    is `f_n = n·f0·√(1+β·n²)`: **f₀ is *scale*** (uniform multiplier), **β is *shape***
    (bends high partials up, ∝ n²). Stage B refines **scale only** — β is frozen — so
    **a wrong f₀ is recoverable (Stage B's job) but a wrong β is not.** Quantify the
    broken keys (3/16/17 at 18–25× the deep-bass prior ≈ 1e-4): at partial n=20 the
    prior puts it 2% sharp (`√(1+1e-4·400)=1.02`) while 25× β (≈2.5e-3) puts it **41%
    sharp** (`√(1+1.0)=1.41`) — for A0 that relocates partial 20 from ~11 Hz to ~225 Hz
    sharp. If that β is **over-estimated**, the template's upper partials sit far above
    the real peaks, the true key's forward error explodes, and Stage B — unable to fix
    *shape* with a *scale* knob — drags f₀ to the **−80¢ basin edge** trying to pull the
    over-sharp highs down (smearing the low partials in the process). Result: poor fit +
    garbage f₀ → the true key is out-ranked → false-lock. The `s_win` pinned to the
    basin edge is the *signature* of this shape-error-masked-by-scale failure. **A wrong
    β is strictly more dangerous than a wrong f₀.** Secondary push: symmetric all-key
    seeding sharpens the dense **sub-harmonic impostor** templates too (the octave-below
    key's even partials align with the struck note's), the §8.2(a) asymmetry made real.

  - **Confidence-gating is NOT a grounded fix (analysed, rejected as *the* solution).**
    A natural reflex — "only apply measured β when the measurement is confident" — has a
    *general* basis (reject-option, Chow 1970; uncertainty-weighted fusion / Kalman) but
    **no domain-specific grounding**, and a fatal hole here: our `confidence` is pairwise
    **coherence** = the partials agree *with each other*, i.e. **self-consistency, not
    accuracy.** This project already proved they diverge in the bass — the band-tightening
    experiment drove A#0 to **279× prior** on a *self-consistent but wrong* partial series
    (high coherence, wrong value; the correctness cross-residual *worsened* as the self-fit
    improved). So a confidence gate would *pass* exactly the confident-but-over-estimated
    bass keys that break discovery. A **sanity ceiling** (reject β > k×prior) is worse — it
    presumes the prior, which **begs the question** (the prior is known-wrong in the bass).
    Confidence-gating is at best a partial *downside guard* (catches incoherent cases) — and
    its input isn't even plumbed: `KeyMeasurement` carries no confidence field. It is **not**
    the fix and **not** known to help.

  - **The fix path (what "solving this" requires).** The core unknown is *accuracy*:
    on one out-of-tune upright with no ground truth, "is the deep-bass 18–25× **real** or
    **over-estimated**?" is unanswerable, and the whole sign of the result hangs on it.
    (1) **Get a trusted B reference** (see validation strategy below) — the linchpin.
    (2) **If MAT over-reads bass B → repair the estimator** (likely bass partial-association
    under missing-fundamental / parallel-string divergence). (3) **If MAT's bass B is
    accurate → the realizable fix is *asymmetric* application = the Stage-B joint-(f₀,B)
    step (ADR 0005 revisit #3 / "Prompt 3")**: refine β **per-candidate**, prior-regularized,
    so the true key benefits under its real β while a wrong-β impostor is *penalized* by the
    regulariser — the one form that doesn't feed the impostors. (4) Confidence/sanity gating
    only as a guard alongside, never the core fix.

  - **Validation strategy — "trusted B reference" ≠ "second instrument" (they answer
    different questions).** A second instrument tests whether *discovery behaviour /
    measured-B-feedback* **generalises** across pianos; it does **not** validate B
    *accuracy* (it's just a second run of the same estimator). The strong, available
    accuracy test is **synthetic-truth recovery**: the synthetic (`mobo_evaluator`)
    generates spectra from a **known** `b_actual`, so MAT's recovered B can be scored
    against ground truth (we have only run `validate_mat` on *real* so far). The valuable
    form is a **stress test, not a plain pass**: the existing synthetic's B is prior-centred
    (~1×), so it doesn't probe the doubted regime — *sweep a known B from 1× to ~25× the
    prior on bass-like spectra with missing fundamentals + parallel-string spread* and check
    whether MAT tracks the true B or diverges. This directly probes the suspected failure
    mode (partial **mis-association** when the fundamental is absent → lock onto a
    self-consistent *wrong* series, cf. the A#0→279× band-tightening case). **Necessary but
    not sufficient:** it can *refute* (MAT can't recover known B ⇒ estimator bug) but can't
    fully *confirm* real-bass accuracy (real bass may carry hazards the synthetic lacks).
    **Discounted alternatives (analysed, not pursued):** *independent-estimator triangulation*
    is weak here — PFD/MAT share the peak-association front-end, so they'd mis-associate (and
    agree) the *same* way on a missing fundamental; only a *different-front-end* estimator
    (the comb-filter, no discrete numbering) would be informative, and standing one up isn't
    worth it now. *Physical computation* (`B = π³Ed⁴/(64L²T)`) is impractical without a lab,
    and the plain-string formula breaks for the **wound** bass strings anyway (winding adds
    mass, not stiffness). Public corpora (MAPS, MAESTRO) carry **note-level** truth but **no
    B label** — measuring B from them is just another estimator (useful for *breadth*, not
    as independent truth).

  - **[NEW · 2026-06-29] Synthetic-truth recovery DONE — estimator REFUTES the
    mis-association hypothesis; the deep-bass readings are accuracy-sound (the fix is
    Prompt 3, not estimator repair).** Built `tuner-core/examples/mat_b_recovery.rs`: it
    synthesizes **time-domain** signals (the `gen_frame` physics — partials at
    `f_n=n·f0·√(1+Bn²)`, `n^−α` envelope, graded missing fundamentals, detuned unison
    strings, sub-harmonic decoy, white noise) and runs the **exact** Worker path (two Hann
    FFTs @ 65536 → `cspe` → serial+simultaneous `detect_pitch_mat`), scoring recovered B
    against the known B. (MAT consumes a magnitude spectrum + CSPE map, *not* a peak list,
    so the existing peak-domain `gen_frame` frames cannot be fed to it — rendering through
    real FFT leakage + CSPE is what makes the recovery honest.) Findings:

    | condition (A0/C1 unless noted) | recovered/true B | within ±20% |
    | --- | --- | --- |
    | Baseline (prior B, bass/mid/treble) | 1.00× | 100% |
    | **B sweep 1×→25× prior, full partials** | **1.00× at every step** | **100%** |
    | **B sweep 1×→25×, missing fund. (1–3) + decoy** | **1.00× at every step** | **100%** |
    | Parallel-string (2 str, 12¢, n→32) | 1.00× (self-resid ↑, B unbiased) | 100% |
    | Deep missing-fund., lowest present partial ≤5 | 1.00× | 100% |
    | Deep missing-fund., lowest present partial 6–12 | 1.2–1.5× **with self-resid ↑↑** | 0–4% |
    | **f₀ seed off by ≤±10%** | **1.00×** | **100%** |
    | f₀ seed octave (×2 / ×0.5) | **4.0× / 0.25×, self-resid LOW** | 0% |

    **Verdict (the gate):** MAT recovers known B to **<1%** across the entire doubted
    regime — B to 25× prior, missing fundamentals, parallel strings — **provided the f₀
    seed is within ~±10% of true**. So missing fundamentals do **not**, by themselves,
    cause mis-association; the surviving high partials are correctly numbered off an
    accurate seed and the median nails B. The **only** way to manufacture a large
    *self-consistent* error is an **octave seed error** — ×2 yields exactly 4× B (every
    other partial of a stiff string is itself a stiff series with f₀′=2f₀, B′=4B), with a
    **low** self-fit residual (so confidence/coherence *cannot* catch it — the §"confidence
    is self-consistency not accuracy" point, now demonstrated against ground truth, and the
    same shape as the A#0→279× band-tightening case). Non-octave seed/numbering errors
    over-read only mildly (≤~1.5×) and carry a **high** self-residual flag.

    **Cross-check on the real captures (read-only, validation-only — no recalibration):**
    every bass key's Goertzel seed (`measured_f0` in `diagnostics/key_*/analysis.json`) is
    within **±2.6% of ET** (seed/ET ∈ [0.985, 1.026]) — **none near 0.5× or 2×**. The
    upstream tracker did *not* octave-jump, even on A0 with its missing fundamental. So the
    real 7–25× readings sit on the **robust plateau**, not in the octave-artifact zone, and
    a missing-fundamental over-read caps at ~1.5× — far short of 25×. **The deep-bass B is
    therefore a real measurement, not a mis-association artifact**, which means the
    measured-B→discovery regression (`APPLY_MEASURED_B_TO_DISCOVERY=false`) is the
    **impostor-symmetry / β-orthogonality** problem (the Mechanism bullet above), *not* an
    estimator-accuracy problem ⇒ **the fix path is item (3), asymmetric per-candidate
    (f₀,B) refinement ("Prompt 3"), which is now well-founded; estimator repair (item 2) is
    not needed.**

    **Caveat (necessary-but-not-sufficient, as designed):** this *refutes* the octave/
    missing-fundamental mis-association mechanism on a ground-truthed estimator; it cannot
    *fully confirm* real-bass accuracy — the synthetic may lack hazards real wound bass
    strings carry (per-partial B drift, soundboard formant amplitude distortion, structured
    noise). It does not relax the standing discipline (real captures validation-only; no
    synthetic-recalibration-to `calculated_b`). The standing **trusted-B-reference / second
    in-tune instrument** action is unchanged — but its *expected* result is now "confirms
    the readings", and the bass lever is confirmed to be a *discovery-application* problem,
    not a *measurement* one. Reproduce: `cargo run --release --example mat_b_recovery`.

  - **[NEW · 2026-06-30] Prompt 3 (asymmetric, prior-regularized, per-candidate (f₀,B)
    Stage-B refinement) BUILT as an offline diagnostic and REFUTED on real — scoring-time
    B is not a realizable bass lever; hot-path port (3b) NOT pursued.** Built
    `tuner-core/examples/joint_b_refine_diagnostic.rs`: Stage A is unchanged (prior
    templates, top-K recall identical to baseline), but Stage B refines β **per top-K
    candidate** on a log-grid within ±n·σ_B of *that candidate's* Rigaud prior, with a
    quadratic log-β regularizer `γ·d²` (d in σ_B units), ranking candidates by the
    **regularized** error so a wrong-β impostor pays the regulariser. The fixed-β baseline
    is the n_σ=0 special case (asserted byte-identical to `discover()`); the harness also
    decomposes the TWM forward/reverse error to watch the octave discriminator. Three
    measurements — synthetic (`gen_frame`), a bass octave/sub-harmonic stress, and the real
    captures (the exact gatekeeper → 3-frame-lock path; **baseline reproduces 74/87
    per-register: bass 20/26, mid 32/33, treble 22/28**, validating the harness).

    | policy (real captures) | total | bass | mid | treble | bass-oct lock-frames | vs baseline |
    | --- | --- | --- | --- | --- | --- | --- |
    | fixed-β (baseline) | **74/87** | 20/26 | 32/33 | 22/28 | 129 | — |
    | joint nσ=1–2, γ=2–8 (tight+reg, **shippable**) | **74/87** | 20/26 | 32/33 | 22/28 | 129 | **0 fixed / 0 broke** |
    | joint nσ=2, γ=0 (unreg, tight) | 74/87 | 20/26 | 33/33 | 21/28 | 130 | 2 fixed / 2 broke (treble churn) |
    | WIDE nσ=20, γ=0 (β ≤ 23×, reaches real bass B) | **72/87** | 20/26 | 32/33 | 20/28 | **102** | 3 fixed / **5 broke** |
    | WIDE nσ=20, γ=2 (reg) | 74/87 | 20/26 | 32/33 | 22/28 | 129 | 0 / 0 (reg pins β→prior) |

    **The bind is structural, and the diagnostic makes it quantitative.** (a) At
    **safe/shippable** settings (tight bound + a regulariser strong enough to matter) the
    result is **byte-for-byte the fixed-β baseline** — on synthetic because the true B sits
    at **1.06× prior** (nothing to correct; the reg-pull table shows β_chosen = 1.00×, 0%
    pinned), and on real because the ±2σ bound (±31%) **cannot reach** the deep-bass 7–25×
    gap. Either way the lever is **inert**. (b) The **only** way to let β reach the real bass
    value is the **WIDE** probe, and it *does* engage the lever — it **fixes A#0** (a known
    deep-bass octave-confusion key) and drops bass octave lock-frames **129→102 (−21%)**,
    independently re-confirming bass is B-limited — but it **nets −2 on real (74→72)**, the
    bass gains outweighed by treble/upper-mid breakage, and on synthetic it drives the octave
    **forward-margin NEGATIVE (+0.106 → −0.023)**: the measured (2f₀,4B) forward-collapse
    (`mat_b_recovery`'s even-partial identity), with only TWM's **reverse error** (the true
    note's odd partials; ρ·rev-margin ≈ 0.55) still holding the total positive. (c) A
    **regulariser cannot separate the two cases**, because the separation criterion
    (distance-from-prior) is *backwards in the deep bass*: the **true** bass key is the
    **most distant** deviator (the prior is known-wrong there), so any γ strong enough to
    penalise an impostor penalises the true key *first* — γ=2 pins **everything**, including
    a true 25× key, back to prior (WIDE-γ=2 = baseline exactly). This extends the ADR's
    "confidence-gating presumes the prior / begs the question" analysis to **prior-
    regularisation itself**: prior-regularised per-candidate B is **not** "the one form that
    doesn't feed the impostors" (the fix-path (3) hypothesis above) — at scoring time,
    *"let the true bass key reach its real B"* and *"let an impostor reach a flattering B"*
    are **the same knob**, and the prior cannot tell them apart where it is itself wrong.

    **Decision (gate as written in the prompt): STOP — do not port to the hot path (3b).**
    Both stop-conditions are met: it **wins on synthetic only at the cost of feeding octave
    impostors** (WIDE) and is **inert (no real gain)** at safe settings — never *beats*
    fixed-β on real. The bass-B lever is **not realizable via scoring-time B**; a real bass-B
    fix, if pursued, needs an application path whose asymmetry is **not** distance-from-prior
    (e.g. apply a *trusted, per-key* measured B only to that key's own template — which still
    requires the standing **trusted-B-reference / second instrument** linchpin, unchanged).
    Estimator repair (item 2) remains not-needed (`mat_b_recovery`); fix-path item (3) is
    now **tested and rejected as a scoring-time mechanism**. Reproduce:
    `cargo run --release --example joint_b_refine_diagnostic`.
- [ ] **Stage-A recall / pitch-raise** (deprioritized) — larger K in the detuned
      regime or a detuning-aware Stage A. **Loops back into the K-vs-attractor bind**:
      widening K to keep a detuned true key re-admits the dense-bass attractors the
      small-K filter exists to exclude. Limited headroom (the pitch-raise subset
      only). Revisit after Duan, since Duan may suppress the attractors enough to
      make a larger K safe.

    ⚠️ **[2026-07-02] The "revisit after Duan" condition is stale** — the Duan(-count)
    term was co-tuned-tested and rejected on real (2026-06-23/24, Status note), so it
    will not be suppressing any attractors. Successor plan: (1) a **Stage-A rank
    diagnostic** (log the true key's scale-1.0 rank and top-K membership per frame).
    Note Findings #4/#5 already predict its answer on the *current* captures (they live
    below the ~35¢ crossover → true key nearly always in top-K; the gate binds only
    >55¢), so it is only informative on **detuned / pitch-raise captures** — run it when
    a second or deliberately detuned instrument exists. (2) For V1, **manual mode covers
    pitch-raise** (the user names the key; Stage-A recall is not involved — verify the
    Goertzel tracker and cent meter tolerate a string 100¢+ flat); auto-mode pitch-raise
    remains the gated item.
- [ ] **Window-width sweep** (adjacent-theft ↔ pitch-raise-reach tradeoff;
      currently a fixed ±80¢ compile-time constant).
- [ ] Decide robustness-aware selection (regularize toward analytic priors) vs the
      current "pick the robust Pareto point by real validation".

### Corrections & queued verifications (2026-07-02 review pass)

A fresh-eyes audit of the program record surfaced five items, recorded here so the
manual-mode pivot does not lose them:

1. **The structural-term tests used proxies, not faithful ports.** The tested
   "Duan non-peak" is an **unnormalized hard-tolerance count** (2% match band,
   active-band gate), not Duan Eq. 7's non-peak-region *likelihood*; the tested
   "Emiya smoothness" is a matched-partial log-amplitude 2nd-difference, not
   Emiya's model. The rejections therefore read precisely as: *the ideas, in proxy
   form, rejected under the valid co-tuned protocol.* A faithful-likelihood
   scoring objective remains **untested** — deferred, low expected upside (the
   octave identity is in the data, and the proxies' bass failure mechanism is
   structural), gated with everything else on the second instrument.
2. **⚠️ SUPERSEDED by the 2026-07-05 Prompt A′ re-derivation below — measured on
   biased (pre-jacobsen-fix) peaks; t1898 no longer beats the default on fixed
   peaks.** The 77/87 candidate's exact parameters were never pinned — RESOLVED
   (2026-07-02): pinned to seed-7 trial 1898.** All ten nonpeak≈0.047 trials in
   `docs/mobo-archive/mobo_run4_backup/twm_pareto_arm6.json` were swept via
   `validate_config.py --refine --config "0.5 q r rho 18"` (nonpeak off) against
   the real captures (baseline re-reproduced first: 74/87, bass 20/26, mid 32/33,
   treble 22/28, same 13 fails). Exactly one trial reproduces 77/87 bass 21:
   **seed-7 trial 1898, q≈7.68116, r≈2.90783, ρ≈0.48664** (p=0.5, λ=18) — *not*
   the previously guessed trial 1360 (q≈7.86, ρ≈0.50), which scores 76/87. Full
   sweep: low-q trials (q≈2) score 70–72 with **bass collapsing to 14–15**;
   high-q/high-ρ trials (t1360/t1722/t1885) cluster at 76 — so 77 sits on a
   high-q/high-ρ *shoulder*, +2/+3 over baseline, not a knife-edge point.
   Per-register for t1898: **77/87 = bass 21/26, mid 32/33, treble 24/28**.
   Vs the 74/87 baseline: **fixes 005 (D1), 070 (G6), 084 (A7), 085 (A#7);
   breaks 082 (G7)** — so the net +3 is again treble-heavy (2 of 4 gains in the
   extreme-treble A7–C8 band, and the one break is also extreme treble; bass +1
   is the only non-treble move, but 21/26 is the best measured bass of any
   config). Notably 3 of the 4 fixed keys are the *early-wrong-lock* class of
   item 5 below (070/084/085), and 005 was stable-wrong — the constants sharpen
   the attack-transient race, they don't just re-rank the steady state.
   **Pitch-raise reach does NOT degrade:** the 1¢-resolution key-40 sweep
   (`examples/pitch_reach_sweep.rs`, harness validated — reproduces canonical
   78¢ / conservative 69¢ exactly) gives t1898 **80¢** (+reach; −82¢ down) —
   the high-q/high-ρ shoulder *recovers* the conservative config's ~9¢ reach
   loss rather than worsening it (the low-q trials drop to 59–61¢). **Still NOT
   adopted** — standing reason unchanged: +3 on n=1 is the McNemar-p≈0.22 class
   of evidence, and best-of-N selection on one piano is itself a fitting
   procedure. Queued for second-instrument validation with the exact triple now
   pinned.
3. **Auto-capture provenance risk (data integrity for the tuning curve).** In auto
   mode the Worker's f₀ seed is the *discovery lock*; an octave false-lock (the
   residual failure class) makes MAT confidently measure the wrong key's series —
   the (2f₀, 4B) identity with a LOW self-residual (`mat_b_recovery`), so
   confidence cannot catch it — and persist it under the wrong key in
   `tuning_profile.json`. Discovery is insulated (the gate is off); the persisted
   profile is not. **Rule: the tuning curve consumes manual-mode captures only**,
   until a provenance flag (e.g. `captured_in_auto`) exists on `KeyMeasurement`.
   *Amended 2026-08-13:* the flag exists, and the rule now has a second axis on
   the same footing — a capture whose declared strings are not the note's full
   unison measured one string, not the note. Both live in
   `KeyMeasurement::is_trusted`; see
   [`docs/internals/06-capture-sets.md`](../internals/06-capture-sets.md). No
   capture predating the declaration is affected.
4. **MAT's `confidence` is not part of DAFx-09 (decision recorded).** The paper
   outputs only (f₀, B); the median is its robustness mechanism. Our
   coherence×evidence scalar is a bespoke addition. Decision: it stays a **runtime
   diagnostic only** (UI display), is **not** persisted into `KeyMeasurement`, and
   any tuning-curve weighting will follow whatever the chosen curve method's paper
   prescribes (Hinrichsen consumes spectra, Sethares measured partials — neither
   needs it), with robust fitting handling outliers.
5. **⚠️ SUPERSEDED by the 2026-07-05 Prompt A′ re-derivation below — measured on
   biased (pre-jacobsen-fix) peaks; failure sets changed.** Flicker-vs-stable
   failure diagnostic — DONE (2026-07-02): the failures split
   three ways, and the largest class is one the question didn't anticipate.**
   Per-frame stable-winner sequences (`peaks.csv` `key_idx` × `gatekeeper.csv`
   Stable, the validate_config.py merge) for the 13 baseline failures:

   | class | keys | signature |
   | --- | --- | --- |
   | **Stable-wrong** (5) | 000, 001, 005, 010, 012 — all deep bass | one wrong winner takes ≥71% of *all* stable frames; true key wins ≤1 frame. Zero decision-level headroom — the B-limited class. |
   | **Early-wrong-lock** (6) | 034, 070, 080, 081, 084, 085 | a wrong key wins only the first few attack-transient stable frames (first 3-run at stable-frame index 2–17) and locks; the **true key then wins the plurality of all stable frames (38–89%) with runs of 5–39**. First-to-3 loses a race the note body wins — full-window plurality voting alone would flip all 6. |
   | **Genuine flicker** (2) | 006, 086 | true key wins 10–29% of frames but never 3 consecutively; a wrong winner holds plurality. Winner-voting can't flip these; score-level averaging is uncertain. |

   So the evidence-accumulation idea is **NOT closed** — 6 of 13 failures (incl.
   4 of the 5 extreme-treble fails) are decided in the attack transient while the
   steady state stably favors the true key. Headroom is real but bounded: ≤6 keys
   (+2 uncertain), none of them bass. Per the decision gate a short design note
   exists (`docs/design/sequential-detection-design.md`, faithful
   sequential-detection framing — SPRT/M-of-N); **nothing is built**, and the
   interaction with item 2 matters: the pinned t1898 constants *already* fix 3 of
   the 6 early-lock keys (070/084/085), so the two levers overlap — re-run this
   classification under whichever config survives the second instrument before
   sizing the accumulation win. (Already measured under t1898, same method: its
   10 fails = 4 stable-wrong bass (000/001/010/012) + 2 wrong-plurality
   (006/086) + **4 early-wrong-lock with true-key plurality (034/080/081/082)**
   — including 082, the one key t1898 breaks, which locks wrong at stable index
   13 and then the true key runs 27 straight. So under either config the
   accumulation headroom is ~4–6 keys and the levers are complementary, not
   redundant.)

**⚠️ Supersedure note (2026-07-02, evening).** The faithfulness audit
(`docs/audits/faithfulness-audit-03-jacobsen.md`) found and fixed a real bug in
Discovery's sub-bin peak estimator (a bespoke (−1)^m correction plus a missing
Candan c_N ≈ 2 factor ⇒ error ≈ −2.5·δ bins on **every** Discovery peak, up to
±7.3 Hz at 8192). **Every real-capture number in items 2 and 5 predates the
fix.** On fixed peaks the *shipped default* config scores **76/87 discrete /
77/87 refined** (bass 21/20, mid 33, treble 22/24) — i.e. the default now ties
t1898's old 77/87. Item 2's ten-trial sweep, item 5's failure classification
(the failure *sets* changed; mid is now perfect in both modes), and the
pitch-raise-reach figures must be re-derived on fixed peaks — queued as
**Prompt A′** (done 2026-07-05; results in the [NEW · 2026-07-05] entry
below). The fix is a correctness change
(paper-justified, synthetically verified, regression-tested), not a parameter
selected on the captures, so shipping it does not violate the n=1 rule.

**[NEW · 2026-07-05] Prompt A′ re-derivation DONE — the picture materially
changed: t1898 no longer wins; the failure-mode split survives qualitatively.**
All four Prompt A′ tasks re-run mechanically on fixed peaks
(`scripts/validate_config.py`, `examples/pitch_reach_sweep.rs`, the
peaks×gatekeeper classification method). Standing constraints held (n=1,
validation-only, nothing adopted).

- **Baselines re-confirmed exactly as the audit-03 doc recorded:** DISCRETE
  **76/87** (bass 21/26, mid 33/33, treble 22/28); REFINED **77/87** (bass
  20/26, mid 33/33, treble 24/28). Failure sets: discrete —
  000/004/005/010/012 (bass) + 080/081/082/084/085/086 (treble); refined —
  000/001/002/005/010/012 (bass) + 080/082/086/087 (treble).

- **Task 1 (arm-6 nine-trial re-sweep, `--refine --config`, nonpeak off).**
  Swept all nine nonpeak≈0.047 trials from `twm_pareto_arm6.json`
  (t1108/t1262/t1360/t1509/t1532/t1722/t1782/t1885/t1898) against the fixed-peak
  captures:

  | trial | q | r | ρ | total | bass | mid | treble |
  | --- | --- | --- | --- | --- | --- | --- | --- |
  | t1108 | 4.804 | 2.992 | 0.386 | 73 | 20/26 | 33/33 | 20/28 |
  | t1262 | 1.968 | 2.908 | 0.192 | 74 | 17/26 | 33/33 | 24/28 |
  | t1360 | 7.855 | 2.908 | 0.502 | 75 | 21/26 | 33/33 | 21/28 |
  | t1509 | 1.968 | 2.629 | 0.192 | 74 | 17/26 | 33/33 | 24/28 |
  | **t1532** | 2.954 | 2.975 | 0.272 | **77** | 20/26 | 33/33 | 24/28 |
  | t1722 | 7.604 | 2.646 | 0.422 | 74 | 20/26 | 33/33 | 21/28 |
  | t1782 | 7.096 | 2.975 | 0.386 | 73 | 19/26 | 33/33 | 21/28 |
  | t1885 | 5.903 | 2.621 | 0.421 | 72 | 19/26 | 33/33 | 20/28 |
  | **t1898 (previously pinned)** | 7.681 | 2.908 | 0.487 | **75** | 20/26 | 33/33 | 22/28 |

  **t1898 no longer beats the default — it is now 2 keys worse** (75 vs 77).
  Full cross-tab vs the default *(corrected same day; the first A′ pass listed
  only three broken keys)*: t1898 **fixes 012 (A1) and 087 (C8)** but **breaks
  003 (C1), 079 (E7), 084 (A7), 085 (A#7)** — net −2, and the fixed-peak
  default already recovers 084/085 on its own, so t1898's old edge there is
  gone. **Only t1532 ties the default at 77/87**, with an *identical
  per-register split* (20/33/24) but a **different, lateral failure trade**:
  default fails 087 (C8), t1532 fails 076 (C#7) instead — both extreme/upper
  treble, net zero. **No config in the swept set beats the new default.**
  Mechanism, sharpened by a harness check: `mobo_evaluator`'s synthetic peaks
  carry an **unbiased ~N(0, 0.2 Hz) jitter stand-in** (`emit_partial_cluster`)
  and never route through `extract_peaks`/`jacobsen` — so the −2.5·δ bias was
  **never in the synthetic search layer, only in the real-capture selection
  layer**. The arm-6 "77/87 side-finding" was therefore best-of-N selection
  fitting the *biased* real peaks; fix the peaks and the selection evaporates
  (75/87), while the plateau-chosen conservative default — picked for
  robustness, not for topping the biased ranking — *gains* +3 from the same
  fix. This is the ADR's own "best-of-N selection on a single piano is itself
  a fitting procedure" warning, now **measured**: the selection did not even
  survive a front-end correctness fix on the *same* piano, let alone an
  instrument change. (Corollary: audit-03's "bias was present in both the
  synthetic MOBO harness and the real captures / self-consistent" line was
  wrong about the synthetic half — corrected there. The MOBO *synthetic*
  fronts were never bias-contaminated; only the real-selection step was, and
  A′ has now redone that step on fixed peaks.)

- **Task 2 (pitch-raise reach, key 40, 1¢ resolution).** Unaffected by the fix
  by construction: `pitch_reach_sweep.rs` feeds `discover()` synthetic peaks
  built directly from profile partials (`synth_peaks`), never routing through
  `extract_peaks`/`jacobsen` — so these numbers were never stale, and the
  re-run reproduces the old canonical/conservative figures exactly (78¢/−82¢,
  69¢/−78¢), confirming the harness is peak-estimator-independent. Newly
  measured: **t1898 80¢/−82¢** (unchanged from before — same reason), **t1532
  72¢/−82¢** (better than default's 69¢, worse than t1898's 80¢). Since t1898
  no longer wins on real captures, its reach edge is now moot for adoption;
  t1532 (the only real-capture tie) gives a smaller reach improvement over
  default (+3¢) than t1898 did (+11¢).

- **Task 3 (flicker-vs-stable, per-frame `peaks.csv`×`gatekeeper.csv`, default
  config, both modes).** Re-classified on the new failure sets:

  | mode | stable-wrong | early-wrong-lock | genuine-flicker |
  | --- | --- | --- | --- |
  | discrete (11 fails) | 010, 012 (100% wrong); **005 borderline** (67% wrong, true 18%, brief run of 4) | 000, 004, 080, 081, 082, 084, 085 (7 keys; true plurality 36–60%, longest runs 8–21, wrong locks early at stable-idx 0–15) | 086 (true 10%, longest run 2, no single wrong dominates — fragmented plurality) |
  | refined (10 fails) | 005 (71% wrong — exactly the historical bar), 010, 012 (100% wrong) | 000, 001, 002, 080, 082, 087 (6 keys; **000/001 are near-tied races**, true/wrong within 2pp of each other, decided by which locks 3-in-a-row first; 002/080/082/087 are clean true-plurality cases with long runs 7–26) | 086 (same signature as discrete) |

  **The premise survives and is stronger than pre-fix — including, new, in
  the bass** *(corrected 2026-07-05, same day: the first A′ pass summarized
  this as "still none bass", contradicting its own table — 000/004 are bass
  keys)*: 7 (discrete) / 6 (refined) early-wrong-lock keys vs. the pre-fix 6.
  The mechanism is unchanged (a wrong key wins the attack-transient race at
  stable idx 0–15; the true key wins the note body), but the class membership
  moved — 070 now passes outright, 034 left the failure set — and **now
  contains deep bass**: discrete **000 (A0)** true-plurality 36% vs 32% and
  **004 (C#1)** 45% vs 39%; refined **002 (B0)** true-plurality 52% with the
  impostor's lock being exactly one 3-frame run at stable idx 9, plus
  **000/001 as near-tied races** (true vs wrong within 2 pp; 001's wrong
  winner leads by a single frame). Pre-fix the classification concluded "none
  bass — this is not a bass lever"; on fixed peaks that **no longer holds** —
  consistent with the removed bias (≤±7.3 Hz per peak) having been
  proportionally worst against bass partial spacing, so the fix made the true
  deep-bass keys frame-competitive. Gross plurality-flip ceilings (before
  pricing the symmetric-exposure risk on currently-passing keys — the design
  note's offline replay prices that for free): discrete 76→**83** (all 7
  flip); refined 77→**81** (002/080/082/087 flip cleanly; 000 is a dead tie
  and 001 flips the *wrong* way by one frame — those two need windowed or
  score-level rules, not plain plurality). The hard stable-wrong core shrinks
  to 005/010/012 (+ 086 flicker) — still the B-limited class.
  `docs/design/sequential-detection-design.md`'s premise section needs its
  key lists refreshed (pointer added) but its Decision (build nothing;
  offline M-of-N replay first when picked up) stands.

- **Task 4 decision gate.** Item 2: **77/87 as "the candidate" no longer has a
  standing nominee** — t1898 is refuted on fixed peaks (net −2 vs. default);
  t1532 merely ties the default with a lateral trade, so there is **no config
  in the swept set worth queuing for second-instrument validation** at this
  time (a fresh MOBO re-sweep on fixed peaks, not just a re-scoring of the old
  synthetic-derived candidates, would be needed to find one, and is not
  queued — no evidence it would pay for the cost; and per the Task-1 mechanism
  note, the synthetic search layer was never bias-contaminated, so the
  existing fronts are not stale — only the real-selection layer was, and A′
  has redone it). Item 5: **not closed** — headroom persists, is larger than
  pre-fix, and now includes deep-bass keys (Task 3);
  `sequential-detection-design.md` stays "build nothing yet", but its gate 1
  ("re-run the classification under whichever config survives") is now
  **degenerately resolved**: no constants candidate survived A′, so the
  surviving config is the shipped default and the re-classification under it
  is exactly Task 3 above. That makes the **offline M-of-N replay** (pure CSV
  post-processing, zero engine changes) the next experiment in that thread
  whenever it is picked up — prototype-and-price only; M/N must not be
  finalized on n=1.

- **[2026-07-19] Item 5 update — the offline M-of-N replay has been EXECUTED,
  and the support gate is met on two instruments.** Full record:
  [`ADR 0010`](0010-m-of-n-lock-rule-replay.md); protocol (pre-registered):
  `sequential-detection-design.md` "Replay protocol". Headlines: piano-2's 595
  repeat dumps were replayed as the second lock-accuracy instrument (baselines,
  first on record: discrete 533/595, refined 530/595 — same ~7 pp plurality
  headroom as piano-1, the early-wrong-lock mechanism transfers). Concordant
  plateau recommendations: **refined (M=7, N=8)** — piano-1 77→**81** (= its
  strict plurality ceiling), piano-2 530→**568** (+44/−6); **discrete (M=8,
  N=8)** — piano-1 76→**82** (+6/0), piano-2 533→**561** (+35/−7). Median
  added latency 4–5 stable frames (~100 ms), inside the user's ≤0.5 s budget.
  The n=1 finalization concern above is answered by the two-instrument
  concordance protocol (broad plateaus, no isolated spikes). Deep-bass fixes
  materialized as Task 3 predicted (piano-1 refined 001/002 fixed); the
  stable-wrong core (000 dead tie, 010/012) stays failed as pre-registered —
  still the B-limited class. Next step is a hot-path design note (acquisition
  **plus** lock-release semantics — the replay is acquisition-only); nothing
  built, MSPRT stays gated.

- **[2026-07-20] Item 5 IMPLEMENTED (v1, acquisition-only) — Prompt M.** The
  M-of-N rule now ships in the hot path: `Engine::process`'s auto discovery
  path replaces the 3-consecutive counter with `Engine::record_stable_winner`
  (a fixed N-slot ring buffer of Stable-frame winners; lock the first key to
  reach M votes), refined default **(M, N) = (7, 8)**. Votes accrue on
  gatekeeper-Stable frames only (the engine now takes an `is_stable` flag) and
  reset on onset/silence/bypass — the exact semantics the replay validated. The
  Python lock replicas (`validate_config.py`, `test_engine_all.py`) were updated
  to the same rule (`--lock-m/--lock-n`, defaults 7/8; 3/3 reproduces the old
  rule) and reproduce ADR 0010 as the known-answer gate: refined 3/3→77, 7/8→81
  on `diagnostics_piano_1` (failure set 000/003/005/010/012/086 exactly);
  discrete 8/8→82; piano-2 cache 568/561. Release/re-lock hysteresis is the
  deferred second design (ADR 0010 Limitations); the stable-wrong core stays
  failed. Not committed by the implementing chat.
  **End-to-end engine validation (2026-07-22):** the Python replicas validate the
  *rule*, but `diagnose_engine` drives manual mode, so the *shipped auto path* was
  separately checked by `examples/validate_engine_lock.rs` — it drives the real
  `Engine::process` in auto mode (`target_note = None`) over the captures with the
  live gatekeeper flags and records the first `identified_key` latch. Result:
  **81/87, failures 000/003/005/010/012/086, b21/m33/t27 — identical to the replica
  and this ADR.** The integration is measured, not just argued.

### Provisional conclusion: structural bolt-ons failed, but were NOT co-tuned

All three structural enhancements, **tested as bolt-ons with frozen base constants**,
fail on the **same wall** — the bass register's intrinsic irregularity on **both
axes**: sparse/gappy *in frequency* and jagged *in amplitude* (beating, soundboard,
missing fundamentals, single-frame noise).

| enhancement | charges/forgives | bass failure (frozen-constant test) |
| --- | --- | --- |
| deadzone | forgives frequency distance | dense impostors win |
| non-peak count | charges absent partials (freq gaps) | true bass crushed |
| Emiya smoothness | charges amplitude incoherence | true bass crushed |

The directional pattern is suggestive: any term *assuming regularity* penalizes the
true bass note; any term *adding tolerance* rewards the dense impostor.

⚠️ **But this is NOT yet a "fundamental limit," and an earlier revision overclaimed
it as one.** Two gaps, flagged by review, block that conclusion:

1. **None of the three was co-tuned.** Each was a structural coefficient bolted onto
   the *frozen* conservative q/r/ρ — exactly the test Finding #3 declares invalid for
   error-landscape changes. The non-peak term is deliberately un-normalized, so its
   coefficient interacts directly with the base error scale set by q/r/ρ; freezing
   those confounds it. A MOBO arm with the structural coefficient as a **free**
   parameter (and q/r/ρ re-optimized around it) was never run. `optimize_twm.py`
   only ever suggested `{p,q,r,ρ,λ}` — no structural params.
2. **The B-mismatch diagnostic is unmeasured.** If bass false-locks are partly
   *templates at the wrong inharmonicity* (the fixed Rigaud prior, never per-note
   estimated — see `mobo-methodology.md` §8.2), the limiting factor is the B prior,
   addressable by joint (f₀,B) estimation (0005 revisit #3), **not** a peak-domain
   dead end. "Exhausted" cannot be claimed while this is untested.

**Honest status:** the bolt-on rejections stand *as bolt-ons*. The stronger claim —
"fundamental limit of peak-domain scoring; 74/87 is the ceiling" — is **withdrawn**
pending (a) a co-tuned structural MOBO arm and (b) the B-residual diagnostic. Until
then, 74/87 is the best *measured* result, not a proven ceiling.

## Consequences / scope notes

- Validation is on **one** out-of-tune instrument, and selection (best-of-N Pareto
  by that piano's pass-count) is itself a fit to it. The conservative config is a
  net +3 on it, but the constants' margin is concentrated in the manual-mode extreme
  treble (see revised Decision); a second instrument and the in-tune regime remain
  required confirmation, not optional.
- Extreme treble (A7–C8, ~2–5 partials) is near information-limited; manual mode is
  the accepted fallback there.
- Pitch-raise is in V1 scope (this is a tuner, not a detector); the Stage-A recall
  item above is required for it, not optional.
- **Quantified pitch-raise-reach cost of the conservative config:** measured on the
  full `discover()` pipeline (Stage A K=3 → Stage B), 1¢-resolution key-40 sweep:
  canonical M&B holds the true key to **78¢** of pitch-raise; the conservative
  default holds to **69¢** — a **~9¢** loss at fixed K=3. (An earlier revision wrote
  65/75¢ — the ~9¢ delta was right but the absolutes were ~3¢ pessimistic; corrected
  here.) Two clarifications review surfaced: (a) the failure mode is the **Stage-A
  top-3 gate** (the true key drops to rank ≥3), *not* Stage B — refine-alone on the
  true key recovers past the ceiling; under the old TOP_K=88 the +70¢ case passed for
  *any* config, so this cost is **co-caused by the K=88→3 switch**, not the constants
  alone. (b) Pitch-raise is **V1 scope** (Finding #4), so adopting constants that
  reduce pitch-raise reach for treble gains on an instrument that does not exercise
  pitch-raise is a real tension, not a free win. Encoded in
  `refined_recovers_detuned_notes` (the +60¢ case, well-margined under 69¢).
