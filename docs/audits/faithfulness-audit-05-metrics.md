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

## Audit series status

Item 5 complete (this doc). Running table: `faithfulness-audit-01-twm.md`.
Next: item 6, `models.rs::get_expected_beta` vs Rigaud (PDF in
`resources/moba/`); then 7 (MAT re-check), 8 (Goertzel).
