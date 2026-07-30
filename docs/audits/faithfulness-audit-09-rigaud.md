# Faithfulness audit 09 — `algorithms/rigaud.rs` vs Rigaud 2013

**Series:** Prompt B′ wave-2 faithfulness audits (queue table in `README.md`;
running table in `faithfulness-audit-01-twm.md`), item 2 of 5.
**Date:** 2026-07-15.
**Source of truth:** Rigaud, F., David, B. & Daudet, L. (2013). "A parametric
model and estimation techniques for the inharmonicity and tuning of the
piano." JASA 133(5), 3107–3118 — primary source read in full
(`resources/moba/2013_a_parametric_model...pdf`). DAFx-11 precursor in-tree
(`resources/moba/53_e.pdf`) — not needed; the JASA paper is self-contained.
**Scope:** the whole module — `erf`, `S_T`/`Y_T`, `BXi`/`fit_b_xi`,
`RhoPhi`/`fit_rho_phi`, `invert_rho`, `f0_from_partials`, `midi_from_key`,
and the module-doc conventions. (This audits the *tuning-curve port* of the
paper; audit 06 — `faithfulness-audit-06-b-prior.md` — audited the separate
Discovery B prior `models::get_expected_beta` against the same paper's
Eqs 7–8.)

## Paper specification (as read)

- **Eq 1**: f_n = n·F₀·√(1+Bn²), F₀ the flexible-string fundamental; MIDI
  indexing m ∈ [21, 108] (§II.A).
- **Eqs 7–8**: log B_ξ has two linear asymptotes b_T(m) = s_T·m + y_T,
  b_B(m) = s_B·m + y_B; B_ξ(m) = e^{b_B(m)} + e^{b_T(m)}. Additivity is a
  smoothing convenience, not physics (§II.B.2.b).
- **§III.B.1**: (s_T, y_T) ≃ (9.26·10⁻², −13.64), an L1 regression over 6
  pianos in C4–C8, fixed universal thereafter (consistent with Young 1952's
  physics-based 9.44·10⁻² / −13.68).
- **Eq 29**: ξ̂ = argmin_ξ Σ_{m∈M} |log B(m) − log B_ξ(m)|.
- **Eq 9**: ρ_φ(m) = κ/2·(1 − erf((m−m₀)/α)) + 1, φ = {κ, m₀, α}, defined
  for m ∈ [21, 96]; bass asymptote κ+1, treble asymptote 1 (first-partial
  pitch perception above F6/1400 Hz — §II.B.3).
- **Eq 30**: ρ(m) = √((4F₀(m)² − F₀(m+12)²)/(F₀(m+12)²B(m+12) −
  16F₀(m)²B(m))); missing when the radicand is negative — the compressed
  octave case (§IV.C.1, closing paragraph).
- **Eq 31**: φ̂ = argmin_φ Σ_{m∈M} |ρ(m) − ρ_φ(m)|.
- **Eq 20**: F₀ = [Σ f_n·n·√(1+Bn²)]/[Σ n²(1+Bn²)] — the exact stationarity
  solution ∂C₁/∂F₀ = 0 of the Eq-16 constraint least squares (§III.A.3.a).
- **§III.A.3.b (Initialization)**: typical values s_B = 8.9·10⁻² [sic],
  y_B = −7; κ = 3.5, α = 25, m₀ = 60, d_g = 0.

## Verdict summary

| # | Item | Classification |
| --- | ---- | -------------- |
| 1 | `BXi::b_at_midi` — Eqs 7–8 form | (a) faithful, exact |
| 2 | `S_T`/`Y_T` constants | (a) exactly the paper's §III.B.1 universal fit |
| 3 | `fit_b_xi` objective — Eq 29 | (a) faithful (ln vs unspecified log: argmin-invariant) |
| 4 | `fit_b_xi` optimizer + domain | (b) OURS, documented (coarse-to-fine grid; paper's is NMF-embedded) |
| 5 | `fit_b_xi` over all trusted keys | (b) OURS, documented (paper's Eq-29 M is unrestricted; its demo uses 4 bass notes, §IV.C.1) |
| 6 | `RhoPhi::rho_at_midi` — Eq 9 | (a) faithful, exact |
| 7 | `RhoPhi::TYPICAL` constants | (a) the paper's §III.A.3.b init values — was mis-cited "§III.B.3" → **FIXED** |
| 8 | `invert_rho` — Eq 30 + missing-data case | (a) faithful, exact (`den == 0`/non-finite guard ours, inert) |
| 9 | `fit_rho_phi` L1 term — Eq 31 | (a) faithful |
| 10 | `fit_rho_phi` quadratic prior penalty | (b) OURS, documented (reg_weight = 0 recovers pure Eq 31; weight by LOO-CV+1-SE per ADR 0008) |
| 11 | `RHO_FIT_KAPPA_MAX` = 6, search domains | (b) OURS, documented (brackets Fig. 9's data range) |
| 12 | `f0_from_partials` — Eq 20 | (a) faithful, exact (re-derived: ∂C₁/∂F₀ = 0 ⇒ the code's num/den) |
| 13 | `erf` — A&S 7.1.26 | (a) faithful (all six constants verified; Horner form correct; odd extension valid) |
| 14 | `midi_from_key` / index convention | (a) faithful (m = key + 21; §II.A) |
| 15 | Paper's printed s_B = +8.9·10⁻² init | paper typo, silently corrected in our doc → now **explicitly noted** in-code |

## Findings

**1–2. The B_ξ model and the universal treble pair are exact.**
`b_at_midi` = e^{s_B·m+y_B} + e^{s_T·m+y_T} is Eq 8 verbatim; S_T = 9.26e-2,
Y_T = −13.64 are the paper's §III.B.1 values to all printed digits.
`BXi::DEFAULT_MEDIUM` (−0.066, −7.891) re-derives `get_expected_beta`'s
1-indexed pair in MIDI domain exactly (−0.066·(m−20) − 9.211 =
−0.066·m − 7.891) — consistent with audit 06.

**3–5. Eq 29: objective faithful; optimizer and point-set ours,
documented.** The code minimizes Σ|ln B − ln B_ξ| — the paper's "log" is
base-unspecified; any base rescales the objective by a positive constant, so
the argmin is identical (recorded here, no code change needed). The
deterministic coarse-to-fine grid replaces the paper's NMF-embedded
estimation — already labeled "the *objective* is the paper's; the
*optimizer* is ours" with the search domain documented. The paper's own
demonstration estimates ξ from 4 bass-range notes (§IV.C.1); we fit over
all trusted keys — a documented choice consistent with Eq 29 (M is
unrestricted), justified in-code (L1 + the fixed treble asymptote make
treble points near-inert for ξ; the bass-pair gradient argument checks out:
e^{b_B(m)} is negligible against e^{b_T(m)} in the treble).

**6–7. Eq 9 exact; one mis-citation FIXED.** `rho_at_midi` is Eq 9
verbatim, asymptotes and the m ∈ [21, 96] domain note faithful (§II.B.3's
F6/1400 Hz first-partial rationale correctly summarized). `RhoPhi::TYPICAL`
(κ = 3.5, m₀ = 60, α = 25) had cited "§III.B.3" — that section is d_g
estimation (Eq 32); the values actually live in **§III.A.3.b
(Initialization)**. Comment fixed. Values themselves verified exact.

**8. Eq 30 exact.** num = 4F₀(m)² − F₀(m+12)², den = F₀(m+12)²B(m+12) −
16F₀(m)²B(m), ρ = √(num/den) — verbatim. `None` on a non-positive radicand
is the paper's own missing-data case, correctly cited to §IV.C.1 (verified:
the compressed-octave sentence closes that subsection). The `den == 0` and
non-finite guards are ours and inert (they map the same degenerate
geometry to the same `None`). The round-trip test constructs the octave
via Eq 6 and inverts — the Eq-6 expression in the test is verbatim
(2F₀(m)·√((1+B(m)·4ρ²)/(1+B(m+12)·ρ²))).

**9–11. Eq 31 faithful; the prior penalty is ours and says so.** The L1
term is Eq 31 verbatim; the quadratic penalty toward a prior φ is labeled
**ours** in-code with its rationale (design note §6(c) "strong
regularization"), its unit convention documented, and `reg_weight = 0`
recovering the paper's pure Eq 31 (the recovery test runs at 0). Weight
selection is LOO-CV + 1-SE per ADR 0008 (caller-side; see audit 12).
`RHO_FIT_KAPPA_MAX = 6` is marked ours and brackets Fig. 9's observed ρ
range (data reach ρ ≈ 5.5–6 at m = 21, i.e. κ ≈ 5).

**12. Eq 20 re-derived and exact.** From Eq 16,
C₁ = Σ(f_n − nF₀√(1+Bn²))²: ∂C₁/∂F₀ = 0 ⇒ F₀·Σn²(1+Bn²) = Σf_n·n·√(1+Bn²)
— the code's num/den precisely. The `n == 0`/NaN/non-positive-f skip and
the positive-denominator check are inert input guards (ours). The paper's
reliable-set refinement Δ_r (§III.A.3.c) is an NMF-loop concern; our caller
supplies already-vetted partials — no claim of it is made, correctly.

**13. erf is A&S 7.1.26 exactly.** p = 0.3275911, a₁ = 0.254829592,
a₂ = −0.284496736, a₃ = 1.421413741, a₄ = −1.453152027, a₅ = 1.061405429 —
all six constants verified against the handbook formula; the Horner
evaluation expands to a₁t + … + a₅t⁵; odd extension is valid (7.1.26 is
stated for x ≥ 0, erf is odd). The |error| ≤ 1.5·10⁻⁷ bound is the
handbook's own. Test table values verified against standard erf to 9
digits.

**15. Paper typo, now explicitly noted.** §III.A.3.b prints the bass-slope
initialization as s_B = 8.9·10⁻² (positive). The bass asymptote of log B_ξ
falls with m (Fig. 1(a); B increases toward A0, §II.B.2.a), so the sign is
a typo for −8.9·10⁻². Our doc-comment used (−0.089, −7) silently; a
one-line note now records the typo so nobody "fixes" the sign back.

## Fixes applied (2026-07-15, same session)

1. `RhoPhi::TYPICAL` doc: "§III.B.3 / Algorithm 1" → "§III.A.3.b
   Initialization, cf. Algorithm 1".
2. `fit_b_xi` doc: paper-typo note added for the printed +8.9·10⁻² bass
   slope.

Both comment-only; no behavior change (see wave-2 verification in audit 12 —
tests + `curve_compare` byte-identical run once for the whole wave).

## Audit series status

Wave-2 item 2 complete. Queue: `docs/audits/README.md`. Next: item 3,
`algorithms/giordano.rs` vs Giordano 2015 / Sethares 1993 / Plomp–Levelt
1965 (PDFs in `resources/worker/` + `resources/curve/`).
