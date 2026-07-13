# ADR 0008 — Giordano-Layer Fidelity and Derived Interval Weights (Review Sets 2–3)

## Status

**Accepted (2026-07-10).** Covers Sets 2 and 3 of the three-set
post-implementation review of the Prompt-F tuning-curve engines
(`docs/design/tuning-curve-design.md` is the governing spec; Set 1 is
ADR 0007). Everything below was measured on the 87 regenerated captures —
**diagnostics, not selection evidence; n = 1.** Attribution discipline: one
harness re-run per set; each set moved exactly the engines it touches and no
others ((a)/(b)/(d) byte-identical under Set 2, (a)/(b)/(c) under Set 3).

## Context

The Prompt-F scrutiny left two clusters of issues inside the parts of the
construction that are ours:

* **Set 2 — the (c) calibration path**: the octave scan's fixed window was
  centred on the *current mistuned* interval (exclusions depended on how far
  out of tune the capture was, and `OctaveScan`'s doc claimed offsets were
  "from the pure ET octave" — false); the sufficiency gate counted
  above-median-amplitude cross pairs, which passes 7×7-partial pairs
  Giordano's own convergence analysis rejects; and the Eq.-9 refit used
  `RHO_FIT_REG_WEIGHT = 1.0`, an unsourced constant that never realized the
  design note's "strong regularization".
* **Set 3 — engine (d) weights**: the BALANCED preset's equal type-weights
  averaged the 2:1 family (~2.6 ¢ implied stretch) against 6:3 (~23 ¢) with
  no register awareness — bass stretch 4.4 ¢/oct against ~22–24 data-implied.

## Set 2, Decision 1 — Coincidence-bracket scan window

`dissonance::octave_scan` now scans the **interval-width axis** over
[min, max] of the pair's beatless widths — the widths at which the
coincident pairs 2j:j present in both measured partial lists beat out
(pair 2j:j is beatless exactly at the Eq.-6 width with ρ = j;
`interval_width_cents(b_l, b_u, 2j, j, 12)`) — extended by
`SCAN_MARGIN_CENTS = 10 ¢` on each side, the window's one remaining soft
constant (headroom for non-coincident cross terms and raw-B scatter).
Widths are computed from measured B, so the window is
**mistuning-independent** (tested: `test_scan_mistuning_independent`) and
register-adaptive by construction — tight around the 2:1 width in the
pair-starved treble, wide enough for high octave types in the bass.

The largest admitted pair is j ≤ 7, **derived** from the downstream
consumer's own domain: the scan exists to produce ρ points for the Eq.-9
fit, whose κ search bracket (`RHO_FIT_KAPPA_MAX = 6`) can express octave
types up to ρ = κ + 1 = 7. Beyond that the beatless widths of deep-bass
pairs (j ≈ 15 exists at 30 partials) exceed +400 ¢ — out where the
Plomp–Levelt terms decouple (d₂ → 0 for wide separation), a regime whose
shallow minima are interval-identity artifacts, not octave optima.

Edge semantics: a low-edge hit is compression evidence (an optimum at or
below the ρ = 1 floor, impossible for a real optimum under the §2 theorem);
a high-edge hit means the well lies past every admissible octave type. Both
are excluded from the ρ fit, as before. The `OctaveScan` doc bug is fixed
(offsets are relative to the **measured** upper note).

## Set 2, Decision 2 — §VI.C sufficiency gate

**Verified against the PDF** (`resources/worker/2359_1_online.pdf`,
§VI.C): *"when computing the dissonance of the note pairs A0–A1, and
A1–A2, it is necessary to include at least 16 partials of the lower note
and 8 of the higher member of the pair to reach the asymptotic result …
Including fewer partials gives a significantly smaller predicted stretch.
By comparison, for the note pair A2–A3 the asymptotic value for the
stretch is reached with only six partials from the lower note and three
from the upper one."*

16 lower / 8 upper ⇔ min(⌊16/2⌋, 8) = **8 coincident 2j:j pairs** — the new
gate (`dissonance::coincident_pairs ≥ GIORDANO_MIN_COINCIDENT_PAIRS`). The
paper's A2–A3 case (6/3 = 3 pairs) shows 8 is the bass-derived
*conservative floor*, adopted compass-wide: mid captures carry ≥ ~14
partials so the stricter bound costs nothing there, and it correctly
starves the 3–6-partial treble. The old above-median-amplitude product
(`strong_cross_pairs`) is demoted to a reported diagnostic. On the real
captures: 42/74 pairs pass the §VI.C gate where the old gate passed 53 —
the 11 newly excluded are exactly the pairs whose scan optimum §VI.C says
is biased low.

## Set 2, Decision 3 — ρ-fit regularization by LOO-CV with the 1-SE rule

`RHO_FIT_REG_WEIGHT = 1.0` is deleted. `select_rho_reg_weight` picks the
Eq.-9 regularization weight by leave-one-out CV over the ρ points (drop one
octave pair, refit, absolute prediction error) on a decade grid
(10⁻²…10², `RHO_REG_GRID_DECADES`), then applies the **one-standard-error
rule**: the *largest* weight whose mean error is within one SE of the
minimum (SE = std of per-point LOO errors / √n).

The 1-SE rule is load-bearing, not decoration. Measured on the real
captures (probe, deleted after this record):

* The 34 accepted ρ points span ρ = 1.0–7.8 with neighbour jumps of 4+
  ρ-units; mean LOO error is 1.19–1.43 ρ-units **at every weight** (SE
  ≈ 0.23) — the CV curve is flat at noise level across four decades.
* The last ρ point sits at key 44: the whole upper half of the compass is
  extrapolation the CV never scores.
* The bare argmin picked the weak grid edge (w = 0.01, 2.5 % "better" than
  w = 1) whose fit φ = (κ 2.548, m₀ 75.7, α 75.2) never finishes its
  descent — ρ(A7) = 1.74, driving (c) to A7 +47.2 / C8 +60.5 ¢. That is
  the same defect class as ADR 0007's boundary reversion: an un-scored,
  data-free region's behavior decided by the objective's null direction —
  and it contradicts the spec (§6(c): the starved treble "rides the ρ → 1
  asymptote").
* The 1-SE rule (verified: Hastie–Tibshirani–Friedman, *ESL* 2nd ed.
  §7.10 — "choose the most parsimonious model whose error is no more than
  one standard error above the error of the best model"; corroborating
  secondary sources checked 2026-07-10, same verification status as
  ADR 0007's Eilers citation) selects w = 10: φ = (κ 3.146, m₀ 57.5,
  α 27.2), ρ(A7) = 1.02 — the treble rides the asymptote, and the bass
  keeps a data-driven moderation of the prior (ρ(A0) 4.06 vs typical
  4.45). Ties toward the prior break exactly when — and only when — the
  data cannot distinguish, which is the note's "strong regularization"
  intent made precise.

### Set 2 consequences (harness, 87 captures; engine (c) only — (a)/(b)/(d) byte-identical)

| metric | pre-Set-2 | post-Set-2 |
| --- | --- | --- |
| A0 / A1 (¢) | −39.7 / −21.3 | −60.1 / −30.2 |
| A7 / C8 (¢) | +32.6 / +41.5 | **+28.1 / +36.3** (prior: +29.3 / +37.4) |
| stretch bass/mid/treble (¢/oct) | 11.61 / 3.37 / 17.32 | 17.60 / 3.79 / 14.84 |
| roughness med/max (¢) | 0.084 / 0.296 | 0.114 / 0.452 |
| Giordano-excluded keys | 34 | 40 |
| accepted ρ points | 40 | 34 |
| Giordano cross-score | 4.0997 | 4.1792 |
| **LKO bass / mid (¢)** | 12.68 / 1.63 | **4.46 / 1.54** |

The leave-key-out bass error collapsing 12.68 → 4.46 ¢ is the headline: the
calibrated curve now predicts held-out raw data 3× better in the bass —
the largest LKO improvement any engine has shown. The treble rides the
prior again. Cross-score worsens slightly (fewer, cleaner points + stronger
shrinkage); descriptive only.

## Set 3 — Engine (d) Form-2 derived weights

### Decision 4 — the derived weight

`dissonance::pair_width_sensitivity` implements the design note's bridge
Form 2 as a derivation: near coincidence, the pair's roughness term is
a_p·a_q·d₂(Δf) with d₂ ≈ (b₂ − b₁)·s·Δf (linear at the origin), and one
cent of width error separates the pair by Δf ≈ f̄·ln2/1200, so

    ∂D/∂ε = a_p·a_q·(b₂ − b₁)·s(f̄)·f̄·ln 2/1200,  s(f̄) = x*/(s₁·f̄ + s₂)

— every symbol a published Giordano/Sethares constant, zero new free
parameters (tested against a finite difference of the actual d₂ across
registers, incl. the tempered ~2 ¢ operating points). W_{m,k} = preset
multiplier × sensitivity; a data row exists only where both endpoints
carry measured curve-B **and both coincident partials are measured** (row
absent otherwise — this also subsumes the former ET-based Nyquist check),
with a_p/a_q under Giordano's equal-total-power normalization and f̄ the
pair's mean measured frequency. Weights are normalized to unit mean — a
gauge (a common scale is absorbed into λ), not a parameter. Style presets
survive only as taste multipliers.

### Decision 5 — λ selection: GCV retired for weighted fast-LOO + 1-SE

The first Set-3 run exposed a real failure: GCV picked λ ≈ 10⁻² and the
curve kinked 26–31 ¢ (|Δ²d|) at keys 8–10. Mechanism (probe, deleted
after this record): the derived weights span ~2 orders of magnitude
(2:1 rows: bass median 5.1·10⁻⁵ vs mid 2.7·10⁻³), and GCV's
equal-variance premise breaks — near-weightless rows still count fully in
N, deflating RSS/N, while the dominant self-consistent mid rows drag λ
down; the mutually-conflicting deep-bass rows are then left unsmoothed. A
manual λ sweep shows a sane plateau at λ ∈ [10², 10⁴] (max |Δ²d| 0.52).

Fix, same doctrine as Decision 3: per-row **fast LOO** scores
w_i·(r_i/(1 − h_ii))² (the penalized-WLS identity, the same Eilers Eq.-10
form `smoothing::whittaker_cv` already uses; h_ii = w_i·aᵢᵀA⁻¹aᵢ from the
banded Cholesky) with the **1-SE rule** toward larger λ. The selection
lands on the plateau (~10⁴). `Banded::column` (GCV's edf helper) is
removed with its only caller.

### Set 3 consequences (harness, 87 captures; engines (d) only)

| metric | pre-Set-3 | post-Set-3 |
| --- | --- | --- |
| BALANCED A0 / A1 (¢) | −18.3 / −9.9 | −49.3 / −10.4 |
| BALANCED bass / mid stretch (¢/oct) | 4.41 / 2.55 | 5.89 / 2.44 |
| BALANCED roughness med/max (¢) | 0.054 / 0.480 | 0.038 / 0.525 |
| BALANCED cross-score | 4.0795 | **4.0350** (best of all engines) |
| BALANCED LKO bass / mid (¢) | 23.60 / 1.84 | 20.85 / 2.28 |
| pure-12ths roughness med/max (¢) | 0.159 / 4.344 | 0.039 / **0.518** |

**The bass gap is genuinely explained, not resolved** — the handoff's
anticipated alternative branch. Within the deep bass the expected
reordering did happen (6:3 sensitivity 2.5·10⁻⁴ ≈ 5× the 2:1's
5.1·10⁻⁵ — the wide-stretch pair dominates *locally*), but the
cross-register disparity dominates it: deep-bass rows carry ~50× less
perceptual weight than upper-mid rows, because s(f̄)·f̄ falls toward low
frequency (a cent is fewer Hz, and the roughness slope per Hz is
shallower) and 30-partial spectra dilute any single pair's normalized
amplitude product. Under Giordano's own functional, a cent of deep-bass
octave error is simply cheap. No λ produces the chain-implied ~22–24 ¢/oct
from interval evidence (λ → ∞ recovers the prior's 18.3 via the reversion
term). Consequently engine (d)'s deep bass is prior/smoothness-driven —
A0 −49.3 is the smooth continuation between weak bass evidence and the
prior — and demanding the full measured bass stretch remains the job of
the octave-chain layers (engines (b)/(c)). The pure-12ths preset's old
4.3 ¢ kink also resolves under the new selection.

## Verification

* Giordano §VI.C quoted from the PDF this session (gate + shift rule +
  equal-total-power all confirmed). ESL §7.10 1-SE rule verified via
  secondary sources (primary is a textbook; same status as ADR 0007's
  Eilers item). Form-2 sensitivity verified against finite differences of
  the implemented d₂ (`test_pair_width_sensitivity_matches_finite_difference`).
* New/updated tests: `test_scan_mistuning_independent` (the bracket's
  defining property), `test_scan_requires_coincident_pair`,
  `test_sufficiency_gate_starves_treble` (now exact pair counts),
  `test_pair_width_sensitivity_matches_finite_difference`. Suite: 60/60.
* Harness before/after per set recorded above (`examples/curve_compare.rs`,
  which now also reports the calibration stage: accepted ρ points,
  CV-selected reg weight, calibrated φ, and both gates' pass counts).
* Probes (`rho_probe.rs`, `d_probe.rs`) deleted after this record, per the
  ADR 0007 precedent.

## References

* N. Giordano, JASA 138(4):2359–2366 (2015), §VI.C.
  `resources/worker/2359_1_online.pdf`.
* Rigaud, David & Daudet 2013, JASA 133(5) — Eqs. 6, 9, 29–31; §IV.C.
  `resources/moba/`.
* W. A. Sethares, JASA 94(3) (1993) — d₂ parametrization (constants in
  `dissonance.rs`).
* Hastie, Tibshirani & Friedman, *The Elements of Statistical Learning*,
  2nd ed., §7.10 — the one-standard-error rule.
* P. H. C. Eilers, Anal. Chem. 75(14) (2003) — fast-LOO identity.
* Golub, Heath & Wahba, Technometrics 21 (1979) — GCV (retired here for
  the derived-weight case; rationale in Decision 5).
* Design note: `docs/design/tuning-curve-design.md` §§2, 3.2, 6, 13.
* ADR 0007 — Set 1; the reversion and gauge mechanics Set 3 interacts with.
