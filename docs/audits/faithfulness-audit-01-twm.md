# Faithfulness audit 01 — `twm.rs::score_candidate` vs Maher & Beauchamp 1994

**Series:** Prompt B faithfulness audits (running status table in this doc, below), item 1 of 8.
**Date:** 2026-07-02.
**Source of truth:** the actual paper PDF, `resources/engine/JASA.04.94.pdf` —
Maher, R.C. & Beauchamp, J.W. (1994), "Fundamental frequency estimation of
musical signals using a two-way mismatch procedure," JASA 95(4), 2254–2263.
**Scope:** `tuner-core/src/algorithms/twm.rs` (`score_candidate` + `TwmConfig`).
Upstream peak extraction (paper §II: STFT, parabolic interpolation) is items
2–4 of the series; the Stage-A/B search in `discovery.rs` is covered only where
it maps onto the paper's search strategy. **No behavior was changed in this
audit** — the doc-comment fixes were applied same day (see "Follow-up
resolution" at the end; goldens verified byte-identical).

## Paper specification (extracted for the record)

Steps 1–6 and Eqs. (1)–(3), §II.A:

1. K measured partials (Aₖ, fₖ); A_max = max(Aₖ); f_max = max(fₖ).
2. Trial f_fund → N harmonics fₙ = n·f_fund, N = ⌈f_max/f_fund⌉ ("the smallest
   integer greater than f_max/f_fund").
3. For each fₙ: nearest measured partial, Δfₙ = |fₙ − fₖ|, aₙ = Aₖ of that peak.
4. Eq (1): Err_{p→m} = Σₙ [Δfₙ·fₙ⁻ᵖ + (aₙ/A_max)(q·Δfₙ·fₙ⁻ᵖ − r)].
5. For each fₖ: nearest predicted harmonic, Δfₖ = |fₙ − fₖ|. (Textual wrinkle:
   Step 5 says "set aₖ = Aₙ", but predicted harmonics carry no amplitude; Eq (2)'s
   signature E_w(Δfₖ, fₖ, aₖ, A_max) governs — aₖ is the measured peak's own
   amplitude. Standard implementations read it the same way.)
6. Eq (2): Err_{m→p} = Σₖ [Δfₖ·fₖ⁻ᵖ + (aₖ/A_max)(q·Δfₖ·fₖ⁻ᵖ − r)].
   Eq (3): Err_total = Err_{p→m}/N + ρ·Err_{m→p}/K.

Empirical constants: p=0.5, q=1.4, r=0.5, ρ=0.33 — the paper presents these as
"coefficients which we have determined empirically", and its user-parameter
list (§II end) explicitly invites adjusting p per signal class and N (they used
8≤N≤10 in practice). Negative error totals are canonical (the paper's own worked
example has Err_{m→p} = −3.0). Search strategy: semitone grid (×1.05946) over a
bounded F₀ range, then local refinement with shrinking step around each local
minimum.

## Verdict summary

The core scoring math is a **faithful port**: every term of Eqs (1)–(3), the
nearest-neighbor association, the amplitude selection in both directions, the
frequency-weight bases (predicted fₙ in Eq 1, measured fₖ in Eq 2), the /N and
/K normalizations, and the sign behavior (negative totals allowed) all match
the paper. The headline correction runs the *other way*: the **Dynamic
Bandwidth Cap, previously believed bespoke ("ours"), is in substance the
paper's Step 2** and should be re-documented as faithful. The genuine
undocumented deviations are three small numerical guards plus three doc-comment
inaccuracies — nothing that changes scoring on real inputs.

| # | Item | Classification |
| --- | ---- | -------------- |
| 1 | Eq (1)/(2)/(3) term structure, weights, normalization | (a) faithful |
| 2 | Two-pointer nearest-match association | (a) faithful (equivalent algorithm) |
| 3 | Dynamic Bandwidth Cap (cutoff = max_obs + f₀·scale) | (a) faithful — **reclassified**: exact Step 2 at B=0; conservative generalization at B>0 (≤1 edge partial) |
| 4 | Inharmonic predicted series (stiff-string KeyProfile) | (b) deliberate, documented |
| 5 | 88-key candidates + scale as the trial variable | (b) deliberate, documented |
| 6 | λ ceiling on per-peak M→P term (Duan-inspired) | (b) deliberate, documented (wording nit) |
| 7 | Retuned default constants q=3.88, r=1.426, ρ=0.298 | (b) deliberate, documented (within-paper tunables) |
| 8 | Four experimental config fields, shipped OFF | (b) documented bespoke experiments (prune candidates) |
| 9 | `f.max(1.0)` frequency-weight floor | (c) undocumented numerical guard |
| 10 | `a_max.max(1e-6)` floor | (c) undocumented numerical guard |
| 11 | `active_predicted = max(1)` guard | (c) undocumented (effectively unreachable) |
| 12 | Doc nits: "units of Hz", stale ADR-0001 note, "Mathematical Equivalency" | (c) documentation only |

## Findings

### (a) Faithful

**1. Error-term structure (twm.rs:251–265, 300–311, 314–329).** Per-term
`Δf·f⁻ᵖ + (a/A_max)·(q·Δf·f⁻ᵖ − r)` matches E_w exactly in both directions.
Eq (1) weights by the *predicted* frequency fₙ; Eq (2) by the *measured* fₖ —
both verified correct in the code (an easy place to slip). aₙ in the forward
loop is the nearest measured peak's amplitude (Step 3, "set aₙ = Aₖ") —
matches. aₖ in the reverse loop is the peak's own amplitude — matches Eq (2)'s
signature (see the Step-5 wrinkle above). Totals combine as
`err_pm/N + ρ·err_mp/K` with N = the count of terms actually summed and
K = peaks.len() — matches Eq (3). Negative totals are preserved (pinned by the
negative-score golden in `test_twm_regression`), which is canonical, not a bug.

**2. Two-pointer nearest-match (twm.rs:209–219, 288–298).** The paper defines
association as minimizing |fₙ − fₖ| (Steps 3, 5) and is silent on algorithm and
tie-breaking. The two-pointer sweep over frequency-sorted sequences computes
exactly the nearest neighbor (both sequences monotone; nearest index is
non-decreasing); ties at exact equidistance go to the higher-frequency element
(the `<=` in the advance condition) — an unobservable choice the paper doesn't
constrain. Precondition (peaks sorted ascending) is established upstream at
`peaks.rs:162–164`, explicitly for this sweep. Faithful; O(N+K) vs the paper's
implied O(N·K) is a pure implementation win.

**3. Dynamic Bandwidth Cap — RECLASSIFIED faithful (twm.rs:147–164).** The
handoff and the (pre-audit) code comment both treated `cutoff = max_obs +
f₀·scale` as a bespoke addition. It is not — the precise result, refined at
the 2026-07-02 review pass:

- **B = 0 (harmonic): exact equivalence.** Step 2's count-form
  N = ⌈f_max/f_fund⌉ ("the series ends at the first harmonic at/above the
  highest measured partial") is identical to the cutoff-form
  {n·f₀ < f_max + f₀}, because consecutive harmonics are spaced exactly f₀
  apart. Exact for non-integer f_max/f₀; the integer case is measure-zero
  (and matches under the paper's stricter "smallest integer greater than"
  reading).
- **B > 0 (inharmonic): Step 2 is ambiguous** — the count-form and
  cutoff-form readings diverge because stiff-string spacing is stretched
  (f_{n+1} − f_n > f₀, since d/dn[n√(1+Bn²)] = (1+2Bn²)/√(1+Bn²) > 1). Ours is
  the cutoff-form, and it is the **conservative** resolution with a provable
  bound: with m = min{n : f_n·scale ≥ max_obs} (the count-form's last
  partial), every n < m is admitted by both readings, nothing past m can be
  admitted by ours (the next partial clears the cutoff by more than one
  spacing), and m itself is dropped iff it sits more than one fundamental
  above the observed band — where it has no discriminative value. **Difference
  ≤ 1 edge partial, only at the top of the band.**

**RESOLVED:** the derivation with the Step 2 citation is now in the code
comment at the cap (user-endorsed; behavior unchanged, goldens pass).
Switching to the count-form reading would be a behavior change (golden bits,
re-validation) for no benefit — not recommended; the cutoff-form is documented
as the deliberate resolution of the B>0 ambiguity. Note the paper's separate
user-parameter N (8≤N≤10 in their practice) is a *noise-robustness* cap; ours
is min(Step-2 cutoff, Nyquist, MAX_PARTIALS=128) via `KeyProfile` — the Step-2
formula itself is unbounded, so this stays within the paper's framework.

### (b) Deliberate documented adaptations

**4. Inharmonic predicted series (models.rs:256–276).** KeyProfile predicts
fₙ = n·f₀·√(1+B·n²) (stiff-string, B from the Rigaud prior) instead of the
paper's n·f_fund. Justification verified sound: the paper itself identifies
piano inharmonicity as a bias of its harmonic template (§III.A, citing
Fletcher 1964 — "the initial positive inharmonicity tends to bias the F₀
estimate upward"). Our substitution addresses the exact weakness the authors
named; it is the project's core purpose and is documented in the module docs
and internals/04.

**5. Search strategy: 88 discrete candidates + continuous scale
(discovery.rs).** The paper sweeps a continuous trial F₀ (semitone grid + local
refinement); we score 88 physical key templates at scale=1.0 (Stage A) then
golden-section refine the scale of the top-K (Stage B). Stage B is actually
close in spirit to the paper's "search in the vicinity of each local minimum
with a progressively smaller step size". The trial variable being a *uniform
scale* on an inharmonic template (B frozen at prior) is documented in
discovery.rs:14–22, including the caveat that a physically detuned string does
not move this way. Justified: a piano has 88 known candidates.

**6. λ ceiling on the per-peak M→P term (twm.rs:310, doc 95–111, 276–285).**
Not in M&B (neither error is bounded in the paper). Documented at length with
the Duan et al. 2010 citation; ADR-recorded; λ ships at 18 in both canonical
and tuned configs; asymmetry (M→P only) is consistent with its stated
rationale (spurious *observed* peaks far from any predicted partial asymptote
to a noise floor). Applied per-term before /K, so a strong aligned peak's
negative term is untouched (min caps from above only) — coherent. Two nits:
(i) the phrase "Mathematical Equivalency to Duan et al. (2010)" overclaimed —
it is an asymptote-*inspired* hard bound, the same proxy class ADR 0006
Corrections item 1 concedes for the non-peak term; **wording fix APPLIED
2026-07-02** (both the "As derived by" framing and the "Mathematical
Equivalency" heading now read as adaptation, with a pointer to this audit).
(ii) The forward (P→M) error remains unbounded as in the paper — fine, and the
doc block at twm.rs:191–199 already documents *why* no Duan Eq-7 cliff is
applied there (missing-fundamental robustness). That block is a model of what
this audit wants everywhere.

**7. Retuned default constants (twm.rs:52–71).** Shipped default q=3.88,
r=1.426, ρ=0.298 vs paper 1.4/0.5/0.33, with p=0.5 and λ=18 held. The paper
explicitly frames p, q, r, ρ as empirically determined tunables and invites
per-signal adjustment, so retuning is within-paper, not a deviation from it.
Provenance documented (doc comment cites ADR 0006; MOBO methodology note), and
the canonical math is guarded separately (`test_twm_regression` pins the paper
constants; `test_shipped_default_constants` pins the tuned ones). Verified
correct division of labor.

**8. Experimental config fields (twm.rs:17–49; loops 221–249; terms
319–328).** `sum_forward`, `b_deadzone`, `nonpeak_penalty`,
`smoothness_penalty` are bespoke (no M&B basis), clearly labeled EXPERIMENT
with design-note citations, ship OFF (byte-identical default, test-guarded),
and their rejections are recorded in ADR 0006 (which also already concedes
they are proxies, not faithful ports of Duan/Emiya). The `b_deadzone` formula
was checked against its own claim: tol = c·B·n²·fₙ/(2(1+Bn²)) is exactly
(∂fₙ/∂B)·(c·B) under the stiff-string law — internally consistent.
**Proposal (deferred, decision for the user):** after Prompt A pins the arm-6
candidate (which uses `nonpeak_penalty`), prune the refuted fields
(`sum_forward`, `b_deadzone`, `smoothness_penalty`) from the shipped scorer —
carrying dead experiments in the hot path indefinitely is exactly the drift
this audit series exists to catch.

### (c) Undocumented deviations (all minor; propose documenting, not removing)

**9. `f.max(1.0)` frequency-weight floor (twm.rs:258, 260, 302, 304).** The
paper's f⁻ᵖ is unguarded. Peak admission upstream only requires
`frequency > 0.0` (peaks.rs:65), so the floor is a real guard against weight
explosion for sub-1-Hz garbage, but it is inert for any plausible piano
content (A0 = 27.5 Hz) and carried no doc comment. **APPLIED 2026-07-02**,
kept by user decision. Review clarification: this is a guard, *not* a
computational optimization — the adjacent "fast path" sqrt/powf branch is the
optimization (it preserves the original 1/sqrt bit pattern; powf(-0.5) may
differ in ULPs), and the two are now documented separately at both call sites.

**10. `a_max.max(1e-6)` floor (twm.rs:143–145).** Paper assumes A_max > 0.
Div-by-zero guard for a degenerate all-zero-magnitude frame. **APPLIED
2026-07-02**, kept by user decision.

**11. `active_predicted = max(1)` guard (twm.rs:175–180).** Defensive and
effectively unreachable: the first predicted partial f₁·scale ≈ f₀·scale always
falls below cutoff = max_obs + f₀·scale whenever any peak exists. **APPLIED
2026-07-02**, kept by user decision — the comment quantifies the trigger
threshold (every peak below ≈ (B/2)·f₀·scale, ~20 Hz worst-case at C8) so a
future reader doesn't hunt for the case.

**12. Doc-comment inaccuracies — APPLIED 2026-07-02 (comments only).**

- "Returns Err_total in units of Hz" was wrong — Δf·f⁻ᵖ is Hz^(1−p) (Hz^0.5 at
  p=0.5) and the r term is dimensionless; the *paper's* E_w mixes dimensions
  too. Now documented as a figure of merit, not a quantity in Hz.
- "pending MOBO calibration per ADR 0001" was stale — now cites ADR 0006's
  tuned default and the canonical-vs-shipped test split.
- The paper-constants block now says explicitly these are the *paper's*
  values, NOT the shipped defaults (with pointers to `TwmConfig::default` and
  ADR 0006).

## Follow-up resolution (2026-07-02, same-day user review)

Items 1–4 (all doc-comments, no behavior) are **APPLIED**:

1. Step 2 citation + full derivation at the Dynamic Bandwidth Cap (finding 3).
2. "Mathematical Equivalency" → bounded-error adaptation wording (finding 6).
3. Guards documented at all three sites (findings 9–11); all kept by decision.
4. "units of Hz" fixed and the stale ADR-0001 note replaced (finding 12).

Verified byte-identical: `cargo test -p tuner-core --lib twm` passes both
goldens (`test_twm_regression` bit-patterns, `test_shipped_default_constants`).

Item 5 **DECIDED 2026-07-02 (user)**: the experimental fields (`sum_forward`,
`b_deadzone`, `smoothness_penalty`, `nonpeak_penalty`) are **kept until the
second instrument is introduced** — the same gate as every other structural
decision (ADR 0006). Revisit the prune at that point. The TWM audit is closed.

## Audit series status

| Item | Algorithm vs paper | Status |
| ---- | ------------------ | ------ |
| 1 | `twm.rs` vs Maher & Beauchamp 1994 | **DONE, fixes applied (this doc)** — core math faithful; bandwidth cap reclassified faithful (exact Step 2 at B=0, conservative at B>0); guards documented + kept; only the experimental-field prune decision remains (post-Prompt-A) |
| 2 | `spectral.rs` `cspe` vs Short & Garcia 2006 | **DONE 2026-07-02** (`faithfulness-audit-02-cspe.md`) — core faithful, caller contract verified; doc-only findings (Hamming/Hann stale doc, window-phase justification) |
| 3 | `spectral.rs` `jacobsen` vs Candan 2015 | **DONE + FIXED 2026-07-02** (`faithfulness-audit-03-jacobsen.md`) — **REAL BUG** (bespoke (−1)^m + missing c_N≈2 ⇒ −2.5δ bins/peak) fixed per Candan Eq 1+12; new baselines: discrete 76/87, refined 77/87 (were 74/87); pre-fix analyses stale → Prompt A′ |
| 4 | `peaks.rs` vs cited bases (was "Miron 2014" — phantom) | **DONE 2026-07-04** (`faithfulness-audit-04-peaks.md`) — extract_peaks textbook/no-claim ✓; caller's Kay-1998 NP threshold verified ✓; **mask_peaks Gómez citation FALSE** (no §3.1.2.2, no masking in thesis) → reclassified as *validated bespoke heuristic* (ADR 0002, kept, user-confirmed); Cano 1998 obtained → global gate = documented 40→30 dB Cano §4.3 adaptation; all comment/records fixes APPLIED |
| 5 | `metrics.rs` vs gatekeeper papers | **DONE + FIXED 2026-07-04** (`faithfulness-audit-05-metrics.md`) — rms/ema/nhwrsf structurally faithful (nhwrsf now cited Masri/Bello/Dixon + band de-hardcoded); **`ninos2` MISATTRIBUTED** → relabeled ours (N/N_eff participation-ratio form, H&R family); user-requested **A/B run** (`sparsity_ab.rs`): faithful ODFs = decay trackers; sparsity cores register-split (theirs wins bass/mid, ours wins treble); register-aware gate = candidate, gated on piano #2; rename optional |
| 6 | `models.rs` `get_expected_beta` vs Rigaud | **DONE + FIXED 2026-07-04** (`faithfulness-audit-06-b-prior.md`; renamed from `-rigaud` 2026-07-15 — wave 2's audit of `algorithms/rigaud.rs` owns that name) — model form faithful (Eqs 7–8); treble pair = paper's universal fit EXACT (re-index verified); bass pair = OURS by the paper's own design (now documented); **σ_B 0.157/0.116 "[Rigaud Fig. 3]" attribution FALSE** (not in paper — ours, corrected at all 3 sites) |
| 7 | `mat.rs` re-check (new lens) | **DONE + FIXED 2026-07-04** (`faithfulness-audit-07-mat.md`) — all Eq/§ citations verified accurate EXCEPT phantom **"§7"** (content is the Conclusion §4; fixed at 5 sites) + Fig-10 wording + 2 stale docs (Serial-default, tight-band); all OUR-constants classified (b)-documented; confidence machinery stays demoted-diagnostic per ADR 0006 |
| 8 | Goertzel usage in `engine.rs` | **DONE + FIXED 2026-07-04** (`faithfulness-audit-08-goertzel.md`) — recurrence/finalization verified textbook-correct; non-integer phase offset constant-per-target (differences exact — sound for the vocoder use, constraint now documented); NEYMAN_PEARSON_K verified to all digits vs Kay derivation; MQ/Dolson citations accurate; decision logic all ours+documented; missing Goertzel/Sysel-Rajmic citations added. **SERIES COMPLETE (8/8).** |
| 5′ | `metrics.rs` `ninos2` citation pinning (wave 2) | **DONE + FIXED 2026-07-15** (addendum in `faithfulness-audit-05-metrics.md`) — H&R *define* ℓ²/ℓ¹ (Table I + Thm 4.1); ours = N·(ℓ²/ℓ¹)² satisfies **all six** H&R criteria (D4/P2 from the ×N factor); **H&R Thm A.4/Table III D3 entry erroneous** (contradicts their own Thm A.19 — derivation on record); Hoyer §3.1 pinned, Bell & Dean 1970 added; doc-comment rewritten |
| 9 | `algorithms/rigaud.rs` vs Rigaud 2013 (wave 2) | **DONE + FIXED 2026-07-15** (`faithfulness-audit-09-rigaud.md`) — Eqs 7–9/20/29–31 + §III.B.1 treble pair + A&S 7.1.26 erf all exact; `RhoPhi::TYPICAL` mis-cite §III.B.3 → §III.A.3.b fixed; paper's +8.9e-2 s_B sign typo noted in-code; OURS optimizers/penalty labels verified |
| 10 | `algorithms/giordano.rs` vs Giordano 2015 + Sethares 1993 (wave 2) | **DONE + FIXED 2026-07-15** (`faithfulness-audit-10-giordano.md`) — Eqs 3–6 exact incl. all 5 Sethares constants; cross-pairs-only **reclassified faithful** (it IS Giordano's Eq 5 — old doc undersold it as ours); "Eqs 4–6" → "3–4" ×2; dropped ½ prefactor documented (inert); §VI.C gate quotes re-verified |
| 11 | `algorithms/whittaker.rs` vs Eilers 2003 (wave 2) | **DONE + FIXED 2026-07-15** (`faithfulness-audit-11-whittaker.md`) — Eqs 5/7/8/10/11 exact incl. λ-limit claims; LOO-identity cite Eq 10 → Eq 11 fixed; **w-weighted CV score = ours, documented** (identity exact for general diagonal W — Sherman–Morrison, brute-force test extended); λ grid = Eilers' own practice |
| 12 | `algorithms/curves.rs` assembly audit (wave 2) | **DONE + FIXED 2026-07-15** (`faithfulness-audit-12-curves.md`) — all Eq/§ claims verified (Eq-6/c_{m,k} conversions re-derived; §IV.C.2 `min` typo confirmed via Fig 9; §II.B chain incl. lower-note ρ indexing); OURS components all ADR-linked (w₀ = 4λ/ℓ⁴ re-derived); constants sweep clean; 2 Eq-10 cites + 1 stale GCV comment fixed. **WAVE 2 COMPLETE (5/5)** — tally in audit-12. |
| 13 | `peaks.rs` `coarse_read` (OS-CFAR) vs Rohling 1983 (wave 3) | **DONE + FIXED 2026-07-26** (`faithfulness-audit-13-cfar.md`) — Eq. 14 reproduces **all 32 `N = 32` entries of the paper's Table II** (new test) and Eq. 17's `√` is scoped to OS-CFAR by the paper itself; **"Rohling's own choice is the median" FALSE** (§III lists it as one option, §V recommends `k ≈ 3N/4`); quantile 0.25 **promoted ours-measured → derived** from §V's `(N − k)` criterion (`k/N ≤ 1 − W_lobe/s` = 0.25 at A0, measured lobe occupancy 75 %); `k < N/2` justified by the same criterion under an inverted H₀ (erosion cost = ADR 0011 §5's realized P_fa); guard cells **measured inert** on all 3 sets exactly as §V predicts → **REMOVED** (parity + P_fa re-verified); flank-floor mechanism ("finds the valleys") corrected at 3 sites — the selected cell is a *weak upper partial's* skirt, not a valley. **No value changed**; one inert constant deleted. |
