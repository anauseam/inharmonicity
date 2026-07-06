# MOBO Methodology & Reproducibility

**Purpose.** This document records *how* the TWM constant-tuning runs were actually
performed — the harness, the synthetic signal model, the objectives, the search
configuration, the selection protocol, and (most importantly) the **threats to
validity**. ADR 0001 gives the conceptual rationale; ADR 0006 records the findings.
This file is the operational layer between them: enough to re-run the sweep, audit
the numbers, or distrust them on specific, named grounds.

It is written to be *scrutinised*. The "Threats to validity" section at the end is
not boilerplate — it is the list of ways these results could be wrong, and should be
read before any conclusion drawn from a MOBO run is treated as load-bearing.

---

## 1. Artifacts

| Artifact | Role | Tracked? |
| --- | --- | --- |
| `tuner-core/examples/mobo_evaluator.rs` | Synthetic dataset + objective evaluator (`--serve` stdin/stdout protocol) | modified, **uncommitted** |
| `scripts/optimize_twm.py` | Optuna NSGA-II orchestrator (5 arms) | **untracked** |
| `scripts/validate_config.py` | Real-capture validator (the decision gate) | untracked |
| `twm_mobo.db` | Optuna SQLite study storage (resumable) | untracked |
| `twm_pareto_arm{1..5}.json` | Per-arm Pareto fronts (params + objA/objB + diagnostics) | untracked |
| `diagnostics/key_*/` | Real captures (one out-of-tune upright) — **validation only** | — |

> **Reproducibility risk (current):** the harness is uncommitted. Until it is
> committed alongside this doc, a lost working tree = an unreproducible run. Commit
> the four scripts + the evaluator together with this file.

## 2. How to run

```bash
# 1. Build the evaluator (release; the sweep is hours even parallelised).
cargo build --release --example mobo_evaluator

# 2. Run the 5-arm sweep. Refuses to clobber an existing db (see resume guard).
python3 scripts/optimize_twm.py            # fresh run (errors if twm_mobo.db exists)
python3 scripts/optimize_twm.py --resume   # continue existing studies

# 3. Validate a candidate config on the REAL captures (the decision gate).
python3 scripts/validate_config.py --refine --config "0.5 3.88 1.426 0.298 18"
```

The evaluator can also be driven directly (one trial per stdin line):

```bash
./target/release/examples/mobo_evaluator --serve
# then write:  "<mode> <p> <q> <r> <rho> <lambda>\n"   mode ∈ {refine, discrete}
# reads back:  {"objA":..., "objB":..., "fl_bass":..., ...}
```

## 3. The synthetic dataset

**Why synthetic (ADR 0001).** No labelled in-scope acoustic corpus exists with
sub-cent (f₀, B) ground truth, and hand-annotating ~10⁴ frames is infeasible and
error-prone. The synthetic gives perfect labels by construction; the real captures
are reserved as a worst-case *validation* set, never a tuning target (we will
overfit a single instrument otherwise — see ADR 0006).

### 3.1 Determinism

- Hand-rolled **SplitMix64** RNG (`mobo_evaluator.rs`), chosen over `rand::StdRng`
  because StdRng's stream is not stable across crate versions.
- `FIXED_SEED = 0x1AB4_2026_0612_5EED`; the dataset is regenerated identically every
  run.
- **Dataset fingerprint** (FNV-fold over every frame's key, d_cents, and all peak
  freq/mag bits): expected `e11fea90889dee30`. A changed fingerprint means the
  dataset drifted — any cross-run comparison is then invalid. Printed by the no-arg
  diagnostic mode: `./target/release/examples/mobo_evaluator` (look for the
  `fingerprint …` line); note `--serve` mode does **not** print it.

### 3.2 Sampling structure

- `BASE_FRAMES_PER_KEY = 100` over all 88 keys, plus `HARD_FRAMES_PER_KEY = 20` of
  targeted confusable-pair oversampling on 82 keys (the historically hard pairs).
- **Detuning strata** (the per-frame tuning state, `gen_frame`): 30% freshly tuned
  (0–5¢), 50% typical service drift (5–25¢), 20% neglected / pitch-raise (25–70¢),
  layered *on top of* the Railsback stretch. `hard` frames bias toward the harder
  band. `AMBIGUOUS_CENTS = 78` flags frames past the refinement window's reach.

### 3.3 Signal (physics) model

Per frame, `gen_frame` builds a peak list intended to match the engine's
`extract_peaks` → `mask_peaks` contract (top-64 by magnitude, masked, ascending):

- **Inharmonic partials** `f_n = n·f₀·√(1+B·n²)`, n up to 64 / 9 kHz.
- **B (inharmonicity):** Rigaud two-bridge curve, with the **piano-dependent bass
  bridge** varied per instrument (slope ±10%, intercept spread) and **per-note
  Gaussian scatter** σ=0.157 (bass) / 0.116 (treble) (our calibration; was
  mis-cited to "Rigaud Fig. 3" pre-audit — see faithfulness-audit-06), plus
  **B↔f₀ coupling** ΔB/B = −2·Δf₀/f₀ under detuning. **Critically, the scorer's
  template B is the smooth prior `get_expected_beta` — so every frame is scored
  under a realistic prior-vs-truth B mismatch** (see Threats §8).
- **Stretch / tuning:** the synthetic rolls its *own* randomized Rigaud
  ρ-type-octave model (per-instrument m0/α/K) + the per-frame detune. **Note:** the
  *engine* does **not** centre templates on a stretch curve — it scores at raw ET
  (`build_profiles` uses `NOTES[i].frequency`) and absorbs all stretch in Stage-B
  scale refinement (models.rs:234, "Discovery currently scores templates at raw
  ET"). `railsback_stretch_curve()` exists in `models.rs` but is **defined-but-unused
  future work**, called by neither the engine nor the synthetic. (An earlier revision
  of this doc wrongly stated the engine centres on it.)
- **Unison strings:** 1 (low bass) / 2 (upper bass) / 3 (tenor+), with a 0–15¢
  low-weighted unison spread → beating clusters.
- **Spectral envelope:** aₙ = n^(−α) with lognormal per-partial jitter; flatter α in
  the bass (soundboard kills the lows, not the highs).
- **Missing fundamentals:** keys <15 drop partials 1–3 with high probability
  (soundboard impedance) — the bass missing-fundamental regime.
- **Sympathetic resonance:** octave-below / fifth-above ring-through at −20…−35 dB
  (the energy that makes dense sub-harmonic impostors cheap) — 45% of frames (80% on
  hard frames).
- **Noise:** pink-tilted broadband peaks; plus a treble "attack archetype" (30–70
  dense low/mid peaks) on 35% of keys ≥55.

## 4. Objectives

Computed per frame in `process_frame`, accumulated across the dataset.

All false-locks below are the **production** `discover()` outcome at the shipped
`TOP_K=3` (Stage A top-K → Stage B refine → argmin) — what actually ships.

- **Objective A (minimise) — BASS false-lock** (production K=3, keys 0–26).
- **Objective B (minimise) — TREBLE false-lock** (production K=3, keys 60–87).

**Both objectives redesigned 2026-06-20** (reviews §8.3, §8.13). Two changes:

1. *Production, not separability.* The optimized metric was the all-88 *separability*
   argmin, which in refine mode is exactly the **K=88** regime ADR 0006 Finding #1
   measured as the *worst* real setting (degenerate, error-collapse optimum). It is now
   the production K=3 outcome; separability is demoted to the `sep_fl` diagnostic.
2. *A real second objective.* The old objB (ordinal confidence) was near-degenerate
   (<0.001 spread across the front → "multi-objective" in name only, §8.13). Replaced
   with the **bass-vs-treble** tradeoff: two *disjoint* registers that respond to
   opposite parameter regimes, so the Pareto front is genuinely 2-D and surfaces the
   exact register tension that drives config selection (ADR 0006: the conservative
   config gained treble while bass stayed flat). Plain `overall-vs-bass` was rejected
   as partly collinear (bass dominates overall) — it would re-create the degeneracy.

A **floor gate** in `optimize_twm.py` rejects any trial with `floor_frac > 0.05`
(worst-corner penalty), so NSGA-II cannot exploit the error-collapse regime.
The M&B-default sanity values: overall prod_fl ≈ **0.308**, fl_bass ≈ 0.269,
fl_treble ≈ 0.347.

- **K-robustness diagnostic (not optimised) — `prod_fl_k{2,3,4,5}`.** Production
  false-lock at K ∈ {2,3,4,5}, computed in the evaluator by reusing the already-
  refined per-key errors + the unrefined Stage-A ranking (so `prod_fl_k3` == objA
  bit-for-bit). **Purpose: K=3 is empirical (best of measured {3,88}, n=1), not
  proven optimal, and K interacts with the constants — so we *measure* whether a
  chosen config is K-robust instead of assuming it.** First reading is reassuring:
  M&B `{0.310, 0.308, 0.308, 0.307}` and the conservative config
  `{0.266, 0.263, 0.262, 0.262}` are nearly flat across K (slightly *better* at K4/5)
  — the constants are not K=3-overfit. A proper K-sweep (or making K a search
  dimension) remains future work.

**Other diagnostics (not optimised):** `sep_fl` (K=88 separability — the old objA),
detuning strata (<15 / 15–35 / 35–55 / 55–78¢), register strata
(bass/mid/treble/hard), scale-fidelity |ŝ−d_cents|, and degeneracy probes
(median-floor hits, near-tie hits).

## 5. The search

- **Optimiser:** Optuna `NSGAIISampler(population_size=128, seed∈{42,1,7})`, directions
  `[minimize, minimize]` (bass FL, treble FL), SQLite-backed, `load_if_exists=True`.
- **Multi-start seeding (2026-06-20, review §8.11):** each arm is run under **3 seeds
  (42/1/7)** and the Pareto candidates are **pooled** (union, dedup by rounded
  params) into `twm_pareto_arm{n}.json`. This is multi-start optimization *for
  coverage* — NOT ensemble averaging (evaluation is deterministic; nothing to
  average). Population was enlarged **50→128** because seed-fragility at pop=50 in a
  5-D space is premature convergence; a larger population is the more effective fix and
  3 seeds then verify residual stability.
- **Arms** (fixed/free split isolates what each tests):

  | Arm | Mode | Free | Fixed | Tests |
  | --- | --- | --- | --- | --- |
  | 1 | refine | q, r, ρ | p=0.5, λ=∞ | amplitude terms, no ceiling |
  | 2 | refine | q, r, ρ | p=0.5, λ=18 | amplitude terms, with ceiling |
  | 3 | refine | q, r, ρ, λ | p=0.5 | + free ceiling |
  | 4 | refine | p, q, r, ρ, λ | — | full freedom |
  | 5 | discrete | p, q, r, ρ, λ | — | discrete vs refined (ADR 0005 ablation) |
  | 6 | refine | q, r, ρ, **nonpeak** | p=0.5, λ=18 | **co-tuned Duan non-peak** (§8.7) |
  | 7 | refine | q, r, ρ, **smoothness** | p=0.5, λ=18 | **co-tuned Emiya smoothness** (§8.7) |

  Arms 6–7 are the co-tuned structural test ADR 0006 Finding #3 requires: the
  structural coeff is a *free* parameter with q/r/ρ re-optimized around it, not the
  invalid frozen-constant bolt-on. They mirror arm 2 + one structural term.
- **Bounds:** p∈[0,1], q∈[0,8] (widened from a 4.0 ceiling pin), r∈[0,3] (widened
  from 1.5), ρ∈[0,1.5], λ∈[1,50], nonpeak∈[0,1], smoothness∈[0,3] (each can hit 0 ⇒
  the optimizer may turn the structural term off if unhelpful).
- **Budget / stopping:** `n_trials=2000` per (arm, seed) with a `PlateauStopper`
  (2-D hypervolume vs ref (1,1), stagnant for 300 trials ⇒ stop). 7 arms × 3 seeds =
  21 studies — expect a multi-hour sweep.
- **Sanity gate (fail-loud):** before the sweep, the M&B default must reproduce
  overall prod_fl ≈ 0.308 (±0.02) **and** the dataset fingerprint must equal
  `e11fea90889dee30`. A mismatch ⇒ flipped objective / stale binary / drifted dataset —
  abort before burning hours.
- **NaN-safety:** all score comparisons use `f32::total_cmp` (no `partial_cmp().unwrap()`
  panic path), so a degenerate config can't kill a multi-hour sweep (§8.12).
- **Resume guard:** refuses to run if `twm_mobo.db` exists without `--resume`.

## 6. Selection protocol

This is the load-bearing discipline (ADR 0006): **tune on synthetic, decide on
real.**

1. Run the sweep → per-arm **seed-pooled** bass-vs-treble Pareto fronts
   (`twm_pareto_arm*.json`, union over seeds 42/1/7).
2. Do **not** trust the synthetic optimum — it is degenerate and seed-fragile (§8.3,
   §8.11). The fronts are only a *candidate menu*, deliberately spanning the
   bass↔treble register tradeoff.
3. Take the pooled candidates to `validate_config.py` on the real captures and pick by
   **overall real pass-count** with **no register regressed**. (This is what surfaced
   the prior conservative config `0.5 3.88 1.426 0.298 18` at 74/87 — but that win is
   not statistically significant on n=1, §8.6, so the *real* decision waits for the
   second instrument.)

## 7. Reproduction checklist

- [ ] Evaluator rebuilt (`--example mobo_evaluator`, release).
- [ ] Dataset fingerprint == `e11fea90889dee30` (run `./target/release/examples/mobo_evaluator` no-arg).
- [ ] Sanity: M&B default → overall prod_fl ≈ 0.308 (asserted by the orchestrator).
- [ ] `twm_mobo.db` absent (fresh) or `--resume` intended.
- [ ] Python deps: `pip install -r scripts/requirements.txt` (Optuna pinned) in the .venv.
- [ ] Final config decided on **real** captures (overall pass-count, no register
      regressed), not synthetic hypervolume.

## 8. Threats to validity (read this before trusting a result)

These are the named ways the methodology could mislead. Items are tagged with
severity and provenance after the **two-agent adversarial review of 2026-06-20**
(Review 1 = dataset realism; Review 2 = optimization/inference). `[CONFIRMED]`,
`[UPGRADED]`, `[NEW]`, `[SHARPENED]`, `[CLEARED]` mark what that review changed.

### Independent review verdict (2026-06-20): trust-with-caveats

Both reviews land in the same place: the **synthetic machinery carries near-zero
trustworthy signal about the *final constants*** — its own optimum is degenerate
(§8.3) and seed-fragile (§8.11) — so **~100% of the protective value rests on a
single out-of-tune upright**, and on that one piano the headline win is **not
statistically significant** (McNemar p≈0.22, §8.6). The shipped constants remain a
*defensible provisional default* (conservative; p/λ pinned canonical; on a broad
real plateau; no register regressed by the constants), but three stronger claims do
**not** survive and are withdrawn in ADR 0006: "MOBO *selected* good constants"
(real-plateau validation did, not the search); "+3/+4 beats M&B" (sub-significance);
"structural terms exhausted" (untestable in current code).

**The two reviews converge on one highest-value action: a second real instrument
(plus an in-tune capture set).** Review 1 needs it because the bass B gap cannot be
quantified from the one piano (the only "real B" available, `calculated_b`, is the
engine's own clamped, prior-seeded estimate and fails internal consistency). Review 2
needs it because n=1 gives the selection no statistical footing. Until it exists, the
mid-register constant-rankings are trustworthy; bass and the headline margin are not.

**Experiments required before any conclusion is load-bearing** (status as of
2026-06-20):

1. **Second real instrument + in-tune-regime captures** — ⏳ pending (user can source a
   second, slightly-less-out-of-tune upright later). Non-negotiable; the only thing
   that can carry the selection. Highest value.
2. **Co-tuned structural MOBO arm** — ✅ **done 2026-06-23/24** (arms 6–7). Verdict:
   both structural terms **rejected on real** by the valid co-tuned test. `nonpeak`
   *looked* like a win on synthetic (dominated arm 2) but **hurts bass on real even
   co-tuned** (77/87→68/87, bass 21→14); `smoothness` didn't help even on synthetic.
   Coherent with the oracle-B / B-limited-bass finding. See §8.7.
3. **B-residual diagnostic** (0005 revisit #3) — ✅ **synthetic oracle-B done
   2026-06-21** (`b_oracle_diagnostic`): bass separability FL **27%→1.5%** with true-B
   templates ⇒ bass is **B-limited, not peak-domain-limited** → joint (f₀,B) is the
   high-value bass lever (§8.2). ⏳ Real-data confirmation still pending (the robust
   real-capture B-fit is hard — Review 1 — so it waits for the second instrument with
   a trustworthy B reference).
4. **objA/objB redesign + degeneracy gate** — ✅ **done 2026-06-20.** objA = production
   K=3 **bass** FL, objB = production K=3 **treble** FL (orthogonal, non-degenerate
   front); separability demoted to `sep_fl`; `floor_frac > 0.05` trials rejected;
   `prod_fl_k{2,3,4,5}` K-robustness diagnostic added (§4, §8.3, §8.13). **Consequence:
   the existing db/fronts are stale (old objA/objB) and must be regenerated by the
   re-sweep.**
5. **Process fixes** — ✅ Optuna pinned (`scripts/requirements.txt`); ✅ fingerprint
   asserted in the orchestrator; ✅ multi-seed (42/1/7) pooling + population 128 (§8.11);
   ✅ NaN-safe comparisons (§8.12). ⏳ Remaining: commit harness + db + fronts + this doc
   together (deferred until the MOBO work is complete).

### Threats

1. **[CONFIRMED · model-fidelity] The synthetic prices the synthetic.** Every objA/objB
   "improvement" is against our generative model. Review 1: the model is *not* a
   strawman — the peak-domain contract is faithful (§8.8) and the mid-register
   (~keys 40–60) is genuinely well-modeled — but the sub-harmonic / sympathetic
   attractor field is parametric, not mechanistic, and its prevalence/amplitude can't
   be validated against the captures (the real bass churn, e.g. A0→F#1, may be
   low-SNR garbage as much as a modeled attractor). Synthetic *rankings* can still
   mis-order the candidates we validate.

2. **[SHARPENED · HIGH-for-bass] B is never estimated — and the mismatch model has the
   wrong *shape* in the bass.** The scorer always uses the Rigaud prior
   `get_expected_beta`; B is never estimated per note (joint (f₀,B), the
   ADR-0005-gated step). The synthetic injects per-instrument + per-note B mismatch
   (§3.3) — but as **zero-mean scatter centered on the prior**. Review 1 finds the
   real bass error is **one-directional**: across all 25 captured bass keys, every
   robust strong-partial B refit sits *above* the prior (~2.6–22×, median ~7×; the
   prior's deep-bass B ~1e-4 is low for this short-string upright vs literature
   ~3–10e-4). So the generator models a symmetric error where the real sign is
   consistent → bass templates track synthetic truth more closely than real ones do →
   bass separability (~25% loss) is **plausibly optimistic**. ⚠️ **Magnitude is
   unquantifiable and must NOT be "fixed" by recalibrating the generator to
   `calculated_b`** — that estimate is clamped (−0.001, 0.01), prior-seeded, and
   demonstrably operates on mis-numbered partials (yields B<0 on bass keys 0/11/13/22).
   The right resolution is the B-residual diagnostic + a second instrument with a
   *measured* B reference, not a generator tweak.

   > **[Footnote · 2026-06-25 · Worker-MAT rewrite]** The specific characterisation of
   > `calculated_b` above (clamped/prior-seeded, mis-numbered partials, B<0 on keys
   > 0/11/13/22) describes the **old** Worker estimator and is now **superseded**: the
   > Worker's MAT was rewritten as a faithful Median-Adjustive-Trajectories joint (f₀,B)
   > estimator (serial growth + CSPE sub-bin refinement; `tuner-core/src/algorithms/mat.rs`).
   > `calculated_b` is now a genuine measurement — no prior seeding, no negative/impossible
   > bass B (all 39 captured bass keys measure positive, ~6–10× the prior, with an honest
   > confidence), and `None` when unmeasurable rather than a laundered prior. The deeper
   > point of this threat still **stands**: this is one out-of-tune upright with no *trusted*
   > B reference, so the new `calculated_b` must STILL NOT be used to recalibrate the
   > generator — the second instrument with a measured B remains the gate. This footnote
   > only narrows *which* failure modes are real; the rest of §8.2 awaits a separate review.

   **🔑 Oracle-B diagnostic result (2026-06-21, `b_oracle_diagnostic` in the no-arg
   evaluator).** Giving the *true* key its *true* generated B (impostors stay at the
   prior) and measuring separability (all-88 argmin) false-lock:

   | register | sep-FL prior-B | sep-FL oracle-B | Δ | mean B_true/B_prior |
   | --- | --- | --- | --- | --- |
   | bass | 0.269 | **0.015** | **−0.253** | 1.06× |
   | mid | 0.274 | 0.138 | −0.135 | 1.00× |
   | treble | 0.230 | 0.114 | −0.117 | 0.99× |

   **Bass separability false-lock collapses 27% → 1.5% from a mere ~6%-mean B
   correction** — so the bass error is *overwhelmingly a wrong-B-template problem, not
   a peak-domain limit.* This answers the 0005 revisit-#3 gate: **joint (f₀,B)
   estimation is warranted and is likely the single biggest bass lever** — bigger than
   any TWM-constant tuning, which cannot fix a template of the wrong *shape*. It also
   reframes ADR 0006's "structural fixes fail on bass": they fail because they tweak
   *scoring* while the bass bottleneck is the *template's B*. **Caveats (it is an
   upper bound, not the achievable gain):** (a) the oracle is *asymmetric* — only the
   true key gets perfect B; real joint estimation must estimate B for all candidates
   from the same noisy peaks and would also help impostors; (b) it is *separability*,
   not production (which adds K-recall); (c) real bass B is one-directionally ~7× the
   prior (Review 1) so the *synthetic* understates the gap — the real lever is at least
   this large. Net: a strong, decision-relevant hypothesis to confirm on a real
   instrument, not a delivered gain. **Open (0005 revisit #3 now triggered).**

   > **[Footnote · 2026-06-27 · measured-B→discovery, real-data validation]** The
   > confirmation step ran: the measured per-key B (rewritten MAT) was compiled to
   > discovery templates and replayed over the 87 real captures
   > (`test_engine_all.py --refine --profile tuning_profile.json`). Result: **net
   > regression, not the predicted bass collapse** — 74→73/87 overall; the
   > highest-ratio bass keys (3/16/17 at **18–25×** the prior) *broke*, while the
   > only clean gains were treble keys where the prior *over*-estimates B. This does
   > not contradict the synthetic ablation above; it sharpens it: the oracle used true
   > B only ~1.06× prior (a 6% nudge) and gave it to the **true key alone**, whereas
   > the real MAT B is 7–25× prior (untrusted, plausibly over-estimated on this one
   > out-of-tune upright) and production seeds **every** key (boosting impostors too —
   > caveat (a) above, realized). The pathway is shipped but **gated off by default**
   > (`pipeline::APPLY_MEASURED_B_TO_DISCOVERY`), pending the **second instrument with
   > a trusted B reference** — the same gate this section already names. Full
   > write-up + cross-tab in ADR 0006 (B-residual item). The real bass-B refit itself
   > remains unmeasured against a trusted reference.

3. **[UPGRADED → HIGH · objective design — ✅ FIXED 2026-06-20] Objective A optimised the
   architecture the project rejected, and its optimum was degenerate.**
   **Resolution:** objA was retargeted to the production K=3 `prod_false_lock` (§4),
   separability demoted to the `sep_fl` diagnostic, and `optimize_twm.py` now rejects
   trials with `floor_frac > 0.05` (the collapse gate). A K-robustness diagnostic
   (`prod_fl_k{2,3,4,5}`) was added so K=3 is measured, not assumed. **The stale
   `twm_mobo.db`/`twm_pareto_arm*.json` were generated against the OLD (broken) objA —
   they must be regenerated by re-running the sweep before the next selection.** The
   original defect, for the record: in refine mode objA was the
   argmin over **all 88** refined candidates — i.e. exactly the **K=88 exhaustive
   refinement** that ADR 0006 Finding #1 measured as the *worst* real setting (61/87),
   because refining every candidate re-exposes the true key to the dense-bass/adjacent
   attractors the shipped K=3 Stage-A gate excludes. Production is K=3
   (`prod_false_lock`, diagnostic-only). Review 2's evidence this bias is real:
   (a) the objA↔prod_fl gap **sign-flips** — conservative configs have prod_fl > objA
   (M&B 0.289→0.308) but the search optimum has prod_fl < objA (0.2005→0.1975); ~7.5%
   of trial-pairs disagree on direction, so minimizing objA does not monotonically
   minimize production loss. (b) The objA optimum is **degenerate**: at the arm-4/5
   optimum (p≈0.785, λ≈1.757) `floor_frac=0.22` — on 22% of frames ≥44/88 keys score
   ≈0 (collapsed-error regime). Moving λ→18 *or* p→0.5 individually removes the floor
   and ~doubles objA (0.20→0.39/0.42). So objA is *minimized by entering* the very
   error-collapse the ordinal-objB switch (0006 #2) was meant to retire — objB simply
   no longer *sees* the collapse, so nothing steers the search away. The "floor 0.0%"
   health check (§4) is config-cherry-picked (holds at the conservative config, not at
   the optimizer's operating point). Only the n=1 real gate stops objA shipping its
   own optimum.

4. **[CONFIRMED · external validity] Single real instrument.** The decision gate is one
   1–2-yr-out-of-tune upright; "winning on real" = winning on *that* piano. No second
   instrument, no in-tune regime. (See review verdict: highest-value fix.)

5. **[CONFIRMED · external validity] Selection-on-real is itself a fitting procedure.**
   We don't merely validate on the one piano, we *select* best-of-N Pareto candidates
   by its pass-count — fitting the sample. The chosen config's gain is concentrated in
   the manual-mode extreme treble (A6–C8); bass moved only +1. (Quantified in §8.6.)

6. **[UPGRADED → HIGH · statistics] The headline win is not statistically significant.**
   Not merely "small deltas are suggestive": Review 2 ran the test. Shipped-vs-M&B
   (both refined) net +4 rests on **6 discordant keys** — gained {C#1, A6, G7, G#7,
   C8}, lost {G6} — and **4 of 5 gains are extreme treble A6–C8** (the info-limited
   manual-mode register). Bass (the entire rationale for raising q/r) moved **+1**
   (C#1 only), despite synthetic bass false-lock improving monotonically
   (0.269→0.251). **McNemar exact on the 6 flips: two-sided p≈0.22** (one-sided 0.11)
   — indistinguishable from a coin-flip on its own validation captures, *before*
   instrument-to-instrument variance (which n=1 cannot estimate).

7. **[CONFIRMED → ✅ RESOLVED 2026-06-23/24 · landscape] Structural-term rejections —
   now settled by the valid co-tuned test.** Arms 6–7 made `nonpeak`/`smoothness`
   *free* params co-tuned with q/r/ρ (the test 0006 Finding #3 required). **Result —
   the synthetic and the real piano disagree, and real wins:**
   - **Synthetic:** co-tuned `nonpeak` (arm 6) *Pareto-dominated* the no-structural
     arm 2 (100% of arm 2 dominated; overall FL 0.254→0.207) at small nonpeak ≈0.05 —
     it *looked* like a clear win. `smoothness` (arm 7) did **not** help even on
     synthetic (arm 2 dominated 94% of it).
   - **Real (the decision):** the co-tuned `nonpeak` **hurts** — same q/r/ρ, nonpeak
     0.047 vs off = 68/87 (bass 14) vs **77/87 (bass 21)**; arm-6 min-bass (nonpeak
     0.022) = 72/87 (bass 17). The term drops bass even co-tuned.
   - **Verdict: both structural terms rejected on real, by the valid test.** Coherent
     with the oracle-B finding (§8.2): real bass is *B-limited* (wrong template shape),
     so charging predicted-but-absent partials penalizes the true note whose mis-shaped
     template already "predicts" missing partials. The synthetic's bass B sits near the
     prior, so it doesn't show this → **the synthetic overstated `nonpeak`; real
     validation caught it** (a clean tune-synthetic/decide-real save). Deadzone's
     rejection was already mechanism-robust and was not re-run.
   - **Side-finding (candidate, NOT adopted):** the nonpeak-*off* high-q (7.68)/high-ρ
     (0.49) config scored 77/87 (bass 21, treble 24) on the one piano, +3 over the
     provisional default — but n=1 (prior +3 was McNemar p≈0.22), so it is queued for
     second-instrument validation, not adopted.

8. **[CLEARED-in-part · front end] Peak *contract* is faithful; only detection noise is
   unmodeled.** Review 1 retracts an earlier "contract mismatch" worry: the engine's
   input is top-64 + `mask_peaks` — **the same contract the synthetic implements**
   (`mobo_evaluator.rs:391-394`); `peaks.csv`'s 128 is a pre-cap diagnostic scratch,
   not the engine input. What remains true: the synthetic does not reproduce the
   FFT/Jacobsen/Neyman-Pearson *detection* stage (only 0.2 Hz + lognormal jitter), so
   amplitude-dependent conclusions are still the least trustworthy.

9. **[SHARPENED · model-fidelity] Hand-tuned generative constants; bass envelope too
   smooth.** Strata weights, σ_B split, resonance probs, α ranges, missing-fundamental
   probs, noise levels are literature-anchored but *chosen*; none calibrated to
   measured real statistics. Review 1 adds direction in the bass: real bass spectra
   have **formant-like dominant high partials** (e.g. D1: n5/n12 loudest, fundamental
   ~50× weaker) that the smooth `n^−α` (α∈[0.4,0.9]) rarely reproduces even with
   partials 1–3 dropped → amplitude terms (q,r) look more trustworthy than real bass
   supports. (The precise log-residual multiple was fit-noise; the direction holds.)

10. **[EXTENDED → ✅ ADDRESSED 2026-06-20 · LOW] HV reference point.** **Resolution:** with
   objB now a real second objective (treble FL) the HV is genuinely 2-D, and it was
   re-derived for *minimize-minimize* against ref (1.0, 1.0) — which is the actual
   worst corner and *within* the [0,1] support of both objectives (not the old
   out-of-support `ref_x=1.0` vs an objA living at 0.2–0.3). Cross-arm HV is still only
   used for the plateau-stopper, never final selection. Original issue, for the record:
   with the flat ordinal objB, HV ≈ a monotone function of the single best-objA corner
   and the stopper *under*-stopped.

11. **[QUANTIFIED → ✅ MITIGATED 2026-06-20 · MED] Seed-fragility: the synthetic optimum is
   seed noise.** **Resolution:** the re-sweep runs **3 seeds (42/1/7) pooled** and
   enlarges the population **50→128** (the root cause: premature convergence in 5-D at
   pop=50). The candidate menu handed to real validation is now the seed-union, not one
   realization. Note this *mitigates* (broader coverage) rather than *eliminates* —
   the synthetic optimum *location* is still not to be trusted as informative; that's
   why final selection is on real data. Original evidence: equal 200-trial budget,
   objA *value* seed-stable (~0.247 arm-2) but *location* not — arm-2 q 5.70–7.60;
   arm-4 **λ 1.76 / 14.2 / 15.1** (8×) and p 0.65–0.79; the "p≈0.8, λ≈1.5 overfit"
   (0006 #6) is a seed-42 artifact; shipped q=3.88 sits *below* every seed-optimum,
   confirming it came from the real plateau.

12. **[CONFIRMED → ✅ FIXED 2026-06-20 · robustness] `unwrap` on NaN comparisons.** All
   score comparisons in `mobo_evaluator.rs` now use `f32::total_cmp` (a total order),
   so a NaN score can no longer panic a trial — the sweep degrades gracefully instead
   of dying. (NaN remains unreachable on the current dataset; this is insurance for the
   multi-hour run.)

13. **[NEW → ✅ ADDRESSED 2026-06-20 · MED] objB was a near-degenerate second objective.**
   **Resolution:** objB was replaced with **treble false-lock** (objA = bass FL), an
   *orthogonal* tradeoff that produces a genuine 2-D front (§4). Original evidence: the
   old ordinal objB varied <0.001 across the entire front (arm 2: 0.9895–0.9905 over
   174 "non-dominated" points; arm 4: 0.00014 spread) — NSGA-II was effectively
   single-objective on objA with objB a 4th-decimal tiebreaker, undercutting 0006 #6.
   The bass-vs-treble redesign makes the multi-objective framing real (and the front a
   decision-relevant register-tradeoff menu for selection).

14. **[NEW · MED — partially addressed 2026-06-20] Auditability: the evidence chain is
   unversioned.** ✅ Optuna now pinned (`scripts/requirements.txt`, `optuna==4.9.0`).
   ✅ The orchestrator now parses the fingerprint from the evaluator's ready line and
   **asserts it == `e11fea90889dee30`** before running (a drifted dataset is refused,
   not silently accepted). ⏳ Still open: `optimize_twm.py`, `validate_config.py`, this
   doc, `twm_mobo.db`, the Pareto JSONs, and the real captures (`diagnostics/` is
   gitignored) remain **untracked** — they must be committed *together* so the
   evidence chain is reproducible (the user is deferring the commit until the MOBO
   work is complete). Nothing yet binds a *resumed* `twm_mobo.db`'s existing trials to
   the fingerprint (the assert guards a fresh run, not historical trials in the db).

15. **[NEW · LOW] HOLDOUT_KEYS is only a partial holdout.** The 6 "held-out" keys are
   excluded from hard oversampling but still contribute 100 base frames each to
   objA/objB. Minor; doesn't affect conclusions.

> **[CLEARED] objB threading is deterministic.** An earlier worry that the parallel
> `margin_sum` f64 reduction (chunked by core count) could perturb objB is **not** a
> defect: objB is serialized at 6 decimals and is bit-identical across 1/2/16 cores;
> the fingerprint is single-threaded. Determinism claims hold.

### Where this most plausibly "messed up" (revised)

If a real-world result contradicts the MOBO ranking, suspect — in order — (1) the
**objective itself** (§8.3: objA optimises the rejected K=88 regime and its optimum is
degenerate), (2) the **seed-fragile, objB-degenerate search** (§8.11/§8.13: the
synthetic optimum is noise), (3) the **bass B-prior gap** (§8.2: one-directional,
unquantified), and (4) **selection-overfit to one piano** (§8.5/§8.6: p≈0.22). The
first two say *do not trust the synthetic optimum at all*; the last two say *the one
real piano cannot carry the selection alone* — hence the second-instrument
requirement above.
