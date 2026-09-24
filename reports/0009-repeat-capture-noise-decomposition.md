# report 0009 — repeat-capture noise decomposition

Nine analyses over 595 repeat captures on the second upright — the project's
first data able to separate *measurement* noise from *instrument* structure,
because it is the first set with ≥ 5 repeats per key.

The headline is Analysis 1: σ_lnB falls as $n^{-3}$ in partial count, with the
exponent measured at exactly −3. That law is what makes Decision 1 possible, and
it is also the reason the decision is robust — the steepness means a 2×
calibration error only matters within about one partial of the crossover.

Analyses 7–9 were added later (2026-08-25 to 09-02) and answer questions raised
elsewhere: the top-octave $B$ ceiling, engine (d) on the model's $B$, and where
the analysis window starts.

## Where this stands

**Accepted (2026-07-10).** Records the repeat-capture experiment on
instrument #2 (an upright distinct from the original validation upright)
and the one code change it licensed: the `CURVE_B_MIN_PARTIALS = 8` hard
trust switch is replaced by **precision-weighted (inverse-variance)
shrinkage** of measured B toward the B_ξ fit. Everything else below is a
measurement on record — the six Prompt-G/report 0007 open questions (the six
(b) flags, the ρ signal-vs-noise question, strike-strength dependence,
chain-noise independence, the keys-40–51 zone, and capture duration) are
closed by data, and one candidate refinement (a conditioning-weighted ρ
fit) is refuted and *not* ported.
Attribution discipline (report 0007/0008 precedent): the shrinkage is the only
engine change; engine (a) and the entire (c) calibration stage (ρ points,
reg weight, φ) are byte-identical before/after on both instruments.

## Context

Three findings from the Prompt-F/G review series converged on one missing
measurement — per-key capture-to-capture variance:

1. The ρ points feeding engine (c) looked noise-dominated (LOO error
   ≈ 1.2 ρ-units, flat across four decades of regularization — report 0008
   Decision 3), but capture-condition noise and estimator noise could not
   be told apart.
2. The treble target level was threshold-sensitive at ±5 ¢ under the
   `CURVE_B_MIN_PARTIALS` trust boundary (report 0007, characterization #4) —
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
`cargo lab mat regen diagnostics`,
never the raw JSONs. Audit: `tuner-lab/scripts/audit_captures.py` on the regen JSON.

**The harness.** `cargo lab mat repeats` (kept — the standing
consumer for repeat sets): per-octave-pair capture-combination sweeps
through the exact engine-(c) path (gate → coincidence-bracket scan →
Eq.-30 inversion), strike-strength regressions, and 24 deterministic
resampled draws (one capture per key) through the raw chain, the per-draw
φ fit, and engines (b), (c), (d)-BALANCED, (d)-octaves-only.
Post-processing scripts were scratchpad-ephemeral (report 0007 probe
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

## The shrinkage, derived — and what it moved

Decided in report 0009.

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
artifact can park on it (the report 0007 failure mode of the hard switch).
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

The six report 0007 (b) flags were **a boundary artifact of the hard switch**:
with the boundary gone (upper-of-pair B now shrunk by its own precision
instead of hard-trusted), the marginal −0.14…−0.34 ¢ descents flip
positive. The instrument-#1 treble rises to ≈ the old thr-5
characterization value (+33.5 ¢ at A7) — the blend reaches the same place
a *looser* threshold did, but smoothly and by measured precision. The
report 0007 "±5 ¢ threshold sensitivity" question is not answered but
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
  report 0008 hypothesized — and conditioning worsens toward the mid, where
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
  1-SE rule remains load-bearing** (report 0008 Decision 3 stands).
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

## Analysis 4 — Chain noise vs LOO independence (deferred from the curve review)

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
the 2-parameter B_ξ family cannot follow. report 0007's open alternative is
resolved in favor of the instrument. (This also independently justifies
Decision 1's trust of precise measured B over the fit.)

## Analysis 6 — Capture-duration contingency: closed, keep 1.5 s

The revisit condition was "σ_B dominated by within-capture noise in the
bass". Measured total bass σ_B is 0.1–1.2 % — there is nothing left for a
within-capture component to dominate; a 2¹⁷ window could only chase
tenths of a percent while integrating drift/unison beating and outliving
treble sustain. Contingency closed; the stable window stays 66,150
samples / 2¹⁶ FFT.

## Analysis 7 — The top-octave B ceiling: amplitude, not bandwidth (added 2026-08-25)

Recorded after acceptance; it moves no shipped value. Analysis 1 measured the
treble as the information floor (median 26 %, worst key 38 ¢). This is the
mechanism behind that number, the levers measured against it, and the one lever
still open.

**The bandwidth explanation is the obvious one, and it is wrong.** Nyquist caps
the partial count independently of capture technique:

| key | f₁ (Hz) | max n @ 44.1 kHz | @ 96 kHz |
| --- | --- | --- | --- |
| A6  | 1760 | 9 | 16 |
| D#7 | 2489 | 7 | 11 |
| A7  | 3520 | 5 | 8 |
| C8  | 4186 | 4 | 7 |

so the ~7 partials the σ_m ≤ σ_p crossover needs are unreachable above D#7
(key 78) at 44.1 kHz. But count is a *proxy* in the σ_m law, not the cause: a
capture holds few partials because the ones beyond it were too quiet to detect.
Measured over instrument #2's captures, relative to each key's own fundamental:

| n | treble (keys 62–87) | bass/tenor (keys 0–47) |
| --- | --- | --- |
| 2  | −28.3 dB | **+8.2 dB** |
| 3  | −40.7 dB | +5.2 dB |
| 4  | −49.8 dB | +3.9 dB |
| 5  | −53.0 dB | — |
| 6+ | −56 to −60 dB | +2.4 dB (n = 6) |

A treble string radiates almost everything through its fundamental; a bass
string puts *more* energy into its upper partials than into f₁. That 35–55 dB
swing, not the band edge, is why B collapses above A6 — and it means a higher
sample rate would buy partials at **−53 to −60 dB, weaker than the ones already
failing**. Nothing here establishes that 96 kHz would recover treble B, and the
amplitude trend argues it would not: the sample rate is not the binding
constraint, and raising it is not a measured fix. That is a statement about the
*curve*. A selectable rate is planned for its own reasons (ARCHITECTURE.md,
"Hardcoded 44.1kHz"), and more treble partials on record would still serve
analysis even where they cannot move the curve. Raw evidence from the same
captures — partial 1 repeats to ±0.2 Hz at A6 (1.1e-4 relative) while partial 2
of those captures scatters across 30 Hz.

**What arrives where the amplitude goes.** The collapse is not noise winning an
empty band; it is a specific competitor. Normalising every stored partial's
position (0 = the exact harmonic n·f₁, 1 = the model-stretched position): below
A6, 0–1 % of located partials sit at the exact harmonic and 83 % near the model;
from A6 up, **24–27 % sit at exact integer multiples of f₁**, to the hertz,
identically across captures — A#6's partials 3, 4, 5 land within ±2 Hz of 3f₁,
4f₁, 5f₁ where its own partials belong +158, +391, +771 Hz higher. A string's
partial is never harmonic and noise does not repeat; exact harmonics of a
dominant fundamental are a **nonlinear product** of it — from the recording
chain or from the instrument (phantom partials), which this data does not
separate. A product at n·f₁ reads as B = 0, so it can only pull the median
toward zero, and MAT's median cannot save the capture: one wrong partial poisons
K−1 pairs, and A#6's one correct pair (6.6e-3, on the model) loses 14 to 1.

A dedicated primer on this mechanism — the full derivation, with figures — is
planned, and is the right place to read the maths at length. Until it lands,
this analysis is the record.

**Levers measured, both marginal.**

* **Estimator form.** Replacing MAT's full-trajectory median with a
  pre-registered single-pair B (fixed n = 3) moves the treble repeat log-SD from
  0.137 to 0.102 and the count of keys beating the prior from 9/25 to 10/25. A
  *post-hoc best pair per key* looks 3.7× better; that is selection on the test
  statistic and does not survive pre-registration
  ([`07-evidence-and-methodology.md`](README.md)).
* **Pooling repeats.** The profile stores 3–8 captures per key and the curve
  reads exactly one (`active()`). Taking the median across them gives
  9/25 → 11/25. It cannot rescue the keys that fail hardest (A#6, B6, G#7, A7
  all sit above 0.9 log-SD, where √N buys nothing). It is what stabilises the
  treble-asymptote fit: reading the newest capture per key instead of the per-key
  median moves that fit's slope by 0.9 SE
  ([faithfulness-audit-06](retiring/faithfulness-audit-06-b-prior.md)).

**Lever not yet tested: the MAT search band.** MAT admits a partial within
±f₀/4 of its predicted position — wide enough to admit these nonlinear products
where a tighter band would exclude them. A uniform tightening is not available:
the bass needs the band that wide for an unrelated reason, since its bootstrap
pair can land ~300 % off and the wide band absorbs the error. An
uncertainty-sized band is the candidate form.

**What the ceiling costs.** Sweeping the treble B across 0.5×–2× of the model
moves engine (d)'s C8 target by **29.9 ¢** and A7 by **21.3 ¢** on instrument
#2. That is the size of the top-octave modelling assumption. It does not reach
the operator's readout: above key 48 the strobe and the coarse read both target
n = 1, and f₁* carries no B.

**The one lever still open: window placement.** Treble upper partials decay far
faster than their fundamental, and the Golden Window (sustain stability, State 3) waits for
post-attack stability — which up there lands after the collapse. Measured on
instrument #2's full-event dumps, in dB relative to the fundamental at each
instant: A6's partials 2–6 sit at **−2 to −8 dB** through the first 0.1 s and
fall to −25/−37/−39/−42/−45 by t = 0.19 s; C8's partial 3 is **+15 dB** — louder
than f₁ — at the attack and −33 dB by t = 0.28 s. The informative segment up
there really is early, by 20–45 dB.

Two caveats before acting on it. The first ~0.1 s contains the broadband hammer
transient, so part of that energy is not resolved partials at all; and A6's
collapse by t = 0.19 s means the gap between "transient over" and "partials
gone" may be too narrow to use. A negative result — that the treble has no
window which is both post-transient and partial-rich — is a real outcome and
would close the question. The test is offline against the piano-#2 full-event
dumps (every treble key is covered) and changes no gatekeeper code unless it
succeeds.

## Analysis 8 — Engine (d) on the model's B instead of the blend (added 2026-08-26)

Recorded after acceptance; it moves no shipped value. Decision 1 made the
curve-side B a blend of the key's measurement and the B_ξ fit — ours, not
Rigaud's — and engine (d) builds every interval width from it. The
paper-backed alternative is to build the widths from $B_\xi$ alone, and it had
never been run. This is the run.

**Build.** `CurveBSource::{Blend, Model}` on `CurveParams` (default `Blend`,
so the shipped curve is unchanged), read only by `multi_interval`'s width
computation; `b_is_measured` (row existence), the Form-2 amplitude weights, the
§2 pre-exclusion and the prior all stay on the blend, so the one difference is
the B the widths read. Harness rows: `curve_compare` "d: model-B widths",
`auralize` `d_model-b.wav`.

**Data — the two instrument-#2 sessions, and only those.** `diagnostics_piano2/`
(595 dumps, 5–6 repeats/key, report 0009's own set — "A" below) and the extended
as-found set under the app's dump directory for profile `Piano2_extended`
(587 dumps, ~4 repeats/key, 8 isolation series — "B"). Instrument #1 is excluded
on the user's instruction: one capture per key cannot separate a string's real
deviation from a bad capture, which is the whole question here. Both sets are
the same physical piano in its as-found state, captured about a month apart,
which makes a **cross-session** test possible for the first time.

Two harness loaders had to be fixed first: `curve_compare` and `auralize` both
discarded `sounding_strings`, so an isolation solo could stand for the note
(`KeyMeasurement::is_partial_unison`, which the shipped path honours). Set B
has 8 isolation series; the loaders now carry the declaration through.
Full-unison captures: 594 (A) and 464 (B).

### The measurement that decides it: do the strings' deviations reproduce?

The blend exists to let a key's measured B pull its widths away from the smooth
fit. That is worth having only if the pull is the *string*, not the capture. Two
independent sessions answer it directly — per-key median $\ln(B/B_\xi)$
residual in each, correlated across keys:

| register       | r (A vs B) | slope | median \|residual\|    |
| -------------- | ---------- | ----- | ---------------------- |
| bass (0–27)    | **+0.993** | 1.004 | 0.064 → **6.6 % in B** |
| mid (28–62)    | **+0.958** | 0.938 | 0.034 → 3.4 %          |
| treble (63–87) | +0.514     | 2.331 | 0.102 → 10.8 %         |

σ_p is 0.077 (A) and 0.074 (B) — the two sessions agree on the scatter as well
as on its per-key pattern. Keys 12/13 sit at +2.8/+3.2 σ_p in A and +3.0/+3.5 in
B; keys 30/31 at −3.4/−4.6 and −3.4/−4.4. **In bass and mid the deviations from
the fit are a reproducible property of the strings, not capture noise** — a
slope of 1.00 with r = 0.99 across sessions is as clean as this project's data
gets. The treble is the opposite (r = 0.51, slope 2.3), which is what σ_m(n)
already says; and it is precisely where the blend hands over to the model
(w < ½ above key 73), so the shrinkage is doing the right thing at both ends.

One caveat the correlation alone cannot dismiss: a *systematic* estimator error
— a partial mis-numbered the same way every time — would also reproduce across
sessions. Analysis 5 is what rules it out for the largest deviators here. The
reproducing bass/mid outliers (keys 28–31, −3.4 to −4.6 σ_p) are exactly the
below-fit zone Analysis 5 attributed to string design, on the cross-instrument
argument that a MAT bias would sit at the same keys on both pianos and does not.
Reproducibility across sessions plus non-reproducibility across instruments is
the pair of facts that makes it the strings.

**This is the answer to "the blend is ours and unexamined".** It is ours, and
what it carries in bass and mid is signal.

### What that is worth on the curve

| \|d_Model − d_Blend\| (¢), 24 resampled draws | bass        | mid         | treble      |
| --------------------------------------------- | ----------- | ----------- | ----------- |
| set A: median (max)                           | 0.14 (0.23) | 0.05 (0.24) | 0.07 (0.07) |
| set B: median (max)                           | 0.23 (0.56) | 0.03 (0.19) | 0.01 (0.02) |
| ratio to the same set's draw-to-draw SD       | 2.5–5.8×    | 1.0–2.7×    | 0.0–0.5×    |

Resampling (one capture per key, 24 draws) re-measures Analysis 4's noise floor
on each set instead of quoting it: SD of the (d)-Balanced curve is 0.020 ¢ (A)
and 0.135 ¢ (B) in the bass. So the pre-registered rule is met in the bass — the
difference is 2.5–5.8× the noise, real and not a resampling artifact — and it is
**not** met in the treble, where the difference is smaller than the noise and
has no stable sign. The earlier claim (from the instrument-#1 run) of a
systematic treble offset does not survive the better data: the treble effect is
draw noise.

**The chain, end to end.** A bass string sits ~6.6 % off the fit in B →
that changes one 2:1 octave row's beatless width by a median of **0.036 ¢**
(max 0.18) → the least-squares solution accumulates those small row differences
across the compass into a **0.14–0.23 ¢** curve difference → which is worth
**0.016–0.037 Hz** of 2:1 octave beat rate, against bass octaves that beat at
0.147–0.229 Hz under either curve. One extra beat every 27–60 seconds, on notes
that do not sound that long. For scale, the app's own unison panel cannot
resolve a bass string finer than ≈5–16 ¢ (report 0014); this choice is two orders
of magnitude under that.

### Outcome

The pre-registered rule says a bass difference above the noise floor is a
listening decision, and it is above it. But the size is now known: **0.2 ¢, or
1/30 Hz of beat rate**, and the two candidate curves differ by less in the bass
than two capture sessions of the same piano differ from each other in set B
(SD 0.135 ¢). Both sides of the original argument survive intact and neither is
refuted by a number:

- for `Model` — it is what Rigaud supports, and a single bad capture cannot
  reach the widths;
- for `Blend` — in bass and mid the per-key deviations are demonstrably the
  strings (r = 0.99 across sessions), and they are what an aural tuner is
  tuning.

n = 1 instrument cannot select between them (the two-instrument rule in
[`CONTRIBUTING.md`](../CONTRIBUTING.md)), and no metric here can:
the beat-rate table is engine (d)'s own objective and LKO's reference is the
chain's. The WAVs are `auralize_out/piano2{,_extended}/d_balanced.wav` against
`d_model-b.wav`. If `Model` ships it is a **product judgment** — that an
unexplained layer should not sit under the default curve — and it is recorded
as one, with the cost stated: it discards a reproducible 6.6 % per-key signal
to buy immunity to bad captures worth 0.2 ¢.

## Analysis 9 — where the analysis window starts (added 2026-09-02)

Recorded after acceptance; it moves no shipped value. The capture begins at the
gatekeeper's `Stable` verdict, ~116 ms after the onset, so the loudest part of
the note is never measured. The literature's reason for skipping it is that the
attack's frequencies are unsettled and its energy would drag the peak positions
the B fit reads. That had never been tested on our own dumps — and this report's
σ_lnB is exactly the yardstick the test needs, because the question is whether a
window shift moves B by more than two captures of the same key already differ.

**Build.** `validate_mat --offset-ms 0,116,300` re-measures each capture on a
32768-sample window cut at each offset from the **physical** onset — an RMS rule
independent of the gatekeeper, whose verdict is the quantity under test — with
the ET seed held across offsets so the comparison is paired within a capture.
595 piano-2 captures, none skipped. The window is half the shipped 65536 because
the full-event dumps hold only ~1.15 s after the pre-roll, so the attack's share
of it is twice production: conservative against admitting it.

**Result — the attack is inert.** Paired ΔB against the 116 ms reference, beside
the same set's repeat scatter recomputed on the same rows:

| register | ΔB at 0 ms (attack in) | ΔB at 300 ms       | repeat SD of ln B |
| -------- | ---------------------- | ------------------ | ----------------- |
| bass     | +0.11 % (IQR 0.46)     | +0.08 % (IQR 0.67) | **0.44 %**        |
| mid      | −0.04 % (IQR 0.95)     | −0.01 % (IQR 1.45) | **0.59 %**        |
| treble   | −2.48 % (IQR 25.7)     | +2.00 % (IQR 19.4) | **17.1 %**        |

In the bass the paired shift is *smaller* than the scatter between two captures
of the same key, which is what a null looks like once pairing has cancelled the
between-capture term. Mid is at parity. **Treble resolves nothing in either
direction**: its 17 % repeat scatter on a median of five located partials swamps
any offset effect, and the extreme tail — |ΔB| > 5 % on 62–67 % of treble
captures — is the index mis-numbering the MAT path review names, not window
position. The treble row is reported because omitting it would overstate the
result, not because it decides anything.

**What does move is the partial count.** Median located partials in the mid
register fall 18 → 17 → 15 across 0 → 116 → 300 ms. The argument against waiting
*longer* is that the note is decaying, not that the attack is dirty.

**Consequence: none, and that is the finding.** The shipped start is vindicated
for a different reason than the one on record. Admitting the attack does not
corrupt the fit at this window length; there is simply nothing to gain by moving
the start in either direction, and partials to lose by moving it late. A later
argument for starting the capture earlier — latency, say — has its bass and mid
answer here already; the treble question cannot even be asked until an estimator
with better repeat scatter exists there.

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
  confirmation on a *tuned* second instrument.

## Verification

61/61 lib tests (new: `test_curve_b_shrinkage` — σ-model shape, σ_p
self-calibration on on-model vs deviating profiles, blend flag semantics,
shrinkage-is-not-exclusion). Harness re-runs on both instruments recorded
above; `curve_analysis.png` + `curve_report.json` regenerated in
`diagnostics_piano_1/` and `diagnostics_piano_2/`
(`curve_compare --json` → `tuner-lab/scripts/plot_curves.py`). Clippy clean on all
touched code. Nothing committed (user decision pending).

## References

* Rigaud, David & Daudet 2013, JASA 133(5) — Eqs. 9, 29–31 (fit machinery),
  Eq. 30 (the inversion whose conditioning analysis 2 measures).
* Giordano 2015, JASA 138(4) — §VI.C gate, scan machinery (report 0008).
* Hastie, Tibshirani & Friedman, *ESL* 2nd ed. §7.10 — the 1-SE rule
  (report 0008; re-affirmed here).
* Inverse-variance weighting / conjugate-normal posterior mean — derived
  in Decision 1 (self-contained); the Efron–Morris/empirical-Bayes
  shrinkage family is the named lineage.
* MAD scale estimation, consistency constant 1.4826 = 1/Φ⁻¹(3/4) —
  derived from the normal quartile; standard robust-statistics form.
