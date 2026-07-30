# ADR 0011 — The Coarse Spectral Readout (bounded search + OS-CFAR)

## Status

**IMPLEMENTED 2026-07-25 (Prompt N).** Five measurement rounds settled every
constant and rule before any hot-path code was written; this ADR carries the
numbers and the decisions they forced. The shipped read
(`algorithms::peaks::coarse_read`, pipeline step 5b,
`FrameOutput::coarse_hz`) is a **bit-exact port** of the harness code the
measurements were taken with — verified on all three capture sets
(§Reproduction). The strobe band remains the accurate readout inside its range;
this is what the display falls back to outside it.

**Tier-2 and display smoothing are CLOSED, not deferred (live check,
2026-07-26)** — the motion tail proved not to be visible in use; see
Limitations. Still deferred: multi-partial coincidence annotation.

## Context

The strobe panel had two readouts and both failed in the regime this app
explicitly targets — a badly out-of-tune string being pulled to pitch.

- The **band-slope** read (phase-integrated, ±0.2 ¢ jitter) aliases past
  ±0.5·fs/HOP ≈ 21.5 Hz. That limit is fixed in *Hz*, so in cents it is
  ≈ 37200/f: ±1000 ¢ at A0 but only ±85 ¢ at A4, ±21 ¢ at A6, **±9 ¢ at C8**.
  A7's string sat 25 Hz off ET in the captures — already aliased.
- The **fallback** was the engine's adaptive tracker, which needs a note lock,
  and whose α = 0.05 EMA (τ ≈ 0.46 s) is outrun by a turning peg: measured
  aliasing of 152–387 ¢ at detune rates of 200–400 ¢/s.

A bounded read of the magnitude spectrum has neither limit: it is
target-relative, lock-independent, and its error is its own group delay. The
open question was whether it *works* — the prompt required this be measured, not
assumed, and pre-registered a three-way comparison (tracker as-is / tracker with
the Defect-1 window fix / spectral read) on availability, accuracy against an
independent hi-res DFT truth, and behavior under simulated detuning.

The instrument for all of it is `examples/pitch_ground_truth.rs`.

## Decisions and the measurements that forced them

### 1. Defect-1 first — the window fix does not remove the need

The engine tracker called a hardcoded 1024-sample Goertzel for every note
(Hann main-lobe half-width 2·fs/N = ±86.1 Hz), so guitar E2's 82.4 Hz partial
spacing put its 2nd partial *inside the main lobe*; the strobe bank had received
this fix (R3) and the tracker had not. Giving the tracker the same
register-dependent window: **E2 jitter ±47.3 → ±4.3 ¢, A1 ±200 → ±20 ¢**, and a
stable +115 ¢ lie at F#1 disappeared; A2 and above byte-identical.

That answered open question 1 in the affirmative *for jitter* and left the
coarse read still justified: A0–D#1 at n = 1 remain unusable (acoustic — the
fundamental is barely radiated), and neither the alias limit nor the
lock-dependence moved at all.

### 2. The window rule is a *validity* condition, derived three times

**N > 4·fs/f₀** — the analysis window must be long enough that the band clears
the 2nd partial's main lobe. It was derived from partial spacing, then
independently confirmed twice:

- as a *resolution* rule (2048 breaks D2, marginal at E2, clean from F2 —
  predicted boundary 86.1 Hz);
- as the condition for **local noise estimation** to mean anything: at 2048 the
  ±43 Hz Hann lobes *tile* the low-mid spectrum, so there are no inter-partial
  valleys, the CFAR reference cells read lobes as noise, and the gate silently
  degrades to a peak-to-lobe-cusp ratio test below ≈ F3. (Recorded as a caveat:
  P_fa semantics are void there *while a note sounds*; it stays safe on dead
  notes.)

This retired the round-1 plan of assigning 2048 by register.

### 3. FFT size: dual-window read, not a register split

Both spectra are already computed every hop (2048 at the step-4 tap, 8192 at the
chain), so reading both costs nothing but the search. Measured, static:

| | 8192 | 2048 |
| --- | --- | --- |
| availability, C8 | **94 %** | 66 % |
| availability, everywhere else | dominates | — |
| F2 / A2 availability | fine | **0 %** (§2 lobe tiling) |

Under simulated detuning the ordering **reverses**:

| detune rate | 8192 availability | 2048 availability |
| --- | --- | --- |
| 0 ¢/s | 100 % | 100 % |
| 200 ¢/s | 71 % | 86–99 % |
| 400 ¢/s | 39 % | 86–99 % |

⇒ **the real split is motion, not register.** Rejected: 8192-only (no motion
currency), an explicit motion detector (new state, new threshold), and a
register split (static jitter cost). Adopted: run the search on both and take
8192 when it is admitted, else 2048 — **zero constants, zero state**, because
8192's smearing-induced availability loss *is* the motion detector. Simulated
tier-1: static behavior 8192-identical (churn 0 %), motion availability restored
(200 ¢/s 71 → 100 %, 400 ¢/s 39 → 86 %), source churn 4–6 %.

Round-1's "duration kills 8192 in the treble" was an ambient-gate artifact and
is retired.

### 4. The gate must be CFAR, not the shipped ambient threshold

The shipped Neyman–Pearson gates test against the calibrated *silence* RMS. Its
H₀ is a quiet room, which is the wrong null while a note is sounding — and the
consequence is not subtle: the ambient gate admitted **100 % of ±400 ¢ deep-bass
garbage**. This confirms by measurement the σ-misspecification recorded in
`docs/internals/suspected-issues.md`.

Ordered-statistic CFAR (Rohling 1983; Finn & Johnson 1968 lineage) estimates the
threshold from the neighbourhood of the cell under test. Settled configuration,
by sweep:

- **quantile 0.25**, below the median. Recorded here as a sweep result; audit 13
  (2026-07-26) showed it is **fixed by Rohling's own §V criterion** — an
  inhomogeneity is tolerable only while it affects fewer than `N − k` cells, so
  for a harmonic comb `k/N ≤ 1 − W_lobe/s`, which is 0.25 at A0. At the median
  that bound is violated for every key up to F1 and deep-bass junk is admitted,
  as measured.
- **no guard cells.** Shipped as ±2 (the peak's own main-lobe half-width), then
  **removed 2026-07-26** by audit 13: Rohling §V states guards are unnecessary for
  an OS detector, and sweeping 0…4 changed availability, error and jitter by
  nothing on any capture set. Flanking references make it structural — the only
  lobe cells that can enter sort above a low quantile. Re-verified after removal:
  parity 100.0000 %/Δf = 0 both sides, realized AWGN P_fa 0.00097 → 0.00102
  against nominal 0.001 with the per-bucket structure unchanged.
- **flanking** reference cells, never in-band: the capped deep-bass band is
  ≈ 5 bins wide, so in-band references are both too few and entirely inside the
  peak's own lobe.
- **flank half-width 1.5 × spacing, floored at 172 Hz** — *in Hz, not bins*,
  because a bin is 5.4 Hz at 8192 and 21.5 Hz at 2048, so a bin-specified floor
  silently quadruples when the read switches size. An 8-bin flank collapses A0
  availability; 64 bins leaks 16 % junk; 32 bins at 8192 = 172 Hz is the pick.
  Audit 13 corrects the *mechanism*: at 5-bin spacing 75 % of cells are inside
  some lobe and there are no valleys to reach, so what the wide flank buys is the
  **weak upper partials** (172 Hz spans partials ≈ 1–11 at A0, skirts 19–36 dB
  down) — the selected cell is a lobe cell in 56–68 % of deep-bass hops, and a
  valley cell in ≥ 95 % of hops from F1 up.
- **Rohling Eq. 14 finite-N** exact multiplier, with **Eq. 17** `T_lin = √T_q`
  for our Rayleigh-magnitude cells. Pinned values at P_fa = 10⁻³, k = median:
  N = 10 → 4.835, 16 → 4.089, 32 → 3.582, 64 → 3.360, ∞ → 3.157 (the asymptotic
  quantile form `√(ln P_fa / ln(1−q))`, mutually validating).

Result: **0 % junk admission** in the deep bass at the same median accuracy, and
C8 availability **42 → 100 %** (tail jitter 23–40 ¢ — median-accurate,
tail-jittery).

### 5. The search loss — a real defect the radar framing does not cover

Realized AWGN false-alarm rate came out **0.0386 against a nominal 0.001 (39×)**.
Isolated by collapsing the search band to a single bin, which brought it to
0.0012 — i.e. Rohling's per-cell calibration is *exactly* right and the excess
is entirely the **argmax over the band**: a multiple-comparisons search loss.
Fix: budget `P_fa / M` per cell, with `M` = band width halved (Hann correlation
makes adjacent bins non-independent). Realized after the fix: **0.00097**.

Per-band-width audit at 8192 shows one bucket (17–32 bins) at 0.0027 — **2.7×
nominal, i.e. slightly permissive**, bounded and on record. At 2048 every bucket
is ≤ 0.0017 (os25 = 0.00005, over-conservative — acceptable in a fallback role).

### 6. Search band: cents span, bin floor, neighbour cap

Half-width = `max(±100 ¢, 4 bins)` **capped at spacing/2**. The cap is what
makes the bin floor safe: at 2048 four bins is an 86 Hz half-width, wider than a
bass fundamental, and uncapped the read at E2/A2/A0 returns the **2nd partial**
(+1200 ¢). `mat.rs`'s 4-bin floor could not be copied without it — that floor
lives in the Worker's 2¹⁶ FFT where a bin is 0.67 Hz.

The cap must use the **partial spacing, not the centre frequency**: a read
centred on A0's 4th partial has centre ≈ 110 Hz but spacing ≈ 27.5 Hz, and
capping at centre/2 would admit a ±55 Hz band spanning two neighbours.

A capped flank variant was tried and **refuted** (cap-32 admits −21.5 ¢ at C8 at
88 % availability); the uncapped flank ships.

### 7. Which partial: fixed n\* = 4 below key 16, else 1

Deep bass cannot be read at n = 1 at all (availability 0 % at G1, 32 % at F1;
B1 reads 10.7 ¢ off at 81 %), so the coarse read needs its own partial — and
deliberately *not* the strobe's display table (6/4/2/1).

The two instruments disagree under a per-instrument optimum and that is what
decides it: on piano-2, n\* = 5 is the only strict all-pass (n = 4 misses C#1 on
jitter, 10.2 vs 10.0); on the guitar, **E2 fails hard at n = 5 (jitter 13.8 ¢,
and 23 % availability under the final gate) and passes at n = 4**. Cross-
instrument answer: **fixed n\* = 4**. Two independent tiebreaks agree — cold-start
bias with an unmeasured B grows as (n²−1), 5 ¢ at n = 4 vs 8 ¢ at n = 5; and
selecting the per-key margin argmax instead rides systematically high (B0 +7.5 ¢
vs +0.7 ¢ for fixed 4).

**Handover at key 16 (C#2)**, derived: n = 1 is clean from 16 up, and n = 4 is
clean across keys 8–20 — the two overlap, so this is a handover *zone*, not a
knife edge. Every guitar string (key ≥ 19) therefore reads on the fundamental,
coherent with ET mode publishing only the fundamental reference.

**Per-hop argmax is refuted, not merely unbuilt**: switching partials steps the
displayed number by 866·ΔB·n², measured at 20 ¢+, so it would need hysteresis
constants and partial-identity plumbing to the GUI to buy nothing.

### 8. Silence, and what "room noise" was

Room noise gives P_fa ≈ 0.34 at any quantile — colored rumble *is* narrowband
energy, so it is not a valid H₀ for this gate. The resolution is that it never
reaches the gate: calibration sets the silence threshold above ambient, so at
the live default (0.005) **95 % of pre-onset frames are `Silence`** and the
coarse read is simply not computed there. The "room P_fa 0.34" figure is struck
from the record — it was never an H₀ measurement (it contained real ring-out
energy).

This also resolved the display question. The user's "suppress unless locked" is
vacuous in manual mode (a manual lock is an assignment on any non-silent frame),
so it is implemented as **compute outside `Silence` only**, preserving
lock-independence.

### 9. Cold start, and the honesty bound

On an unmeasured key the references come from the Rigaud B prior. The truth
spread across n = 2..6 under prior-B references is **median 11.3 ¢, max 15.2 ¢**
(25 captures, keys 0–8) — approved as acceptable for a *coarse* readout. An
earlier "30 ¢" figure was an artifact of fitted-garbage-B references and is
retracted.

The prior is load-bearing here and B = 0 is **not** an acceptable stand-in: a
harmonic reference at the coarse partial reports the string's entire stretch as
mistuning (≈ 24 ¢ at A0 on a 4.7×-prior string, vs ≈ 19 ¢ residual against the
prior). The strobe band is indifferent to the choice — its n = 1 target is f₁
for any B, bit-identically — so the prior applies to the higher references only,
which is exactly what the coarse read consumes and what every measurement round
used.

The equal-cents identity makes the partial choice free of reference error:
because fₙ = n·f₀·√(1+Bn²) is linear in f₀, a partial's cents deviation from its
own target **equals** the string's deviation from its target, exactly. The GUI
therefore reads `coarse_hz` against the coarse partial's own reference.

## Consequences

- `FrameOutput` gains `coarse_hz: Option<f32>` (crossing #2 widen — **no
  seventh crossing**). `StrobeRefUpdate` gains `coarse_index: u8` and
  `spacing_hz: f32`, staying `Copy` and heap-free; the pipeline retains the
  drained update so the bank and the search see one reference set.
- Readout selection in the GUI: band-slope while ungated, filled and in range;
  else the coarse cents (labelled); else "listening…". The range test is
  *derived*, not a new constant — the coarse read supplies the offset in cents,
  so the offset in Hz at the displayed reference r is r·(2^(¢/1200) − 1),
  compared against `BAND_READABLE_HZ`. The **range verdict is debounced over 8
  hops** (§10, 2026-07-26); the other two conditions act immediately.
- The engine-tracker fallback (`StrobeState::live_hz`) is **removed**; the
  coarse read replaces it.
- Sustained-vs-attack convention unchanged (**sustained**), now measured at
  +5.4..+6.2 ¢ at A4 — larger than the ~4 ¢ previously recorded. It confounds a
  |e| ≤ 2 ¢ criterion in the mid/treble, where availability and jitter are the
  decision columns.
- α = 0.05 on the engine tracker is **kept** (user-agreed): it is the
  pitch-raise follower and the lobe-centering loop, and α = 1 at N = 4096 gives
  |z| = √2 — unstable.

### 10. The readable-range margin, and why the switch is debounced (2026-07-26)

Added by Prompt P step 2. `BAND_READABLE_HZ = 18.0` shipped as "the 21.53 Hz
alias boundary, kept with a margin" — the boundary is exact
(`fs/(2·HOP)`, and the GUI unwraps once per DSP frame so it is the right one),
but the 3.5 Hz margin was unexplained.

**The margin is the unwrap's noise budget, and it checks out.** A single noisy
hop folds the branch, so the readable limit is `f_hop·(0.5 − z·σ_d)` where σ_d is
the per-hop phase-delta noise in cycles, i.e. the margin is `f_hop·z·σ_d`.
Measured (`strobe_replay` E3, detrended over the GUI's own 25-hop fit window,
piano #1):

| register | σ_d (cycles) | 3σ_d | median max \|Δ\| |
| --- | --- | --- | --- |
| bass 0–23 | 0.0046 | 0.59 Hz | 0.54 Hz |
| mid 24–59 | 0.0180 | 2.32 Hz | 3.00 Hz |
| treble 60–87 | 0.0733 | 9.47 Hz | 11.97 Hz |
| pooled p99.9 | 0.0772 | — | **3.33 Hz** |
| guitar (all) | 0.0031 | 0.40 Hz | 0.41 Hz |

Pooled p99.9 is 3.33 Hz against a shipped 3.5 Hz — so the value is the noise
tail, to 5 %. **Kept.** But note it is a pooled figure over a 16× register
spread: over-conservative in the bass (0.59 Hz would do) and thin in the treble,
where 3σ alone wants 9.5 Hz and the worst capture's detrended delta reaches
0.80 cycles — past the ±0.5 branch, i.e. the treble band folds on its own noise
regardless of any margin. That is the register where the coarse read earns its
keep, and it is already the fallback there.

**The switch needed a debounce, and no margin could have supplied it.** The range
test compares a noisy estimate against a fixed threshold, so while a string sits
*at* the boundary the verdict flips. Measured (`--chatter`, verdicts pooled over
0.9/1.0/1.1× each key's own boundary):

| M (hops of unbroken opposing evidence) | 1 (shipped) | 5 | **8** | 11 | 16 |
| --- | --- | --- | --- | --- | --- |
| mean flip rate, 87 keys | 5.58 % | 0.96 % | **0.38 %** | 0.30 % | 0.17 % |
| worst key | 31.0 % | 4.2 % | **2.4 %** | 1.8 % | 1.8 % |
| keys over 1 % | 62 | 36 | **16** | 11 | 6 |
| added latency | 0 | 93 ms | **163 ms** | 232 ms | 348 ms |

The flip rate tracks ×1.0 of the boundary across boundaries from 7 ¢ (C8) to
179 ¢ (A0) — it is **scale-free in the threshold**, so moving the margin only
relocates the chatter. Hence `READOUT_SWITCH_HOPS = 8`, bracketed by two
quantities the system already fixes:

- **floor 8 hops** — the 8192/1024 window-overlap correlation length. Consecutive
  verdicts share 7/8 of their samples, so 8 hops is the first independent one;
  the measured curve confirms it (the improvement is far slower than the
  independence model's `2^−(M−1)`, and M = 5 still leaves a key at 4.2 %).
- **ceiling ≈ 11 hops** — `BAND_SLOPE_MIN_SPAN_SECS · f_hop`, the band's own fill
  time. Below it the debounce is free on the band-ward switch, because the band
  has no number to show yet.

Cross-instrument: reproducible across piano #2 repeats of the same key (C#5
3.0–8.3 % → 0.0–1.2 %; C#6 4.8–7.1 % → 1.2–1.8 %) and near-absent on the guitar
(0 % on five of six strings), which is why the 2026-07-26 live guitar session did
not expose it — the guitar is the cleanest instrument we have (σ_d 0.0031) and
the session swept detuning rather than resting at a boundary.

**The debounce is symmetric, and that is a weighting, not a derivation.** Holding
the *band* past the boundary can mean displaying a folded number, so the
out-ward direction was re-measured against asymmetric variants — `(m_out,
m_back)` in hops:

| variant | flip% at the boundary | worst | band-held past the alias limit | worst |
| --- | --- | --- | --- | --- |
| none | 5.58 % | 31.0 % | 0 | 0 |
| **8, 8 (adopted)** | **0.38 %** | **2.4 %** | 0.90 % | 12.5 % |
| 1, 8 | 1.35 % | 6.5 % | 0 | 0 |
| 2, 8 | 1.05 % | 4.2 % | 0.16 % | 1.8 % |
| 4, 8 | 0.73 % | 3.6 % | 0.60 % | 12.5 % |

Read the two costs asymmetrically, because they are not alike:

- The flip column is measured at 0.9–1.1× the boundary — 16–20 Hz, **still inside
  the 21.53 Hz alias limit** — so both sources are *correct* there and the flicker
  is pure annoyance with no informational content. It is also where a tuner
  spends time.
- The held column is measured at 2–5× the boundary (36–90 Hz), where the band
  genuinely folds. The exposure is bounded **by construction** at
  `READOUT_SWITCH_HOPS` hops (163 ms) per crossing, once, and self-corrects; the
  0.90 % mean is a fraction of one such episode per dwell.

Symmetric therefore wins on the merits (3.5× fewer flips where the user is
looking, for a bounded episode where the number is changing anyway) and on the
project's own terms: `(1, 8)` and `(2, 8)` need **two** constants, and neither `1`
nor `2` has a derivation, where the single symmetric 8 does.

Only the *range* verdict is debounced. A gated band and an unfilled fit window
are facts, not estimates, and still act on the hop they occur — so "never a stale
number" is intact.

### 11. The band-slope fit window: linear cost, inelastic benefit (2026-07-26)

Prompt P step 3. `BAND_SLOPE_WINDOW_SECS = 0.6`, `BAND_SLOPE_MIN_SPAN_SECS = 0.25`
and `BAND_SLOPE_MIN_SAMPLES = 6` were tuned by feel. Both sides of the trade are
now derived or measured, and they point the same way.

**The lag is exact, and needs no budget.** An OLS fit over a window estimates the
rate at the window's **midpoint**, so a readout fit over `T` lags a turning peg by
exactly `T/2` — a group delay, not a bias to be traded off. At the shipped 0.6 s
that is **300 ms**.

**The jitter side is inelastic in `T`, which is the finding.** The unwrapped series
telescopes (`y_h = Σd_i = φ_h − φ_0`), so its samples carry the phase noise σ_η
with `Var(d) = 2σ_η²`, and textbook OLS gives
`Var(slope) = σ_η²·12/(n(n²−1))` ∝ `T⁻³`. That assumes **independent** samples,
and the bank's windows overlap 75–87 %. Measured (`strobe_replay` E4, pooled
window-to-window scatter in cents at each capture's own displayed reference — an
*upper* bound, since genuine rate drift lands in it too):

| T | hops | lag T/2 | piano bass ¢ | mid ¢ | treble ¢ | vs 0.186 s | guitar ¢ | overlap penalty |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 0.186 s | 8 | 93 ms | 2.45 | 2.04 | 2.08 | 1.00 | 0.055 | 5.9× |
| 0.250 s | 11 | 125 ms | 2.02 | 2.13 | 1.81 | 0.91 | 0.069 | 8.9× |
| 0.400 s | 17 | 200 ms | 1.57 | 2.16 | 2.15 | 0.90 | 0.038 | 14.4× |
| 0.600 s | 26 | 300 ms | 1.06 | 1.61 | 1.03 | 0.56 | 0.050 | 18.2× |

The independent-sample law predicts 0.32 at 0.4 s and 0.08 at 1.0 s; measured is
0.90 and (captures too short). **Tripling the window buys ≈1.8×, where the law
promises 5.8×** — and on the guitar it buys *nothing measurable* (0.04–0.07 ¢,
no trend). The overlap penalty rising 5.9× → 18.2× with `n` is the mechanism:
extra hops are mostly the same audio, so they add little independent information.

So the trade is **linear cost against an inelastic benefit**, and 0.6 s sits at the
expensive end. Two caveats keep this from being a unilateral retune: the 0.6 s row
requires 52 consecutive ungated hops on ~1.3 s captures, which selects the
cleanest captures and biases its figure low; and the absolute piano figures
(1–2.5 ¢) include real rate drift, so they are not a contradiction of this ADR's
±0.2 ¢ band claim — the guitar's 0.05 ¢ confirms that claim for a clean signal.

**Two internal anchors exist if the window is shortened**, both removing a
constant rather than adding one:

- `T = 0.186 s` (8 hops) is the **8192-sample analysis window**, i.e. the coarse
  read's own group delay. At that length the two readouts lag the truth *equally*,
  so switching source (§10) no longer steps the displayed number by
  `(300 − 93) ms × rate`. It is also step 10's window-overlap correlation length.
- `T = 0.25 s` collapses `BAND_SLOPE_WINDOW_SECS` **into**
  `BAND_SLOPE_MIN_SPAN_SECS`: the fit would always run over a full window, and two
  constants become one.

Either choice forces the other constant to follow (a minimum span longer than the
window can never be reached), so both collapse three constants into two.

**Kept at 0.6 s (user, 2026-07-26):** manual mode is responsive enough in use, and
1 ¢ is not below professional relevance — 1 ¢ at A4 is 0.254 Hz, so a 1 ¢ unison
error beats once every four seconds, and beat-nulling resolves far finer than
sequential pitch comparison. Note what the register split says about *where* that
1 ¢ comes from: the guitar — one stable string — sits at 0.04–0.07 ¢ at every
window length, so the estimator itself is sub-0.1 ¢ and the piano's 1–2.5 ¢ is
largely the instrument (decay drift, coupled unison strings beating), which a
longer window delays rather than removes. Escaping the trade instead of moving
along it needs a two-state (g–h / Kalman) estimator on (phase, rate); the anchors
above are the note for anyone who wants the lag back.

**`BAND_SLOPE_MIN_SAMPLES = 6` was a count that meant a ratio — replaced.** Two
points already determine a slope, so it was never a precision floor: it is the
**frame-loss guard**. The GUI accumulates one delta per delivered frame, so a
window can span its time while holding few points if frames were dropped. Its
value had no citation, no ADR and no measurement behind it, and as an absolute
count it would silently become lax if the window shortened.

It is now `BAND_SLOPE_MIN_FRAME_FRACTION = 0.25`, tested against the frames the
*measured span* should have delivered, and derived from what loss costs: for
points spread over a fixed span `Var(slope) ∝ 1/n`, so a quarter of them is
where the slope's standard error has **doubled**. At the shipped window that
reproduces the value it replaces (26 expected × ¼ ≈ 6), it now follows the window
if the window moves, and it trips when the UI falls below ≈ 11 fps against a
43 Hz hop. Two tests pin it, along with the analysis-window floor on the minimum
span.

## Limitations / threats to validity

- **Out-of-range flicker — CLOSED by measurement (2026-07-26).** Recorded here
  as a deferred display-smoothing item; §10 measured it, found it universal
  (33 of 87 keys) rather than treble-specific, and fixed it with the derived
  8-hop debounce. Note this is *not* the item the same day's live check closed:
  that was the motion tail below, a different phenomenon on the one instrument
  where boundary chatter barely appears.
- **Motion tail — CLOSED, tier-2 not needed (live, 2026-07-26).** Offline, p90
  tail errors reach 82 ¢ at 200 ¢/s and 131 ¢ at 400 ¢/s with 4–6 % mixed-source
  churn. The gate on building anything was a live judgement, and it came back
  negative: on a guitar high-E detuned quickly and returned, tracking "works
  reasonably well" and the outliers are "rare enough to not be a bother". So the
  **tier-2 disagreement rule is not built**, and GUI display smoothing /
  median-of-3 is not needed either. The specification stays on record here in
  case a future instrument or a faster peg motion revives it; do not build it
  without new evidence that it is visible.
- **The 17–32-bin bucket at 8192 is 2.7× nominal P_fa.** Bounded, and only ever
  evaluated outside `Silence`.
- **Below ≈ F3 at 2048 the gate is a ratio test, not a P_fa test** (§2). Safe on
  dead notes; the tier-1 rule prefers 8192 there anyway.
- **Ring-out coincidence**: a decaying neighbour can sit in the band. Rare,
  self-correcting, and the same exposure the strobe band already accepts.
  Multi-partial coincidence annotation (≥ 2 ungated references) is a gated
  follow-up.
- n\* rests on two instruments. It is a cross-instrument *disagreement* resolved
  conservatively, which is stronger than a single-instrument optimum, but it is
  still n = 2.
- **Retarget staleness (accepted).** The frontend gates `coarse_cents` on its
  own push succeeding, but the DSP applies the new reference set on its *next*
  hop, so for one or two ticks after a key change a reading taken against the
  old target can be shown against the new reference. Same class as the band's
  angle mirror, self-correcting within a frame or two, and not worth a
  generation echo across crossing #4 to close.
- **Out-of-range flicker (accepted, display-side).** The in-range test needs a
  coarse reading to contradict the band; on a hop where the coarse read is
  rejected while the string is far off pitch, the test falls back to trusting
  the band, which can flash an aliased number for that tick. The superseded
  tracker fallback had the identical shape. It belongs to the deferred display
  smoothing, not to the DSP.
- **The 2048 fallback is not unit-tested.** By design 8192 dominates
  statically, so only *motion* reaches the `or_else` branch, and the evidence
  for it is the offline detune sweep rather than a test. A synthetic chirp test
  would be timing-flaky for what it proves; the coverage boundary is recorded
  instead.
- **ET mode publishes no coarse read below key 16.** Its reference set is the
  fundamental only (`count = 1`), so the n\* = 4 index resolves to nothing and
  the readout shows "listening…". That is deliberate: ET mode carries no
  inharmonicity knowledge by charter, and the n = 1 deep-bass read is measured
  junk (0 % availability at G1). Piano work below C#2 belongs in curve mode.
  **Flagged for a far-future revisit** (user, 2026-07-25): clamping the index to
  the reference count would let ET mode attempt the fundamental instead of
  nothing — safe, since the CFAR gate rejects the junk rather than displaying
  it, and it succeeds occasionally (81 % at B1, 32 % at F1). Not worth the
  behaviour change now; revisit if ET mode ever becomes a real piano workflow.

## Artifacts & reproduction

Harness: `examples/pitch_ground_truth.rs` (kept — the standing instrument).
Modes used: `--readout --detune --bass-partials --gate-ab --span --min-bins
--partial --cfar-profile --pfa --fft --flank-hz --max-n`.

Reference-offset reach (`--reach`, added 2026-07-26) substitutes for detuning a
real instrument: the capture is held fixed and the *reference* is moved, which
the bounded search cannot distinguish from a detuned string. On piano-1 treble
captures the coarse read holds **100 % availability at 0.1–3.1 ¢ error out to
75 ¢** of offset — where the band hands over at 7–36 ¢ — then fails at a cliff
at 100 ¢ (the search-span edge) rather than degrading. This is the measured
statement of the range extension the readout exists for.

Port verification (`--verify-shipped`) compares the harness read against the
shipped `peaks::coarse_read` hop-by-hop at both analysis sizes, on the partial
the shipped rule selects:

| capture set | hops / size | admission agreement | max Δf |
| --- | --- | --- | --- |
| `diagnostics` (guitar, 6) | 338 | 100.0000 % | 0 Hz |
| `diagnostics_piano_1` (87) | 4 492 | 100.0000 % | 0 Hz |
| `diagnostics_piano2` (595) | 31 693 | 100.0000 % | 0 Hz |

Unit coverage: pinned CFAR multipliers and the asymptotic limit, band geometry
(span / bin floor / neighbour cap), tone recovery, noise rejection, neighbour
rejection, withholding without a band; plus two pipeline integration tests
(`coarse_readout_reaches_frame_output`, `coarse_readout_withheld_in_silence`).

## References

1. Rohling, H. (1983). "Radar CFAR Thresholding in Clutter and Multiple Target
   Situations." *IEEE Trans. Aerospace and Electronic Systems* AES-19(4),
   608–621. DOI 10.1109/TAES.1983.309350. Eqs. 9–10 (OS definition), 12
   (exponential-cell pdf), **14** (closed-form finite-N P_fa), **17**
   (linear-detector conversion). In-tree: `resources/tracker/`.
2. Finn, H. M. & Johnson, R. S. (1968). "Adaptive Detection Mode with Threshold
   Control as a Function of Spatially Sampled Clutter-Level Estimates." *RCA
   Review* 29(3), 414–464 — the cell-averaging predecessor. In-tree:
   `RCA-Review-1968-09.pdf`.
3. Kay, S. M. (1998). *Fundamentals of Statistical Signal Processing:
   Detection Theory*, Ch. 9 — the Neyman–Pearson framework the P_fa = 10⁻³
   budget is shared with.
4. Candan, Ç. (2015). Signal Processing 114, 245–250 — the sub-bin refiner
   (`spectral::jacobsen`, faithfulness-audit-03).
5. ADR 0009 — σ_lnB(n) and the B-shrinkage the cold-start references inherit.
6. `docs/design/strobe-and-manual-tuning-ui-design.md` §5.5 / D4 — the readout
   pair this ADR completes.
