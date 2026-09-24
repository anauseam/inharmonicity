# report 0006 — TWM calibration and validation

The longest record in the project. The shipped configuration is the guard at
`TwmConfig::default`; everything below is how it was arrived at,
including two rounds of adversarial review that withdrew five load-bearing claims
and a peak-estimator bug fix that invalidated a candidate the search had chosen.

**Read the 2026-07-05 re-derivation (2026-07-05) before quoting any figure from
the earlier sections.** [Audit 03](audits/faithfulness-audit-03-jacobsen.md)
found a real bug in the Jacobsen interpolator — a bespoke $(-1)^m$ term and a
missing Candan $c_N$ correction, biasing every Discovery peak by −2.5 δ bins —
and every lock-accuracy number measured before it is on biased peaks. A′ re-ran
the affected work mechanically. Two conclusions changed sign.

## What is provisional here

The conservative tuned constants ship as `TwmConfig::default()` **provisionally**.
Two rounds of adversarial review in 2026-06 withdrew five load-bearing claims,
and they are corrected throughout this report. They are listed because each one
has been quoted since:

1. **"No register regressed" was false.** Against the discrete baseline (71/87)
   the shipped config is bass +1, mid **−1**, treble +3. The mid regression is a
   refinement cost and constants-neutral, but the headline had mixed the discrete
   and refined baselines.
2. **"Options exhausted / fundamental limit" was premature** at the time. The
   structural-term rejections were frozen-constant tests. *Since closed:* the
   co-tuned arm ran and rejected both terms on real, and the B-residual
   diagnostic was measured — both below.
3. **NSGA-II did not select the constants.** Its own optimum is degenerate
   (`floor_frac = 0.22` enters the error collapse the ordinal objective was meant
   to retire) and seed-fragile. The shipped constants came from the **real-data
   plateau**, not the synthetic argmin: q = 3.88 sits below every seed's
   synthetic optimum of 5.7–7.6.
4. **"+3/+4 beats M&B" is not significant.** McNemar on the six discordant keys
   gives p ≈ 0.22, and four of five gains are manual-mode extreme treble.
5. **The multi-objective framing is thin.** The second objective varies by under
   0.001 across the whole front, so the search was effectively single-objective
   on a flawed first objective.

**One gate remains: a second real instrument with in-tune captures.** n = 1
cannot carry the selection. The methodology was rebuilt before the re-sweep —
production-K bass and treble false-lock objectives, a K-robustness diagnostic
showing the constants are K-robust rather than K = 3-overfit, three-seed pooling,
and a pinned dataset fingerprint — and that rebuild is recorded in
[the NSGA-II methodology](nsga2/README.md) §§4, 5, 8.

**The bass bottleneck is a wrong-B template, not a scoring limit.** The synthetic
oracle-B ablation collapses bass separability false-lock from 27 % to 1.5 % when
the true key is given its true inharmonicity. That is why the deadzone, Duan and
Emiya terms all failed on bass: they tweak *scoring* while the problem is the
template's *B shape*. Caveats in `nsga2/README.md` §8.2 — idealised oracle,
separability rather than production, and the synthetic understates the real
one-directional gap.

## Context

report 0001 specified NSGA-II tuning of the TWM constants; report 0005 specified split
discovery (Stage A discrete scan → Stage B basin-clamped scale refinement). This
report records what the empirical program actually found when those were built and
validated — including several course corrections, because the journey is the
evidence. The operational *how* (harness, synthetic signal model, objectives,
search config, selection protocol, and the threats-to-validity audit) lives in
[the NSGA-II methodology](nsga2/README.md).

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
   per-frame median toward the normalizer floor, inflating objB. NSGA-II exploited
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

6. **The NSGA-II's synthetic optimum is a synthetic overfit; the robust Pareto point
   wins on real.** With p and λ free, NSGA-II (seed 42) converged on p≈0.8, λ≈1.5 (best
   *synthetic* separability, objA 0.205). On the real captures that regime only
   matches the conservative config by **trading bass for treble** (bass 15–16 vs 20)
   and is mode-fragile. The **conservative Pareto point** (p=0.5, λ=18, tuned
   q/r/ρ) — which the synthetic ranked *lower* — wins cleanly on real.

   ⚠️ **Corrected by the 2026-06-20 methodology review** (`nsga2/README.md`
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

## What the decision was measured against

Decided in report 0006.

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
register this very report calls "near information-limited … manual mode is the accepted
fallback" (~2–5 partials, most exposed to the dense-treble attack archetype). The
**bass — the register the entire dense-attractor narrative is about — moved only +1.**
Selecting a config by one-piano pass-count where the margin lives in the flakiest
register is a real overfit risk: the deltas are deterministic (not run-noise), but
**one instrument cannot bound instrument-noise**, and best-of-N Pareto selection on a
single piano is itself a fitting procedure (see Threats in `nsga2/README.md`).
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
(small Bm²). The deadzone as **implemented** ([twm.rs](../tuner-core/src/algorithms/twm.rs))
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

## Open items, and what has been closed

### Still open

- **Stage-A recall under pitch-raise** (deprioritized). Widening K to keep a
  detuned true key re-admits the dense-bass attractors the small-K filter exists
  to exclude, so the headroom is the pitch-raise subset only. The old "revisit
  after Duan" condition is void — the Duan-count term was co-tuned and rejected
  (below). A Stage-A rank diagnostic would only be informative on detuned or
  second-instrument captures: the present set sits below the ~35 ¢ crossover, so
  the true key is nearly always in the top K and the gate binds only past 55 ¢.
  For v1 manual mode covers pitch-raise, since the operator names the key and
  Stage-A recall is not involved; auto-mode pitch-raise stays gated.
- **Window-width sweep** — the adjacent-theft against pitch-raise-reach tradeoff.
  The ±80 ¢ search window is a compile-time constant.
- **Robustness-aware selection** — regularising toward analytic priors, against
  the present "pick the robust Pareto point by real validation".

### Closed: four structural scoring terms, every one rejected on real captures

Each was implemented, measured against the 74/87 baseline on the 87-capture set,
and left in `TwmConfig` as a default-off field. Arms 6–7 then re-ran the last two
with q/r/ρ **co-tuned** rather than frozen, so the rejections are not an artefact
of testing a structural coefficient against a fixed error scale.

| term | what it changes | measured on real | field left behind |
| --- | --- | --- | --- |
| Wider q/r bounds (q→[0,8], r→[0,3]) | re-tunes the two exponents | 74/87, identical per-register; q ∈ [3.9, 7.7] all give 74 | — (bounds not widened) |
| B-deadzone (n-kernel) | forgives forward distance within ∂f_n/∂B | monotone harm: 66 / 64 / 62 at c = 0.14 / 0.3 / 0.5 | `b_deadzone` |
| Duan non-peak, count form | charges each predicted partial with no peak | bass collapses: 10 / 5 / 0 at c = 0.05 / 0.2 / 1.0 | `nonpeak_penalty` |
| Emiya amplitude smoothness | charges amplitude incoherence of matched partials | bass 66 / 54 / 35 at s = 0.3 / 1.0 / 3.0, λ-stable | `smoothness_penalty` |

Co-tuned, `nonpeak` dominated on synthetic but still hurt real bass (77/87 bass
21 → 68/87 bass 14), and `smoothness` failed to help even on synthetic.

**The meta-finding, which is what survives all four.** The
"predicted-but-absent partial" signal is fundamentally ambiguous in the bass.
Forgiving it lets dense sub-harmonic impostors win, because forgiveness is
symmetric and the impostor has more partials to be forgiven on; charging it
crushes true bass notes, because real bass spectra are gappy throughout rather
than only below the fundamental. A count cannot separate a true bass note's
legitimate gaps from an impostor's hallucinations, and the amplitude route
(Emiya) fails for the matching reason — beating, soundboard dips and missing
partials make a true bass note's envelope jagged, so a smoothness prior charges
the true note. Both axes of the bass register's irregularity, frequency and
amplitude, defeat any term that assumes regularity or adds tolerance.

The four are **proxies, not faithful ports** — the non-peak term is an
unnormalised hard-tolerance count rather than Duan Eq. 7's likelihood, and the
smoothness term is a log-amplitude second difference rather than Emiya's model.
A faithful-likelihood objective is therefore untested, deferred on low expected
upside and gated with everything else on the second instrument.

### Closed: measured B fed back into discovery (built, shipped gated off)

The pathway is implemented and gated off behind
`APPLY_MEASURED_B_TO_DISCOVERY`. Sharpening a template with the key's own
measured B also sharpens the **sub-harmonic impostor** templates, because the
octave-below key's even partials align with the struck note's.

Measured with a per-candidate joint (f₀, B) refinement, the bind is quantitative
and structural:

| setting | real | bass octave lock-frames | net |
| --- | --- | --- | --- |
| tight bound + regulariser (the shippable one) | 74/87 | 129 | byte-identical to fixed-β baseline |
| wide bound, unregularised (β reaches real bass B) | 72/87 | 102 (−21 %) | fixes A#0, breaks 5 |
| wide bound + regulariser | 74/87 | 129 | γ pins everything back to prior |

At safe settings the lever is **inert**: the ±2σ bound cannot reach the deep-bass
7–25× gap on real, and on synthetic the true B sits at 1.06× prior so there is
nothing to correct. Only the wide bound engages it, and it nets −2.

**A regulariser cannot separate the two cases, and this is the durable result.**
The separation criterion, distance from the prior, is backwards in the deep bass:
the true bass key is the *largest* prior-deviator, because the prior is known
wrong there, so any γ strong enough to penalise an impostor penalises the true key
first. At scoring time, "let the true bass key reach its real B" and "let an
impostor reach a flattering B" are the same knob. The same objection defeats
confidence-gating, whose input is pairwise coherence — self-consistency, not
accuracy — and this project has already driven A#0 to 279× prior on a
self-consistent but wrong partial series.

**Decision: do not port to the hot path.** A real bass-B fix needs an application
path whose asymmetry is not distance-from-prior — applying a *trusted, per-key*
measured B to that key's own template only — and that still rests on the standing
trusted-B-reference linchpin. Estimator repair is not needed: synthetic-truth
recovery shows MAT tracks a known B to under 1 % across the doubted regime when
the f₀ seed is within ±10 %, so the deep-bass 7–25× reading is a real measurement
rather than a mis-association artefact.

### Corrections from the 2026-07 review pass

1. **The structural-term tests used proxies, not faithful ports** — stated in
   full above, with the four terms.
2. Superseded by the 2026-07-05 re-derivation below: the 77/87 candidate was
   pinned to seed-7 trial 1898 (q ≈ 7.681, r ≈ 2.908, ρ ≈ 0.487), but that was
   measured on biased pre-jacobsen-fix peaks and no longer beats the default.
3. **Auto-capture provenance is a data-integrity risk for the tuning curve.** In
   auto mode the Worker's f₀ seed is the discovery lock, so an octave false-lock
   makes MAT confidently measure the wrong key's series — the (2f₀, 4B) identity
   with a *low* self-residual, which confidence cannot catch — and persist it
   under the wrong key. Discovery is insulated because the gate is off; the
   persisted profile is not. **The tuning curve consumes manual-mode captures
   only.** Since 2026-08-13 the rule has a second axis on the same footing: a
   capture whose declared strings are not the note's full unison measured one
   string, not the note. Both live in `KeyMeasurement::is_trusted`; see
   [`capture-sets.md`](../docs/internals/capture-sets.md).
4. **MAT's `confidence` is not part of DAFx-09.** The paper outputs only (f₀, B)
   and the median is its robustness mechanism; our coherence × evidence scalar is
   a bespoke addition. It stays a runtime diagnostic, is not persisted into
   `KeyMeasurement`, and no tuning-curve weighting depends on it.
5. Superseded by the 2026-07-05 re-derivation below: the flicker-versus-stable
   failure split was measured on biased peaks and the failure sets changed.
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
