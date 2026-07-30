# Faithfulness audit 05 — `metrics.rs` (gatekeeper metrics) vs their cited papers

**Series:** Prompt B faithfulness audits (status table in `faithfulness-audit-01-twm.md`), item 5 of 8.
**Date:** 2026-07-04.
**Sources (all primary, read this session; PDFs in `resources/gatekeeper/`):**

- Mounir, Karsmakers & van Waterschoot (2021). "Musical note onset detection
  based on a spectral sparsity measure." EURASIP JASMP 2021:30 — the paper
  `ninos2` cites (`s13636-021-00214-7.pdf`).
- Mounir et al. (2016). "Guitar note onset detection based on a spectral
  sparsity measure." EUSIPCO — the original NINOS² (`mounir2016.pdf`).
- Giannoulis, Massberg & Reiss (2012). "Digital Dynamic Range Compressor
  Design — A Tutorial and Analysis." JAES 60(6) — the `ema` citation.
- Hurley & Rickard (2009). "Comparing Measures of Sparsity." IEEE Trans. IT
  (`0811.4706v2.pdf`) — the sparsity-measure family both papers build on.
- Noted, not applicable: Mounir et al. (2025) EURASIP howling-detection paper
  (same sparsity family, different application); the SFM lineage PDFs
  (Gray 1974, Johnston 1988, Tzanetakis 2002, 000008.pdf) are the
  **rejected** alternative per ADR 0003, not current bases.

**Scope:** `tuner-core/src/algorithms/metrics.rs` (`rms`, `ema`, `nhwrsf`,
`ninos2`) plus usage/thresholds in `gatekeeper.rs`. **No changes applied** —
finding 3 is a reclassification queued for user review.

## Verdict summary

| # | Item | Classification |
| --- | ---- | -------------- |
| 1 | `rms` | (a) textbook, correct |
| 2 | `ema` + gatekeeper's dynamic attack/release α | (a/b) faithful Giannoulis one-pole smoother (α-convention swapped); cold-start sentinel ours |
| 3 | `ninos2` | (c) **misattributed** — implements a Hurley–Rickard-family sparsity ratio, *not* Mounir's NINOS² (any variant); mechanism sound for its purpose |
| 4 | `nhwrsf` | (a/c) faithful SpectralFlux structure; band-limit + normalization are ours and uncited — needs its citation + ours-notes |
| 5 | Gatekeeper thresholds (0.5 / 10.0 / α=0.5) | (b) ours, already honestly labeled ("arbitrary") in config comments |

## Findings

**1. `rms` — textbook.** √(mean(x²)), matches its own formula, no claims. Fine.

**2. `ema` — faithful smoother, two notes.** The filter
`α·x + (1−α)·y_prev` is Giannoulis's one-pole ballistics smoother with the
**opposite symbol convention** (their α multiplies the *previous* output;
ours the current input — same filter under α ↔ 1−α; our doc's "higher = more
responsive" is consistent with our convention). More interesting: the
*gatekeeper's* use (α = 1.0 when rising, slow α on decay,
gatekeeper.rs:194–201) is exactly Giannoulis's decoupled attack/release peak
detector pattern — genuinely faithful in spirit, worth saying in the
doc-comment. The `previous_ema == 0.0 → jump to current` cold-start is ours:
it conflates "uninitialized" with a legitimate zero (benign here — the
gatekeeper resets the EMAs to 0.0 in Silence precisely so they re-seed on the
next active frame — but the sentinel semantics deserve one line).

**3. `ninos2` is not NINOS² — the audit's second misattribution (after
`mask_peaks`/Gómez).** The doc-comment claims to implement "the ℓ₁/ℓ₂ variant
(Eqs. 14–15)" of Mounir 2021. What the paper's Eqs. 14–15 actually are:

- **Preprocessing first (Eqs. 4, 6–7):** STFT **log**-magnitudes
  `Y_k = log(λ|X_k|+1)`, sorted ascending, keep only the **lowest
  J = ⌊γ/100·(N/2−1)⌋** coefficients (γ = 95.5 % tuned) — deliberately
  **discarding fundamentals and harmonics**. The paper stresses this
  low-energy-subset step as fundamental to the method.
- **Eq. 14:** Υ_ℓ₁ = ‖y‖₂·(‖y‖₁/‖y‖₂) = **‖y‖₁** — the ℓ₁-norm itself.
- **Eq. 15:** ℵ_ℓ₁ = ‖y‖₂/(√J−1)·(‖y‖₁/‖y‖₂ − 1) — normalized, but still
  carrying the **energy factor** ‖y‖₂ (deliberately *not* scale-invariant:
  onsets come with energy rises), and oriented as **inverse** sparsity
  (peaks at onsets/non-sparse frames).

Our implementation is `N·Σ|X|²/(Σ|X|)² = N·(‖X‖₂/‖X‖₁)²` over **all linear
magnitude bins except DC**: reciprocal orientation (high = sparse/tonal), no
energy factor (fully scale-invariant), no log compression, no low-energy
subset, squared. That is **not any equation in either Mounir paper**. It *is*
a legitimate, classical sparsity measure — an N-normalized squared ℓ₂/ℓ₁
ratio from the family surveyed by Hurley & Rickard 2009 (whose PDF sits in
`resources/gatekeeper/` already), with clean endpoints: 1-sparse → N, flat
(white noise) → ≈ 1.

**Why the deviations are (mostly) right for our purpose — and must be
documented as ours:** the Gatekeeper is not detecting onsets; it gates the
**tonal steady state** (the "golden window"). For that job: the reciprocal
orientation is exactly what's wanted (high = tonal); scale-invariance is
*required* by internals/04's own heuristic rules (Mounir's energy factor
would re-introduce hardware/gain dependence — the "fragile threshold" class);
and using **all** bins is defensible because we measure whole-spectrum
tonality rather than the noise-floor rise between harmonics. The
linear-vs-log choice is a real behavioral difference (linear is dominated by
the strongest partials) that has simply been validated implicitly by the
shipped gatekeeper. What survives of Mounir is the load-bearing *idea* —
spectral sparsity separates a note's transient from its steady state — which
is exactly what should be cited, as inspiration.

**Also noted (trivial):** the N factor uses `spectrum.len()` (includes DC)
while the sum skips DC, so white noise converges to len/(len−1) ≈ 1.001, not
exactly 1 — harmless, worth one honest line. The **function name itself**
(`ninos2`, and the `ninos2_*` config/telemetry fields) perpetuates the
misattribution; renaming (e.g. `spectral_sparsity`) would touch gatekeeper
fields, diagnostics CSV headers, and the GUI — **user's call**, and fine to
defer; the doc-comment fix suffices for the record.

**4. `nhwrsf` — faithful structure, uncited, two silent modifications.** The
core is the half-wave-rectified magnitude spectral flux — Mounir 2021 Eqs.
1–2, lineage Masri 1996 / Bello 2005 — currently cited **nowhere** in the
doc-comment. Two deviations are ours and undocumented: (a) the **band limit**
(bins 2–464 ≈ 43 Hz–10 kHz), hardcoded for 2048 @ 44.1 kHz — correct for the
gatekeeper's WINDOW_SIZE but silently wrong if the window ever changes
(precondition worth stating); (b) the **normalization by the current frame's
Σ|X|** — the paper's SF is unnormalized (LSF gets robustness from the log
instead); ours buys scale-invariance, same rationale as finding 3. Queued:
add the SF citation + mark both modifications ours.

**5. Thresholds.** `nhwrsf_threshold = 0.5` ("Arbitrary starting threshold"),
`ninos2_stability_threshold = 10.0`, `ninos2_ema_alpha = 0.5` — all ours,
already labeled honestly at the config site. No action beyond finding 3's
relabeling making their units ("1 = white noise … N = pure tone") accurate.

## A/B addendum (2026-07-04, user-requested): faithful NINOS² measured, not assumed

The user challenged the "deviations are right for our purpose" claim:
*implement the faithful NINOS² and measure it.* Built
`examples/sparsity_ab.rs`: replays all `diagnostics/key_*/` captures;
**time-anchored** classes (onset = first |x| ≥ 1 % of max, Mounir Eq 19
style; TRANSIENT = onset ± [−N/2, +90 ms]; STEADY = onset + [300, 1000] ms;
both classes RMS-gated at 5 % of max frame RMS so decayed treble tails don't
pollute the steady class); per-key Mann–Whitney AUC of transient/steady
separation, oriented per metric (1.0 = perfect); 74/87 keys yield both
classes. Faithful variants per Mounir 2021: preprocessing Eqs 4+6–7
(log(1+|X|), sorted ascending, lowest J = ⌊0.955·(N/2−1)⌋ kept), ODFs
Eqs 13/14/15, and the **level-independent sparsity core** Eq 12 (S̄, both
ℓ₂ℓ₄ and the ℓ₁ℓ₂ analog).

| variant | bass mean/min | mid mean/min | treble mean/min |
| --- | --- | --- | --- |
| ours N·(ℓ₂/ℓ₁)², linear, all bins | 0.703/0.08 | 0.689/0.00 | **0.981**/0.83 |
| full ODFs Eq 13/14/15 (energy-weighted) | 1.000 | 1.000 | 1.000 |
| S̄(ℓ₂ℓ₄) Eq 12, log+LE-subset | **0.999**/0.97 | **0.876**/0.05 | 0.382/0.00 |
| S̄(ℓ₁ℓ₂) Eq 12-analog | 0.990/0.84 | 0.629/0.00 | 0.114/0.00 |

**Findings from the A/B:**

1. **The full faithful ODFs' perfect 1.000 is their energy factor tracking
   the piano's decay envelope** (Eq 14 is literally ‖y‖₁ — total
   log-energy; any loudness proxy separates "attack" from "300 ms later"
   on a monotonically decaying instrument). As a *gate* they would be a
   loudness gate — the fragile-threshold class internals/04 bans, and
   redundant with the Gatekeeper's existing RMS/EMA. This empirically
   confirms that stripping the energy factor was necessary, not stylistic.
2. **The honest head-to-head is the sparsity cores, and neither dominates —
   they are complementary by register.** The paper's log+LE-subset core is
   decisively better in bass (0.999 vs 0.703) and better in mid
   (0.876 vs 0.689); ours is decisively better in treble (0.981 vs 0.382 —
   theirs drops *below chance* because discarding the top-4.5 % bins
   assumes many harmonics, and an extreme-treble note has ~2–5 partials in
   1023 bins, so the subset throws away the entire signal; a known
   source-property of this project's treble, cf. ADR 0006).
3. **So the audit's original "arguably right" claim was partly wrong**: the
   scale-invariance argument survives measurement; the "all bins is
   defensible" argument holds only in the treble. A register-aware gate (or
   the ℓ₂ℓ₄ core below some key split) is a real candidate upgrade —
   **not adopted now**: n = 1 instrument, the Gatekeeper is not a current
   bottleneck (87/87 keys already yield MAT measurements), and any swap
   needs its own threshold recalibration. Gated on instrument #2 with the
   rest.
4. Caveats: labels are crude time-anchored proxies; the Gatekeeper's true
   figure of merit is downstream capture quality, not this AUC; some
   bass/mid keys invert for ours (min 0.0 — plausibly unison-beating
   frames spreading the steady spectrum); 13 keys had no steady frames
   above the RMS floor (fast decays).

## Queued fixes — APPLIED 2026-07-04 (user go-ahead: "fix all known documentation issues" + the A/B request)

1. ✅ `ninos2` doc-comment rewritten: formal identity stated
   (N/N_eff; Cauchy–Schwarz effective support size = participation ratio =
   reciprocal Herfindahl/Simpson index; Hurley & Rickard 2009 family, affine
   to Hoyer 2004); ours, Mounir cited as inspiration; four deviations stated
   with the A/B result quoted; white-noise ≈ len/(len−1) nuance; name noted
   as historical.
2. ✅ `nhwrsf`: Masri 1996 / Bello 2005 / **Dixon 2006** citations added (the
   user correctly identified the canonical lineage — Mounir Eq 1 is a
   restatement); Σ|X| normalization documented as ours; and the hardcoded
   bins **replaced with runtime derivation** from new `(window_size,
   sample_rate)` parameters (user request): band 43 Hz–10 kHz, formulas
   reproduce the old bins 2/464 exactly at the shipped 2048 @ 44.1 kHz —
   byte-identical behavior, signature change, gatekeeper call site updated,
   28/28 lib tests pass.
3. ✅ `ema`: α-convention swap vs Giannoulis, cold-start sentinel rationale,
   and the gatekeeper's dynamic-α = the paper's attack/release pattern —
   all documented.
4. **DEFERRED (user decision 2026-07-04)**: rename `ninos2` →
   `spectral_sparsity`. Surface measured: 11 files, including the GUI view
   module `ninos2_calibration.rs`, settings/telemetry plumbing,
   `diagnose_gatekeeper`'s CSV column names, and `plot_gatekeeper.py` which
   parses them — plus historical `gatekeeper.csv` header compatibility.
   Fails the "just change the function name" bar; the doc-comment carries
   the record. Revisit only if the gatekeeper is ever reworked anyway.

**Doc-bloat trim (2026-07-04, user feedback):** the audit's in-code comments
had drifted into narrating audit history (A/B statistics in `ninos2`,
mis-citation stories in `mask_peaks`/`jacobsen`). Trimmed to
constraint-statements + pointers; the histories live here and in audits
03/04. Tests unchanged (28/28).

## Addendum (2026-07-15, wave 2 / Prompt B′ item 1) — ℓ²/ℓ¹ citation pinning

**Motivation.** The 2026-07-04 fix relabeled `ninos2` as ours and cited
Hurley & Rickard family-level ("the ℓ₁/ℓ₂ family"), leaving three identity
claims asserted rather than source-verified. Concern raised by the user:
does H&R actually *define* the ℓ²/ℓ¹ measure, or only mention it? Sources
read: `resources/gatekeeper/0811.4706v2.pdf` (= IEEE TIT 55(10), Oct 2009)
and `resources/curve/hoyer04a.pdf` (Hoyer 2004, JMLR 5).

### Addendum findings

**1. H&R DO define the measure — the concern is dispelled.** Table I
("Commonly used sparsity measures…") defines the ℓ²/ℓ¹ entry as
√(∑ⱼcⱼ²)/∑ⱼcⱼ = ‖c‖₂/‖c‖₁ (over non-negative coefficients — ours are
magnitudes), and the definition is restated in-text in the proof of
Theorem 4.1. Citation now pinned to Table I + Thm 4.1. Exact relation:
**S = N·(ℓ²/ℓ¹)²** — at fixed N a strictly increasing transform of the H&R
measure.

**2. Criteria profile derived — S satisfies all six H&R criteria.**
H&R's Table III row for ℓ²/ℓ¹: D1 (Robin Hood) ✓, D2 (Scaling) ✓,
P1 (Bill Gates) ✓; D3 (Rising Tide), D4 (Cloning), P2 (Babies) ✗. For
S = N·(ℓ²/ℓ¹)²:

| Criterion | S | Source |
| --- | --- | --- |
| D1 Robin Hood | ✓ | H&R Thm 4.1 + strict monotone transform at fixed N |
| D2 Scaling | ✓ | ℓ²/ℓ¹ scale-invariant; N unchanged |
| D3 Rising Tide | ✓ | direct derivation below (H&R's ✗ is their error — finding 3) |
| D4 Cloning | ✓ | m-fold clone: S = mN·(m∑c²)/(m∑c)² = N·∑c²/(∑c)² — exact invariance; the ×N factor is precisely what repairs bare ℓ²/ℓ¹'s D4 failure |
| P1 Bill Gates | ✓ | H&R Thm A.5 + monotone transform |
| P2 Babies | ✓ | appending a zero: sums unchanged, N → N+1 ⇒ S strictly increases |

**3. H&R Theorem A.4 (ℓ²/ℓ¹ fails D3) is WRONG — an internal
inconsistency in the paper.** Their own Theorem A.19 proves Hoyer
satisfies D3, and Hoyer = (√N − ℓ¹/ℓ²)/(√N−1) is a strictly increasing
function of ℓ²/ℓ¹ at fixed N (their own observation: "a normalized version
of the ℓ²/ℓ¹ measure"). D3 changes no dimensions, so a strict monotone
transform preserves it both ways — the two table entries cannot differ.
Direct derivation (s₁ = ∑c, s₂ = ∑c², α > 0): squaring
√(s₂+2αs₁+Nα²)/(s₁+Nα) < √s₂/s₁ and clearing reduces to
2s₁(s₁²−Ns₂) < Nα(Ns₂−s₁²); Cauchy–Schwarz gives Ns₂−s₁² > 0 for
non-constant c (the constant case is excluded by H&R's own D3 definition),
so it reads −2s₁ < Nα — always true. **ℓ²/ℓ¹ satisfies D3.** Thm A.4's
algebra mis-handles the division by (s₁²−Ns₂) < 0 (it also asserts
s₁² > s₂, true, where the relevant comparison is against Ns₂). Numeric
spot-check: [1,3,5] → +0.5: 0.6573 → 0.6371 (decreases, as D3 wants).
No published erratum found (web search 2026-07-15). Consequence for the
record: the doc-comment cites Table III for the *inherited* checks only and
flags the D3 entry with a pointer here.

**4. "Affinely related to Hoyer (2004)" was imprecise.** Hoyer §3.1
(unnumbered display), sparseness(x) = (√n − ‖x‖₁/‖x‖₂)/(√n − 1), is affine
in the ℓ¹/ℓ² *ratio*, and therefore strictly monotone — but **not affine**
— in S at fixed N. Wording fixed.

**5. Participation-ratio lineage pinned.** N_eff = 1/∑qⱼ² with
qⱼ = |Xⱼ|/∑|X| is the participation ratio of the ℓ¹-normalized magnitude
distribution — primary source Bell & Dean 1970 (Discuss. Faraday Soc. 50,
55–61; verified: the paper introduces the participation ratio for
vibrational-mode localization). Reciprocal-Herfindahl/Simpson identity kept
as a stated synonym (algebra, not a port). Cauchy–Schwarz bound direction
verified: ‖X‖₁² ≤ ‖X‖₀·‖X‖₂² ⇒ N_eff ≤ ‖X‖₀ — "lower bound on ‖X‖₀" as a
support estimate is correct as written.

### Applied (same session)

`ninos2` doc-comment rewritten: formula line extended with = N·(ℓ²/ℓ¹)²;
Table I/Thm 4.1 pinned as the definition source; six-criteria statement with
per-criterion provenance; Hoyer relation re-worded (affine in the ratio,
monotone-not-affine in S); References block now lists H&R (with tables/
theorems and pages), Hoyer §3.1, Bell & Dean 1970, Mounir (inspiration).
No behavior change (comment-only); the `ninos2 → spectral_sparsity` rename
stays deferred per item 4 above.

## Audit series status

Item 5 complete (this doc), **including the wave-2 addendum** (2026-07-15).
Running table: `faithfulness-audit-01-twm.md`.
Next: item 6, `models.rs::get_expected_beta` vs Rigaud (PDF in
`resources/moba/`); then 7 (MAT re-check), 8 (Goertzel). Wave 2 continues
with audits 09–12 (curve modules).
