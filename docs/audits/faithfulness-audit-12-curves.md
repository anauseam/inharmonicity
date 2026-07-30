# Faithfulness audit 12 — `algorithms/curves.rs` (assembly audit)

**Series:** Prompt B′ wave-2 faithfulness audits (queue table in `README.md`),
item 5 of 5 — **wave complete**.
**Date:** 2026-07-15.
**Lens:** engines (b)–(d) are *declared assemblies* ("componentwise faithful,
assembly = industry practice"), so the faithful-port bar applies to (i) every
Eq/§ citation, (ii) every OURS component's derivation record, (iii) the
numeric literals — not to the compositions themselves. Sources: Rigaud 2013
and Giordano 2015 (read in full for audits 09/10), Eilers 2003 (audit 11),
`docs/design/tuning-curve-design.md` (the governing spec), ADRs 0007–0009.

## Verdict summary

| # | Item | Classification |
| --- | ---- | -------------- |
| 1 | `octave_stretch_cents` — Eq 6 + F₀→f₁ conversion | (a) re-derived, exact |
| 2 | `interval_width_cents` — c_{m,k} + conversion | (a) re-derived, exact (2:1/k=12 ⇔ Eq 6 at ρ=1, tested) |
| 3 | `StretchPreset` — §IV.C.2 ±1 presets | (a) faithful; the paper's `min(ρ̄−1, 1)` read as a typo for the floor — **verified against Fig 9** (the low curve rides ρ=1 in the treble; `min` would sink below the 2:1 asymptote) |
| 4 | `a_chain_prior` — §II.B procedure | (a) faithful (A4 anchor via §1 conversion; Eq-6 chain up + inverted down; ρ indexed by the *lower* note per the paper's Eq-9 note; Eq-4 deviation; §II.B.4 Lagrange over the A notes) |
| 5 | d_g | (b) carried as a user control ("Eq. 32's role"); the paper's Eq-32 *estimation* of d_g is deliberately not implemented — consistent with §II.B.5's purpose (reference-fork choice) |
| 6 | ln-B shrinkage block (`SIGMA_*`, `sigma_prior`, blend) | (b) OURS/measured, ADR 0009 — conjugate-normal posterior-mean form re-checked; 1.4826 = normal-consistent MAD scale, documented |
| 7 | `REVERSION_LENGTH_KEYS` + `reversion_weight` | (b) OURS/derived, ADR 0007 — Euler–Lagrange re-derived: w₀z + λz⁗ = 0 ⇒ decay rate (w₀/λ)^¼/√2 ⇒ w₀ = 4λ/ℓ⁴ exactly |
| 8 | §2 detector (pre-exclusion + `finish` flags) | (b) design-note §2, flag-and-exclude semantics verified (never clamps) |
| 9 | Minimum-norm chain gauge | (b) OURS/derived, ADR 0007 (mean-centering over measured keys; tested) |
| 10 | `GIORDANO_MIN_COINCIDENT_PAIRS`, `RHO_FIT_MIN_POINTS`, `RHO_REG_GRID_DECADES` | (b) documented (§VI.C-derived / ours-labeled / grid rationale) |
| 11 | `select_rho_reg_weight` — LOO + 1-SE | (b) OURS, ESL §7.10 cited, ADR 0008; SE = sd/√n standard |
| 12 | Engine (d) — J(x), τ_k, Form-2 weights, unit-mean gauge, pseudo-rows, row-LOO + 1-SE | (b) documented assembly; leverage h = w·aᵀA⁻¹a and the row-deletion identity are the same Sherman–Morrison result verified in audit 11; GCV retirement rationale recorded with the Golub–Heath–Wahba citation |
| 13 | "Eilers Eq. 10" at 2 sites | (c) **mis-citation → FIXED** (→ Eqs 10–11, matching audit 11) |
| 14 | Stale "which GCV re-selects" comment | (c) **stale doc → FIXED** (GCV retired in ADR 0008; → "the CV") |
| 15 | Numeric-literal sweep | clean — every constant is a paper value, an ADR-linked measurement/derivation, a documented gauge, or an inert guard |

## Findings

**1–2. Both width primitives re-derived.** Eq 6's F₀ ratio times the
√((1+B_U)/(1+B_L)) audible-conversion factor is exactly the code's r₁; the
p:q beatless width comes from q·F₀U√(1+B_Uq²) = p·F₀L√(1+B_Lp²) plus the
same conversion — both match the code symbol-for-symbol, and the tested
identity (2:1, k=12 ⇔ Eq 6 at ρ=1) pins the conversion consistency.

**3. The §IV.C.2 "min" typo call is right.** The paper prints
ρ̄_φ,L = min(ρ̄_φ(m)−1, 1); taken literally the low preset would drop to
ρ ≈ 0 in the treble (below the physical 2:1 asymptote the paper itself
sets, §II.B.3). Fig 9's low dashed curve rides at ρ = 1 through the treble
— the floor, i.e. max. The code's `(rho − 1).max(1.0)` with its in-code
typo note is the correct, documented reading.

**4. The §II.B chain is faithful in the details that bite.** A4 anchored
as f₁ = 440 (F₀ = 440/√(1+B) — the §1 convention applied, not skipped);
the down-chain is Eq 6 *inverted* with the reference/tune roles swapped,
exactly the paper's §II.B.1 prescription; ρ is indexed by the pair's
**lower** note (the paper's explicit note under Eq 9 — "ρ_φ is indexed by
the note m, and not by the note m+12"), which the code honors in both
chain directions; d(m) = 1200·log₂(f₁/F₀,ET) is Eq 4. Lagrange over the
tuned A notes is §II.B.4 verbatim.

**6–7. The two load-bearing OURS derivations re-checked.** (i) The
shrinkage is the textbook conjugate-normal posterior mean in ln B; the σ_m
power law and σ_p self-calibration carry their ADR 0009 provenance
in-code, including the honest sensitivity note (w insensitive to 2× σ_m
errors away from the crossover). (ii) The reversion weight: the tail
functional ∫ w₀z² + λ(z″)² has Euler–Lagrange w₀z + λz⁗ = 0,
characteristic roots r⁴ = −w₀/λ, slowest decaying real part
(w₀/λ)^¼·cos 45°, so ℓ = √2(λ/w₀)^¼ ⇔ w₀ = 4λ/ℓ⁴ — exactly the code. ℓ =
12's justification (B_ξ e-folding lengths 1/s_T ≈ 10.8, 1/|s_B| ≈ 12–15)
is consistent with the audited §III.B.1 constants.

**12. Engine (d)'s CV is the audited identity.** The per-row leverage
h = w·aᵀA⁻¹a with score w·(r/(1−h))² is the same rank-one-deletion result
proven exact for general weights in audit 11 (Sherman–Morrison), applied
to rows instead of points. The 1-SE rule and the GCV retirement carry
their ADR 0008 record; pseudo-rows shape the system and are excluded from
scores (prior, not data) — verified in code.

**13–14. Three comment defects fixed.** Two "Eilers Eq. 10" citations for
the fast-LOO form (the identity is Eq 11 — the same off-by-one found and
fixed in `whittaker.rs`, audit 11), and one stale "which GCV re-selects"
in the unit-mean gauge comment (GCV was retired for row-LOO + 1-SE in ADR
0008; the gauge argument itself survives unchanged under the CV).

**15. Constants sweep.** SIGMA_LNB_COEFF/FLOOR, SIGMA_PRIOR_* (ADR 0009,
measured, labeled); 1.4826 (MAD consistency, documented);
NEGATIVE_STRETCH_TOL_CENTS = 0.01 ¢ (FP-noise guard, documented);
REVERSION_LENGTH_KEYS = 12 (derived, ADR 0007); GIORDANO_MIN = 8 (§VI.C,
audit 10); RHO_FIT_MIN_POINTS = 6 (ours, labeled); RHO_REG_GRID (rationale
documented); preset interval weights (taste multipliers, explicitly "never
silent magic numbers" — the register balance comes from the derived
Form-2 sensitivities per ADR 0008); 440.0/1200/100 (definitions); 1e-12
(guard). Nothing unlabeled.

## Fixes applied (2026-07-15, same session)

1. Two Eilers "Eq. 10" → "Eqs. 10/11" citations (`multi_interval` doc +
   the row-LOO comment).
2. "which GCV re-selects" → "which the CV re-selects" (stale after ADR
   0008).

Comment-only. **Wave-2 verification:** all changes across audits 5′/09–12
are comments plus one test addition; no code expression changed, so engine
output is unchanged by construction. Full `tuner-core` lib test suite +
clippy run at wave close (see the wave-2 summary in `README.md`).

## Wave-2 wrap-up (tally)

- **Mis-citations fixed: 6** — RhoPhi::TYPICAL §III.B.3→§III.A.3.b (09);
  Giordano "Eqs 4–6"→"3–4" ×2 (10); Eilers Eq-10→Eq-11 ×1 in whittaker +
  ×2 in curves (11/12).
- **Reclassified toward faithful: 1** — `giordano::dissonance` cross-pairs-
  only is the paper's own Eq 5, not our deviation (10).
- **Undocumented deviations documented: 2** — Giordano's ½ prefactor
  dropped (inert scale, 10); w-weighted CV score vs Eilers' unweighted Eq 9
  (11, with the general-W exactness derived and test-pinned).
- **Paper errata recorded: 3** — Rigaud §III.A.3.b s_B sign typo (09);
  Rigaud §IV.C.2 `min` typo (12, confirmed via Fig 9); Hurley & Rickard
  Thm A.4/Table III ℓ²/ℓ¹-D3 internal inconsistency (audit-05 addendum,
  with our derivation).
- **Stale docs fixed: 1** — "GCV re-selects" (12).
- **Tests added: 1** — heterogeneous-weights LOO-identity brute-force
  check (11).
- **Core math: zero defects found** — every equation implementation
  checked (Rigaud Eqs 4/6/7–9/20/29–31, Giordano Eqs 3–6 + §VI.C, Eilers
  Eqs 5/7/8/10/11, A&S 7.1.26) verified exact.
