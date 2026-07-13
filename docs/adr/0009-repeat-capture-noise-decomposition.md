# ADR 0009 — Repeat-Capture Noise Decomposition and Precision-Weighted Curve B (Prompt H)

## Status

**Accepted (2026-07-10).** Records the repeat-capture experiment on
instrument #2 (an upright distinct from the original validation upright)
and the one code change it licensed: the `CURVE_B_MIN_PARTIALS = 8` hard
trust switch is replaced by **precision-weighted (inverse-variance)
shrinkage** of measured B toward the B_ξ fit. Everything else below is a
measurement on record — the six Prompt-G/ADR-0007 open questions (the six
(b) flags, the ρ signal-vs-noise question, strike-strength dependence,
chain-noise independence, the keys-40–51 zone, and capture duration) are
closed by data, and one candidate refinement (a conditioning-weighted ρ
fit) is refuted and *not* ported.
Attribution discipline (ADR 0007/0008 precedent): the shrinkage is the only
engine change; engine (a) and the entire (c) calibration stage (ρ points,
reg weight, φ) are byte-identical before/after on both instruments.

## Context

Three findings from the Prompt-F/G review series converged on one missing
measurement — per-key capture-to-capture variance:

1. The ρ points feeding engine (c) looked noise-dominated (LOO error
   ≈ 1.2 ρ-units, flat across four decades of regularization — ADR 0008
   Decision 3), but capture-condition noise and estimator noise could not
   be told apart.
2. The treble target level was threshold-sensitive at ±5 ¢ under the
   `CURVE_B_MIN_PARTIALS` trust boundary (ADR 0007, characterization #4) —
   a principled threshold was blocked on σ_B.
3. The keys 40–51 below-fit B zone (the six (b) negative-stretch flags)
   was either real string design or MAT bias; repeats on a second
   instrument distinguish.

**The data.** 595 capture dumps across all 88 keys of instrument #2
(timestamped worker dumps in `diagnostics/`), every key n ≥ 5, the
crossover/deep subset (keys 39–56) at n = 10–16, strike strength
deliberately varied (measured spectral-power spread 0.5–7.8 dB ≈ mf–f).
Five wrong-strike dumps were discarded at audit; one D7 capture carries
B = 0 and is excluded by the standing validity rules. **Consumption rule**
(load-bearing): deep-bass `analysis.json` files written before the
`worker::MAT_SEED_TOLERANCE` fix carry rumble-seeded garbage — all
analysis reads the audio through
`cargo run --release --example regenerate_partials -- diagnostics`,
never the raw JSONs. Audit: `scripts/audit_captures.py` on the regen JSON.

**The harness.** `examples/repeat_noise.rs` (new, kept — the standing
consumer for repeat sets): per-octave-pair capture-combination sweeps
through the exact engine-(c) path (gate → coincidence-bracket scan →
Eq.-30 inversion), strike-strength regressions, and 24 deterministic
resampled draws (one capture per key) through the raw chain, the per-draw
φ fit, and engines (b), (c), (d)-BALANCED, (d)-octaves-only.
Post-processing scripts were scratchpad-ephemeral (ADR-0007 probe
precedent); their outputs are recorded here.

## Analysis 1 — σ_B(key, partial count)

Per-key SD of ln B across repeats (σ_lnB; multiplicative noise, so the log
is the right coordinate), against the persisted partial count n:

| partials n | keys | median σ_lnB | as % of B |
| --- | --- | --- | --- |
| 28–32 | 36 | 0.0036 | 0.4 % |
| 20–27 | 8 | 0.0033 | 0.3 % |
| 14–19 | 5 | 0.0050 | 0.5 % |
| 10–13 | 8 | 0.0128 | 1.3 % |
| 8–9 | 2 | 0.0215 | 2.2 % |
| 6–7 | 8 | 0.0778 | 8.1 % |
| 4–5 | 18 | 0.2393 | 27 % (max 1.44 ≈ ×4) |

Register medians: bass 0.4 %, mid 0.5 %, **treble 26 %** (worst key 73:
σ_lnB 2.07 ≈ ×8 spread). MAT f₀ repeatability: bass 0.16 ¢, mid 0.47 ¢,
treble 6.7 ¢ (worst 38 ¢). The treble numbers are the information floor
measured live — exactly the honest-variance signal the audit chose to keep.

Least squares of ln σ on ln n over the non-floor bins gives slope
**−3.00** and coefficient 19.3, with a bass/mid plateau at 0.0035:

    σ_m(n) = max( 19.3 · n⁻³ , 0.0035 ).

The **prior scatter** σ_p — the spread of real per-key B structure about
the 2-parameter B_ξ fit, measured where repeat noise is negligible
(σ_m ≤ 0.02 ⇔ n ≥ 10) as 1.4826 × MAD of ln(B_meas/B_ξ) (1.4826 =
1/Φ⁻¹(3/4), the normal-consistency constant of the MAD) — is
**0.062 on instrument #2** (66 keys; plain SD 0.091) versus
**0.186 on instrument #1** (51 keys; SD 0.215): 3× apart, so σ_p must be
self-calibrated per instrument, never a constant.

Two structural facts fall out:

* **Fit residuals ≫ repeat noise everywhere below the treble** (6–19 %
  structure vs 0.4 % noise): the per-key deviations from B_ξ are *real
  string-scale structure*, not estimator scatter — the engines that
  consume measured per-key B are consuming signal.
* The σ cliff sits between n = 5 and n = 8 — the old threshold's location
  was defensible; its *hard* form was the problem (see Decision 1).

## Decision 1 — Precision-weighted curve-B shrinkage (implemented)

**Derivation.** With ln B_meas | ln B ~ N(ln B, σ_m²) and the prior
ln B ~ N(ln B_ξ, σ_p²), the posterior density is the product of two
Gaussians; completing the square in ln B gives the posterior mean

    ln B_curve = ( ln B_meas/σ_m² + ln B_ξ/σ_p² ) / ( 1/σ_m² + 1/σ_p² )
               = w · ln B_meas + (1−w) · ln B_ξ ,  w = σ_p²/(σ_p² + σ_m²)

— the inverse-variance-weighted mean of the two estimates (the standard
fixed-effect/meta-analytic combination; the Efron–Morris/empirical-Bayes
shrinkage family). This **is** the design note §8 pair-count-weighted
blend, with pair count replaced by measured precision. σ_m from the
repeat-measured model above (`tuning::sigma_ln_b`); σ_p self-calibrated
per instrument (`tuning::sigma_prior`, MAD form above; default 0.12 —
between the two measured instruments — below 4 calibrating keys, floor
0.01 against near-interpolating small fits).

**Semantics kept.** `b_is_measured` (chain gauge, smoother data weights,
LKO reference) now means *measurement-dominated*, w ≥ 1/2 ⇔ σ_m ≤ σ_p —
the point of equal information, a derived boundary that only grades keys
as data/prior; the B **value** is continuous across it, so no curve
artifact can park on it (the ADR-0007 failure mode of the hard switch).
The §2 detector is untouched (exclusion still swaps to the pure fit);
strobe targets still use raw measured B always (§5).

**Deltas (the attributable harness re-run, both instruments).** Engine (a)
byte-identical; ρ points/reg/φ byte-identical; movement concentrated where
the trust boundary used to sit:

| | instrument #1 (87 keys) | instrument #2 (88 keys) |
| --- | --- | --- |
| prior-dominated keys (`curve_b_fallback`) | 33 → 21 | 28 → 23 |
| (b) negative-stretch flags | **6 → 0** | 0 → 0 |
| (b) A7 / C8 (¢) | 28.9/37.2 → 33.3/43.3 | 28.5/36.8 → 26.7/34.2 |
| max per-key shift, (b) | 6.1 ¢ (key 87) | 2.6 ¢ (key 87) |
| max per-key shift, (d)-BAL | 0.7 ¢ | 0.4 ¢ |
| Giordano cross-score (b)/(c)/(d) | all improve | all improve |
| LKO bass/mid, (b) | 2.74/1.67 → 2.49/1.52 | 0.56/0.48 → 0.51/0.45 |

The six ADR-0007 (b) flags were **a boundary artifact of the hard switch**:
with the boundary gone (upper-of-pair B now shrunk by its own precision
instead of hard-trusted), the marginal −0.14…−0.34 ¢ descents flip
positive. The instrument-#1 treble rises to ≈ the old thr-5
characterization value (+33.5 ¢ at A7) — the blend reaches the same place
a *looser* threshold did, but smoothly and by measured precision. The
ADR-0007 "±5 ¢ threshold sensitivity" question is not answered but
**dissolved**: there is no threshold left to place. (LKO before/after is
indicative only — the reference chain's measured-key set moves with the
boundary.)

**Honest caveat.** σ_m(n) is calibrated on instrument #2's repeats only;
applying it to instrument #1 assumes the noise-vs-partial-count law
transfers. The law's steepness makes the blend robust to 2× calibration
error except within ~1 partial of the σ_m = σ_p crossover, and the
crossover itself adapts per instrument through σ_p.

## Analysis 2 — ρ-point reproducibility and Eq.-30 conditioning

Per octave pair, all (lower × upper) capture combinations through the
engine-(c) path (25–224 combos/pair). Accepted-pair register medians:

| register | pairs | median σ_width | median σ_ρ | median |∂ρ/∂w| |
| --- | --- | --- | --- | --- |
| bass (m 0–27) | 27 | 0.60 ¢ | 0.137 | 0.26 ρ/¢ |
| mid (m 28–52) | 16 | 0.57 ¢ | 0.423 | 0.66 ρ/¢ |

* **The conditioning explains the noise.** |∂ρ/∂w| (central difference of
  the exact Eq.-30 inversion under a ±1 ¢ width perturbation) times the
  observed σ_width predicts the observed σ_ρ almost exactly, pair by pair
  (e.g. m=8: 0.046 predicted vs 0.046 observed; m=11: 0.049/0.049; m=25:
  0.082/0.075). The ρ = 7.8-class blow-ups are ill-conditioning of the
  inversion (numerator → 0 as the width approaches the B-free octave), as
  ADR 0008 hypothesized — and conditioning worsens toward the mid, where
  ρ itself loses meaning.
* **The scan optimum is reproducible** (σ_width ≈ 0.6 ¢) — so the huge
  pair-to-pair ρ scatter (adjacent deep-bass pairs at ρ = 6.05, 2.33,
  5.26, 2.43…) is **reproducible structure, not capture noise**. The
  Eq.-9 three-parameter family cannot express it; the ρ-fit's LOO error
  (≈ 1.25 flat, same as instrument #1's 1.2) is **model-misfit-dominated**
  — the fit error is ~5–10× the per-point measurement noise.
* **φ is draw-stable**: across 24 resampled draws, κ = 3.29 ± 0.33,
  m₀ = 59.7 ± 1.2, α = 24.8 ± 4.5, ρ(A0) = 4.24 ± 0.32 (37–39 points per
  draw). Engine (c)'s calibration does carry instrument signal beyond
  "bass κ somewhat below typical" — but at ~±0.3 ρ at A0 under capture
  resampling, it is the noisiest engine (analysis 4).
* **Weighted Eq.-31 fit: refuted, not ported.** A Python replica of
  `fit_rho_phi` (verified exact against the Rust: (3.191, 58.3, 26.1) on
  the canonical instrument-#2 points) was run with per-point weights
  w_i ∝ 1/(|∂ρ/∂w|_i · σ_w)² — the analytic error propagation, computable
  from a single capture. The weighted LOO error is *higher* at every
  regularization weight (1.48–2.12 vs 1.25–1.43 unweighted) and the
  weighted 1-SE selection collapses to the grid edge (reg = 100,
  φ ≈ prior). Weighting re-emphasizes well-conditioned bass points whose
  scatter is model misfit — it cannot help. **The CV does not sharpen; the
  1-SE rule remains load-bearing** (ADR 0008 Decision 3 stands).
* Gate stability note: 8 of 76 pairs flip their gate verdict across combos
  (5–95 % acceptance) — the §VI.C gate itself carries mild capture
  sensitivity at its boundary; recorded, no action (the ρ fit is
  regularized against exactly this).

## Analysis 3 — Strike-strength sensitivity of the Giordano optimum

Per-pair OLS of optimal width on combo spectral power (dB), over the 42
pairs with ≥ 10 accepted combos and > 2 dB span: median |slope × span| =
**0.63 ¢**, versus median per-pair σ_width 0.61 ¢ — the amplitude-condition
dependence is *within*, not on top of, the overall repeat noise. Worst
cases (+4.8 ¢ at m=6, +5.1 ¢ at m=40) are the pairs whose σ_width is
already large. The design note §3.2's unquantified amplitude-dependence
concern is measured **benign at mf–f dynamics**; no code change.

## Analysis 4 — Chain noise vs LOO independence (Prompt G deferred item)

24 resampled draws (one capture per key, deterministic xorshift):

* Raw-chain per-key SD: bass/mid ≈ 0.08 ¢, treble 0.17 ¢ (max 1.16 ¢,
  key 87). Chain-noise spatial correlation: lag-1 r = +0.148, lag-12
  (same chain) r = +0.149 — **mild, and no excess same-chain correlation**:
  with σ_lnB this small the Eq.-6 chains do not measurably accumulate
  shared noise, so the LOO-CV independence assumption is adequate on real
  capture noise. Item closed.
* Per-engine curve SD across draws (bass/mid/treble medians, ¢):
  (b) 0.20/0.22/0.78; **(c) 2.21/0.33/0.97 (max 5.97 at A0)** — the φ
  draw-variance made visible; (d)-BALANCED 0.02/0.02/0.07 (the most
  stable, as its heavy regularization predicts); (d)-octaves-only
  0.16/0.03/0.21. Engine-noise correlation (b) vs (d)-octaves-only
  r = +0.14. Capture noise costs ≤ ~2 ¢ anywhere on the curve — an order
  below the engine-to-engine differences; curve-noise is not a blocker
  for any engine, and (c)'s deep-bass ±2 ¢ is the price of its measured
  taste layer.

## Analysis 5 — The keys 40–51 below-fit zone: string design, not MAT bias

Instrument #2 does **not** reproduce instrument #1's crossover-zone
pattern: its residual run at keys 40–49 sits *above* the fit (+0.02…+0.09
ln-units, each many repeat-SEs from zero), with no sign flip at key 52;
its own coherent below-fit zone is at **keys 28–31** (−0.13…−0.35,
z up to ~240 vs repeat noise). A MAT bias would sit at the same keys on
both instruments; instrument-specific location and sign ⇒ the
bridge-crossover misfit is **real per-instrument string-design structure**
the 2-parameter B_ξ family cannot follow. ADR 0007's open alternative is
resolved in favor of the instrument. (This also independently justifies
Decision 1's trust of precise measured B over the fit.)

## Analysis 6 — Capture-duration contingency: closed, keep 1.5 s

The revisit condition was "σ_B dominated by within-capture noise in the
bass". Measured total bass σ_B is 0.1–1.2 % — there is nothing left for a
within-capture component to dominate; a 2¹⁷ window could only chase
tenths of a percent while integrating drift/unison beating and outliving
treble sustain. Contingency closed; the stable window stays 66,150
samples / 2¹⁶ FFT.

## Re-verification checklist (Prompt-G completion gate)

* `CURVE_B_MIN_PARTIALS` / treble ±5 ¢ sensitivity — **resolved by
  construction** (Decision 1; no threshold exists).
* Six (b) negative-stretch flags — **boundary artifact of the hard
  switch** (gone under shrinkage) sitting on **real string structure**
  (analysis 5).
* ρ-noise decomposition → (c) trust — points are precise (σ_ρ 0.14–0.42),
  scatter is model misfit; φ stable to ±0.33 in κ; (c) usable, noisiest of
  the four (analysis 2/4).
* 1-SE tie-breaks — **do not sharpen**; weighted fit refuted; rule stays
  (analysis 2).
* Chain-noise vs LOO independence — mild (r ≈ 0.15); closed (analysis 4).
* Still open (TWM-side, not this experiment): fresh instrument-#2 TWM
  baselines, register-sparsity gate, measured-B→discovery, bass-B 7–25×
  confirmation on a *tuned* second instrument, Prompt D.

## Verification

61/61 lib tests (new: `test_curve_b_shrinkage` — σ-model shape, σ_p
self-calibration on on-model vs deviating profiles, blend flag semantics,
shrinkage-is-not-exclusion). Harness re-runs on both instruments recorded
above; `curve_analysis.png` + `curve_report.json` regenerated in
`diagnostics_piano_1/` and `diagnostics_piano_2/`
(`curve_compare --json` → `scripts/plot_curves.py`). Clippy clean on all
touched code. Nothing committed (user decision pending).

## References

* Rigaud, David & Daudet 2013, JASA 133(5) — Eqs. 9, 29–31 (fit machinery),
  Eq. 30 (the inversion whose conditioning analysis 2 measures).
* Giordano 2015, JASA 138(4) — §VI.C gate, scan machinery (ADR 0008).
* Hastie, Tibshirani & Friedman, *ESL* 2nd ed. §7.10 — the 1-SE rule
  (ADR 0008; re-affirmed here).
* Inverse-variance weighting / conjugate-normal posterior mean — derived
  in Decision 1 (self-contained); the Efron–Morris/empirical-Bayes
  shrinkage family is the named lineage.
* MAD scale estimation, consistency constant 1.4826 = 1/Φ⁻¹(3/4) —
  derived from the normal quartile; standard robust-statistics form.
