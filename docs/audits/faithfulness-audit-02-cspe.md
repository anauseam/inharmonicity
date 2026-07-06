# Faithfulness audit 02 — `spectral.rs::cspe` vs Short & Garcia 2006

**Series:** Prompt B faithfulness audits (status table in `faithfulness-audit-01-twm.md`), item 2 of 8.
**Date:** 2026-07-02.
**Source of truth:** the actual paper PDF,
`resources/engine/Signal_Analysis_Using_the_Complex_Spectral_Phase_Evolution_(CSPE)_Method.pdf`
— Short, K.M. & Garcia, R.A. (2006), "Signal Analysis Using the Complex
Spectral Phase Evolution (CSPE) Method," AES 120th Convention, Paris,
Paper 6645.
**Scope:** `tuner-core/src/algorithms/spectral.rs::cspe` plus its caller
contract (`worker.rs::process_payload`, the `fft` windowing precondition, and
the `mat.rs` test harness). `jacobsen` (Candan 2015) is series item 3;
`goertzel` is item 8. **No behavior changed**; queued comment fixes at the end.

## Paper specification (extracted for the record)

- **Setup (Eqs 2–5):** two N-sample frames of the same signal, s₀ and s_Δ
  (delayed by Δ samples; the development uses Δ=1, where s₁ is the frame
  *advanced* by one sample). DFT convention W = e^(−j2π/N) (Eq 1).
- **CSPE (Eqs 6, 12, 35–37):** CSPE_k = F_k(s₀)·F*_k(s₁), element-by-element.
  For a complex sinusoid with period q+δ bins, CSPE = e^(−j2π(q+δ)/N)·‖F(s₀)‖².
- **Frequency estimate (Eq 7 complex; Eq 38 real windowed):**
  f_CSPE(k) = −N·∠(CSPE_k)/(2π) = q+δ (in bins), independent of the bin k. For
  Δ>1 the scaling is readjusted.
- **Real signals (Eqs 8–16):** a real sinusoid is two complex images at
  ±(q+δ); positive-frequency bins (k>0) remap to +(q+δ), negative bins to
  −(q+δ) (Eq 16). Interaction terms Γ (Eqs 14–15) bias the estimate unless an
  analysis window confines leakage; with a "reasonable analysis window" Γ ≈ 0.
- **Windowing (Eqs 17, 22–38):** both frames are pre-multiplied by the *same*
  analysis window A(n) (Eqs 23–24). In the conjugate product the window enters
  as `M_(k−β)·M*_(k−β) = ‖M_(k−β)‖²` (Eq 37) — **the window's phase cancels
  identically**, so Eq 38 recovers q+δ exactly for any known window, provided
  leakage keeps the cross terms negligible (`‖M_(k+β)‖ ≈ 0`, Eq 36). Hamming,
  Hanning, and rectangular are named; rectangular's leakage is warned against
  (§2.2).
- **§2.3.2 (Eqs 39–40):** magnitude and phase remapping — reconstruction of
  a·e^(jb) from the spectrum and the CSPE frequency. A separate, optional stage.
- **§3 practical notes:** estimates are most accurate at bins dominated by a
  single component; leakage bins remap to the nearest dominant component;
  real-world accuracy ~0.1 Hz for components stable over the window.

## Verdict summary

**Faithful port of the frequency-estimation core (Eqs 7/38, Δ=1), with the
caller contract verified end-to-end.** The genuine findings are documentation:
the doc-comment justifies window-immunity with an imprecise argument (the
paper's own cancellation result is stronger), and the module header + `fft`
docs claim a **Hamming** window while the code implements **Hann** — a stale-doc
bug caught by checking the CSPE windowing precondition.

| # | Item | Classification |
| --- | ---- | -------------- |
| 1 | Core formula f = −∠(F(s₀)·F*(s₁))·fs/(2π), Δ=1 | (a) faithful (Eq 7/38 in Hz) |
| 2 | Caller contract: advanced frame, identical Hann, same plan | (a) faithful (Eqs 23–24) |
| 3 | Positive-bin scope (first N/2 RFFT bins) | (a) faithful (Eq 16, k>0) |
| 4 | Frequency-only port (Eqs 39–40 not ported) | (a) faithful partial port, scope documented |
| 5 | Bin-centre fallback for ≤0 / non-finite estimates | (b) deliberate, documented |
| 6 | Doc justification "symmetric, real response adds no phase" | (c) imprecise — paper's cancellation is the real reason |
| 7 | Module/`fft` docs say Hamming; code is Hann | (c) stale doc — factual bug in comments |
| 8 | Silent `.min()` count clamping | (c) trivial — inconsistent with `fft`'s panics |

## Findings

### (a) Faithful

**1. Core formula (spectral.rs:143–146).** `product = spectrum[bin] *
spectrum_shifted[bin].conj(); freq = −product.arg() · fs/(2π)`. Paper Eq 7
gives f in bins as −N·∠(CSPE)/(2π); converting to Hz multiplies by fs/N,
yielding exactly −∠·fs/(2π). Verified sign chain: realfft/rustfft use the
standard forward DFT e^(−j2πnk/N) = the paper's W (Eq 1); the shifted frame is
*advanced* (paper Eq 5: s₁ = e^(jω)s₀ per component), so F(s₀)·F*(s₁) =
‖F‖²·e^(−jω) and the negation recovers +ω. A delayed frame would flip every
estimate negative and trip the fallback on all bins — loudly visible, and the
`mat.rs` golden tests exercise real CSPE maps, pinning the sign. The Δ=1
specialization is a documented contract ("advanced by one sample"); the paper's
Δ>1 rescaling is not needed.

**2. Caller contract (worker.rs:147–165; fft at spectral.rs:38–71).** The
paper requires the same signal, shifted, with the *identical* analysis window
on both frames (Eqs 23–24). Verified: both calls go through `fft`, which
windows internally (identical Hann, same plan/length); the shifted input is
`stable_buffer[1..fft_size+1]` — same frame advanced one sample, bounds
documented at the call site (capture buffer 66150 > fft_size ≤ 65536 + 1).
The offline harness (`mat.rs::synth_inharmonic`) reproduces the same contract
("one extra sample so the one-sample-shifted frame is fully populated").
CSPE is Worker-only; Discovery uses the single-FFT `jacobsen` instead — the
division of labor the doc-comment states.

**3. Positive-bin scope (spectral.rs:136–143).** Only the first N/2 bins are
mapped, matching Eq 16's k>0 case for real signals (negative-frequency bins
remap to −(q+δ), which we never hold — the RFFT emits only k ≥ 0). Bin 0
degenerates to the fallback (0 Hz bin centre); the Nyquist bin is excluded,
consistent with `magnitude_spectrum`'s convention.

**4. Frequency-only port.** §2.3.2's magnitude/phase remapping (Eqs 39–40) is
deliberately not ported — amplitudes come from `magnitude_spectrum` (and
Goertzel in the engine). The doc-comment's citation "(Eqs. 7, 38.)" accurately
delimits the ported scope. The per-bin *map* usage (every bin reassigned to
its dominating component; MAT reads the map at magnitude peaks) matches §3's
described behavior of the method.

### (b) Deliberate documented adaptation

**5. Bin-centre fallback (spectral.rs:147–152).** The paper does not define
behavior for degenerate bins (no coherent component ⇒ near-zero product with
meaningless angle; or a bin dominated by the negative-frequency image ⇒
negative estimate, possible near DC). Ours falls back to the bin-centre
frequency for non-finite/≤0 estimates, documented in the doc-comment.
Justification verified sound: downstream (MAT) reads the map only near
magnitude peaks, where a real positive component dominates; the fallback keeps
the map total and harmless elsewhere.

### (c) Undocumented deviations / documentation bugs

**6. Imprecise window-immunity justification (spectral.rs:110–111).** The
doc-comment says the estimate holds under Hann because "the window's
symmetric, real response adds no phase at the peak." That is not the paper's
argument, and as stated it is not quite true: the pipeline's Hann is defined
over [0, N−1] (causal), so its DFT response carries a linear-phase factor —
it is *not* phase-free at the peak. The estimate is nevertheless exact, for
the stronger reason the paper proves (Eqs 35–38): both frames carry the
*identical* window, so its phase cancels in the conjugate product
(M·M* = ‖M‖², Eq 37) — any known window works, provided leakage confines the
interaction terms (Eq 36's ‖M_{(k+β)}‖ ≈ 0; Hann qualifies, §2.3). Same class
as TWM audit finding 6 ("Mathematical Equivalency"): right conclusion,
wrong-strength justification. **Queued comment fix.**

**7. Module docs claim Hamming; the code is Hann (spectral.rs:9, :23 vs
:57–59).** The module header ("Hamming windowing for zero-overlap transient
preservation") and the `fft` doc-comment step 1 ("Hamming windowing (preserves
8% amplitude at frame boundaries)") describe a **Hamming** window — the 8%
boundary floor is a Hamming property. The code implements **Hann**
(`0.5·(1−cos)`, zero at boundaries), the inline comment says Hann/COLA, and
`worker.rs` *relies* on the Hann zero-boundary ("the Hann window zeroes the
lone boundary sample regardless"). The window choice itself is
paper-sanctioned (§2.3 names Hanning) and interacts correctly with CSPE; only
the docs are stale — presumably from a pre-Hann revision. **Queued comment
fix** (module header + `fft` doc). Caught here because the window is a CSPE
precondition; also noted for audit item 8 (Goertzel — `HANN_1024` is
correctly named).

**8. Silent count clamping (spectral.rs:136–139).** `cspe` harmonizes
`count = min(N/2, out.len(), spectrum.len(), spectrum_shifted.len())` and
silently computes fewer bins on undersized buffers, whereas `fft` panics on
its size contract. An undersized `out` would silently leave a truncated map —
inconsistency that could mask a caller bug. All current callers pass exact
sizes. Trivial; propose either documenting the truncation as intended or
matching `fft`'s panic contract. **No change in the audit.**

## Follow-up resolution (2026-07-02, same-day user review)

All three findings **APPLIED** on user go-ahead:

1. Window-immunity justification reworded to the paper's cancellation
   argument, citing Eqs 36–37 (finding 6).
2. Module header and `fft` doc fixed: Hamming → Hann, 8% claim dropped,
   COLA/zero-boundary stated (finding 7).
3. Finding 8 **patched** (the one behavior change, user-directed): the silent
   `.min()` truncation is replaced with `fft`'s panic contract — undersized
   `spectrum`/`spectrum_shifted` panic explicitly; `out` is sliced to
   `count` up front (panics if short), which also lifts the bounds checks out
   of the hot loop (iterator zip, no per-bin indexing). Observable behavior
   changes only for buggy callers; all callers (worker.rs, mat.rs tests,
   `validate_mat.rs`, `mat_b_recovery.rs`) verified to allocate exact
   `fft_size/2` buffers. Full `tuner-core --lib` suite passes (27/27,
   including `test_cspe_super_resolution` and the MAT goldens that consume
   real CSPE maps); examples compile.

## Audit series status

Item 2 of the series is complete; the running status table lives in
`faithfulness-audit-01-twm.md`. Next: item 3, `spectral.rs::jacobsen` vs Candan 2015
(no PDF in `resources/` — user to supply if available).
