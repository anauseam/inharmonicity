# Unison assist — design note (superseded by ADR 0012; kept as the derivation trail)

**Status: BUILT 2026-08-07.** What shipped, and the numbers the Rust port itself
produces, are in **[ADR 0012](../adr/0012-unison-line-estimator.md)** — that is
the citable record. This note is kept for what an ADR does not carry: the
alternatives considered and rejected, and the offline measurement phase
(2026-08-05, all three capture sets) that settled the estimator's shape before
any hot-path code was written. The `sequential-detection-design.md` → ADR 0010
pattern.

**Three of the figures below did not survive the port, and the ADR supersedes
them.** Read them here as what the offline model produced, not as what the app
does:

- **E-A's resolution law.** The port's transition is sharp, not gradual: at
  40 dB the outcome is essentially deterministic in the split. More importantly,
  E-A asked only whether two lines appeared and never whether they were in the
  right place: at or below `2/T` the reported split collapses onto the limit
  itself rather than the truth, and it is unreliable either way out to ≈1.6 ×
  (ADR 0012 §4).
- **E-B's −26 dB sensitivity.** True nearly everywhere, but not flat: below
  −12 dB the limit moves out from `2/T` to ≈1.6 × `2/T`. Strings within 6 dB of
  each other resolve at the geometric limit itself. ADR 0012 §4 carries the
  surface.
- **E-K's bass reproducibility of 11 %.** The shipped estimator reproduces the
  bass at 1 %, like the tenor. The bass-is-not-a-unison conclusion stands on its
  other two legs and should no longer be argued from this one (ADR 0012 §7).

The **discriminator** in §3 and §8 was built as specified and measured wrong; the
shipped test is a different one (ADR 0012 §6).

Supersedes the sketch in
[`strobe-and-manual-tuning-ui-design.md`](strobe-and-manual-tuning-ui-design.md)
§7.4, which proposed an **envelope-beat** readout. §7.4's physics is right and
its conclusion is wrong: the same data supports a strictly better measurement.
§7.4 now points here.

---

## 1. What the feature is

A multi-string note must have its strings zero-beat against each other. Today
that is done entirely by ear — the one genuine tuning need the app does not
serve (README, `TODO.md`).

The feature resolves the **individual strings** of the selected note as separate
spectral lines, each with a signed offset from its curve target, so the display
can show a marker per string converging on the target. The beat rate a tuner
listens for falls out as the difference between any two lines.

This is the dominant design in the modern field, and it was missed by the §12
survey: **TuneLab's Spectrum Display** ("each string of a unison produces its own
peak"; the documented workflow is to tune one string, watch which peak moves, and
walk it onto the centre line), **PianoMeter's Peak View** (zoomed harmonic peaks,
bold centre line at target, dotted ±10 ¢, outer ±100 ¢, green pitch-raise target;
sold for muteless pitch raises, treble tuning, and **diagnosing false beats**),
and **pianoscope 4.0**'s spectrum display. §12 needs an addendum recording this.

## 2. Why not the envelope-beat route §7.4 proposed

Two routes to the beat from identical data. **From the lines:** estimate each
string's frequency, subtract. **From the amplitude modulation:** watch only the
Goertzel *magnitude*, which swells and fades at |f₁ − f₂|, and estimate that
periodicity (envelope-spectrum analysis — the standard machinery, e.g. Randall &
Antoni 2011).

The magnitude route discards the phase, and the phase is what carries the
**sign** of each offset and the **positions** of the strings. It also collapses a
three-string note's three pair-beats into one real signal, and needs a decay
detrend which is itself an error source. Both routes hit the same ≈2/T resolution
wall, so the magnitude route buys nothing for what it gives up.

One asymmetry survives in the magnitude route's favour and is worth recording:
the **split** between two lines is immune to baseband aliasing (both fold
identically), while the absolute positions are not. If a pitch-raise readout of
unison spread outside ±21.5 Hz is ever wanted, that is the mechanism.

## 3. The estimator, in detail

Per reference *i* — one curve target frequency `f_ref` from
`TuningCurve::strobe_partials`, i.e. one entry of the reference set `Strobe`
already holds.

### Step 1 — keep the complex Goertzel value

Each hop the strobe already evaluates a Hann-windowed Goertzel at `f_ref` over
the newest `W` samples (`W` = 1024, or 4096 in the deep bass by the existing R3
rule) and returns `(amplitude, phase)`. That pair is a complex number; today the
phase feeds the band angle and the amplitude feeds the D3 gate, and they are
never kept together. Keep

```text
    x[h] = A[h] · e^{ j·φ[h] }
```

### Step 2 — demodulate to baseband

`x[h]`'s phase advances by `2π·f_ref·H/f_s` per hop from the reference alone
(`H` = `HOP_SIZE`). Removing it is exactly the `expected` term already computed
in `Strobe::process`:

```text
    z[h] = x[h] · e^{ −j·2π·f_ref·h·H/f_s }
```

so each string contributes a phasor turning at its **offset** from the target:

```text
    z[h] = Σ_k  a_k · e^{ −h·H/(f_s·τ_k) } · e^{ j(2π(f_k − f_ref)·h·H/f_s + θ_k) }  +  noise
```

A sum of damped complex exponentials, sampled once per hop at
`f_hop = f_s/H = 43.07 Hz`, unambiguous over **±21.53 Hz** around the target.

**This is a zoom FFT** (Lyons, *Understanding DSP*, ch. 13): the Hann-windowed
Goertzel is the mixer and anti-alias filter, the hop is the decimator. It buys
the resolution of a ~57 000-point DFT of the raw audio, restricted to the band we
care about, for about a thousandth of the work. Resolution is set by
**observation time, not bin count** — no FFT of a single 8192-sample frame can
separate strings 1.7 Hz apart, and zero-padding does not change that.

### Step 3 — ring

A fixed-size per-reference ring holds the last `N` samples of `z`. A hop the D3
amplitude gate rejects breaks the run and restarts the ring — the same
hold/restart semantics `BandSlope` already uses, and for the same reason.

### Step 4 — transform

Hann-window the ring and take an `N`-point **complex** DFT, no zero-padding:

```text
    Z[m] = Σ_h w[h]·z[h]·e^{ −j2πmh/N }
```

Bin spacing is `1/T = f_hop/N` (0.77 Hz at N = 56). Natural Fourier bins are used
deliberately: zero-padded bins are interpolated rather than independent, and the
CFAR null below assumes independence.

### Step 5 — candidates

Local maxima of `|Z|`, taken **circularly** — the baseband spectrum wraps, so bin
0 and bin N−1 are neighbours and there are no edge cases. Sorted by magnitude.

### Step 6 — Rayleigh merge

Reject any candidate within **2 bins** of an already-accepted stronger one. Two
bins is the Hann main-lobe half-width, i.e. `2/T` Hz. Components closer than that
are one line; reporting them as two is a measured failure mode (§5).

### Step 7 — CFAR admission

Per candidate, reference cells are a **sliding local window**: bins at circular
distance 3…18 from the cell under test (guard 2 bins = the main lobe of the cell
itself; 16 reference cells per side, 32 total — the same order as Rohling's own
`N = 24…32`). Take the order statistic at rank `⌊0.50·(N_ref − 1)⌉` and threshold

```text
    T = noise · cfar_multiplier(N_ref/2, rank/2, P_fa / m_eff)
```

reusing the existing Rohling Eq. 14 + 17 port in `peaks.rs` (audit 13). The
halvings for Hann correlation and the `m_eff` search-loss divisor follow
`coarse_read`'s calibrated pattern — this detector also takes an argmax, so the
ADR 0011 §5 correction applies. Candidates are magnitude-sorted, so the first
rejection ends the list.

**The reference geometry is *not* `coarse_read`'s** and that difference is
load-bearing — see §5.

### Step 8 — sub-bin refinement

Per admitted line, the three-bin Candan/Jacobsen estimator on the complex bins:

```text
    δ    = c_N · Re[ (Z[m−1] − Z[m+1]) / (2·Z[m] − Z[m−1] − Z[m+1]) ]
    f_off = (m/N)·f_hop + δ·(f_hop/N)
```

`c_N` for a Hann window at these short lengths is **2.050 at N = 56** and 2.103
at N = 28 — the shipped `candan_bias_correction` table holds only 2048/8192 and
falls back to 2.0, which would be a 2.4–5 % scale error on every offset. A new
match arm is required.

### Step 9 — output

Up to 3 lines per reference: **signed offset in Hz** from `f_ref`, plus relative
amplitude, plus the ring's current resolution `2/T`. Hz not cents, per the
crossing-#2 rule — the frontend owns the reference a number is displayed against.

### The discriminator

For each partial *n* with ≥2 lines, the split in cents is
`1200·log₂(1 + Δf_n / f_ref,n)`. A **unison** is two strings at different f₀, so
both strings' partials scale together and this is *constant in n*. A **false
beat** is a mode splitting of a single partial and has no reason to be.

It is a χ² goodness-of-fit test, not a threshold: fit a constant across the
partials that resolved, and test the residuals against the estimator's own line
σ (E-B: ≈0.05 Hz per line ⇒ ≈0.07 Hz per split, converted to cents at each
partial) at a stated P_fa. Same shape as the project's other gates.

**It runs DSP-side** and ships its verdict over crossing #2. Not for cost — it is
arithmetic on a dozen numbers — but because it is a *detector over signal
estimates*, and Prompt R's rule is that estimators live in `tuner-core` while the
GUI owns display policy. Deciding whether a split is a unison is not formatting.
Keeping it in the core also unit-tests with the estimator and serves a future
headless consumer ([`01`](../internals/01-architecture.md)).

## 4. Why the strings beat at all, and what a false beat is

Two strings at slightly different f₀ beat — that is a unison. But each partial of
a *single* string also exists as **two orthogonal transverse polarizations**
(perpendicular and parallel to the soundboard). They are degenerate on an ideal
string and split on a real one, because the bridge termination has different
mechanical impedance in the two directions. String defects (a kink, corrosion,
uneven winding, a poorly seated bridge pin or agraffe) do the same, and a
soundboard mode sitting very close to a partial splits it by avoided crossing.
That is a **false beat**: one string beating with itself. It is not
inharmonicity, which shifts a partial's frequency but never splits it.

Coupled strings also do not beat like two independent oscillators — Weinreich
(1977) shows the coupling produces energy exchange and a double decay, so the
beat is not stationary over a long window.

## 5. The measurement phase (2026-08-05)

Method: a Python model mirroring the above, run against all three capture sets.
Piano #2 consumed through `regenerate_partials` per
[`06-capture-sets.md`](../internals/06-capture-sets.md). The model's numerical
calibration of `c_N` reproduces the shipped Rust table to six decimals
(2.001325 vs 2.001329 at N=2048; 2.000331 vs 2.000332 at N=8192).

**What counts as truth.** On real captures there is none at the splits that
matter: any reference DFT sees the same 1.3 s and so has the same resolution
limit. Real-capture work therefore validates *implementation* and *consistency*;
every **accuracy** claim rests on synthetic signals, which are rendered as audio
and pushed through the same Goertzel front end so the window, the decay and the
strike are in the loop.

### The two corrections the data forced

**(a) CFAR reference geometry.** The first version copied `coarse_read`'s
geometry — reference cells from flanks *outside* the search band. That excludes
the dominant line's own skirt, so a secondary maximum riding that skirt was
compared against distant background and passed. False splits on a *single*
synthetic string: 7.5 % at 56 hops, **20.8 % at 86 hops**, and **26.7 %** with a
fast decay — worse at high SNR and longer windows, the signature of a
deterministic artefact rather than noise. Replacing it with a **sliding local
window with main-lobe guard cells** gives **0 false second lines in 2 160 null
trials** across SNR 6–40 dB, decay 0.15–1.5 s and 30–86 hops.

**(b) Rank.** The sliding window then failed on three strings — the other strings
fill the reference window and the order statistic estimates signal as noise
(0/3.0/6.0 Hz detection fell 100 % → 0 %). This is Rohling's §V interference
criterion binding from the other side, the same criterion that forced
`COARSE_CFAR_QUANTILE = 0.25` in `coarse_read`. Sweep:

| excise peaks | W | q | null | det 2 | det 3a | det 3b |
| --- | --- | --- | --- | --- | --- | --- |
| no | 8 | 0.25 | 0 % | 100 % | 100 % | 100 % |
| no | 8 | 0.50 | 0 % | 100 % | 100 % | 100 % |
| no | 8 | **0.75** | 0 % | 100 % | **0 %** | **0 %** |
| no | 16 | 0.50 | 0 % | 100 % | 100 % | 100 % |
| yes | 16 | 0.75 | 0 % | 100 % | 100 % | 100 % |

Exactly one cell fails and it is the one violating §V (2 interferers × 5 lobe
cells = 62 % of 16 references ⇒ k/N ≤ 0.38). W = 16 with q = 0.50 satisfies it
with margin; excising the other candidates' lobes and using the paper's own
q = 0.75 is equivalent within noise across a 10-case stress set. **Take the
simpler one.**

### E-A — resolution law (synthetic, exact truth)

P(two lines resolved); two equal strings, τ 1.5 s, SNR 40 dB, 40 trials/cell:

| split Hz | 0.46 s | 0.65 s | 0.93 s | 1.30 s | 2.00 s | 3.00 s |
| --- | --- | --- | --- | --- | --- | --- |
| 0.7 | 8 % | 18 % | 20 % | 15 % | 40 % | **100 %** |
| 1.0 | 12 % | 15 % | 15 % | 28 % | **88 %** | 100 % |
| 1.5 | 10 % | 20 % | 42 % | **57 %** | 100 % | 100 % |
| 2.0 | 5 % | 25 % | 57 % | **100 %** | 100 % | 100 % |
| 3.0 | 30 % | **68 %** | 100 % | 100 % | 100 % | 100 % |
| 5.0 | **100 %** | 100 % | 100 % | 100 % | 100 % | 100 % |
| *2/T floor* | *4.31* | *3.08* | *2.15* | *1.54* | *1.00* | *0.67* |

**50 % detection at ≈ 2/T, 100 % at ≈1.3–1.4 × 2/T.** `2/T` is therefore the
honest number for the display to state as its current resolution.

### E-B — accuracy where it resolves (synthetic, 1.30 s)

| case | P(2) | split bias | split σ | position error |
| --- | --- | --- | --- | --- |
| equal, 2.0 Hz, SNR 40 | 100 % | +0.022 Hz | 0.046 | 0.050 |
| equal, 2.0 Hz, SNR 15 | 100 % | +0.008 | 0.057 | 0.057 |
| equal, 2.0 Hz, SNR 6 | 100 % | +0.002 | 0.053 | 0.053 |
| second string −20 dB | 100 % | +0.003 | 0.023 | 0.020 |
| fast decay τ 0.4 s | 100 % | +0.011 | 0.052 | 0.053 |
| split decay 1.5/0.4 s | 100 % | +0.004 | 0.023 | 0.016 |

Bias ≤ 0.02 Hz, σ ≤ 0.06 Hz, essentially SNR-independent down to 6 dB.
Sensitivity reaches a second string **26 dB below** the first at ≥1.3 s.

### E-H — does 1024:1 decimation lose anything?

Zoom estimator vs a full-rate zero-padded DFT of the *identical* span:

| set | register | cases | median \|Δf\| | p90 |
| --- | --- | --- | --- | --- |
| piano1 | bass | 215 | 0.0155 Hz | 0.077 |
| piano1 | tenor | 191 | 0.0278 | 0.141 |
| piano1 | treble | 97 | 0.0500 | 0.288 |
| piano2 | tenor | 1721 | 0.0392 | 0.187 |
| piano2 | treble | 824 | 0.0657 | 0.233 |
| piano2 | high 76–87 | 93 | 0.1497 | 4.570 |

Nothing is lost except in the top octave, which fails for an unrelated reason
(E-M).

### E-I — availability per register

| set | register | cases | ≥1 line | ≥2 | ≥3 | gated hops |
| --- | --- | --- | --- | --- | --- | --- |
| piano1 | bass | 216 | 100 % | 71 % | 34 % | 1 % |
| piano1 | tenor | 192 | 99 % | 80 % | 38 % | 10 % |
| piano1 | treble | 120 | 81 % | 62 % | 39 % | 57 % |
| piano1 | high 76–87 | 23 | 30 % | 26 % | 13 % | 78 % |
| piano2 | tenor | 1728 | 100 % | 82 % | 49 % | 14 % |
| piano2 | treble | 1086 | 76 % | 64 % | 48 % | 61 % |
| piano2 | high 76–87 | 315 | 30 % | 16 % | 7 % | 89 % |

### E-K — repeat reproducibility (piano #2, ≥5 strikes/key)

Truth-free: if the estimator measures the instrument rather than noise,
independent strikes must agree.

| register | keys | median split | median MAD across repeats | relative |
| --- | --- | --- | --- | --- |
| bass | 27 | 33.03 ¢ | 3.48 ¢ | 11 % |
| **tenor** | 24 | 4.39 ¢ | **0.08 ¢** | **2 %** |
| **treble** | 16 | 4.24 ¢ | **0.04 ¢** | **1 %** |

**0.04–0.08 ¢ across independent strikes** in the tenor and treble. This is the
strongest evidence in the set.

### E-J / E-L — the discriminator, and the bass

| set | register | captures ≥3 partials | median split | spread across n | "constant" (<25 %) |
| --- | --- | --- | --- | --- | --- |
| piano1 | tenor | 24 | 4.18 ¢ | 26 % | 46 % |
| piano1 | treble | 11 | 3.59 ¢ | 15 % | 64 % |
| piano2 | tenor | 214 | 4.24 ¢ | 17 % | 67 % |
| piano2 | treble | 125 | 4.04 ¢ | 12 % | 63 % |
| piano1 | **bass** | 27 | **24.56 ¢** | **33 %** | 30 % |
| piano2 | **bass** | 137 | **32.46 ¢** | **51 %** | 19 % |

Null bands, where a second line cannot be a unison:

| band | cases | ≥2 lines | median split | spread across n |
| --- | --- | --- | --- | --- |
| piano keys 0–11 (single-strung) | 72 | **100 %** | 28–51 ¢ | 40–51 % |
| piano keys 12–27 (bichord) | 97 | 94–100 % | 17–28 ¢ | 29–54 % |

**The bass produces second lines essentially always and they are not unisons.**
No piano has bass unisons 25–50 ¢ apart, and keys 0–11 are single-strung. Three
independent facts agree: the splits are not constant in cents (33–51 % spread vs
12–17 % in the tenor), they reproduce only to 11 % across repeats vs 1–2 %, and
they occur where there is only one string. **E-N** rules out the obvious
artefact: only 3–22 % of second lines land within 1 Hz of where a neighbouring
partial would fold after decimation, and the median split (3.3–4.5 Hz) is nowhere
near the median fold (7.4–17 Hz).

What they *are* is not established — see §7.

### E-M — the real bound on ring length

The D3 gate breaks the ring, so the longest unbroken **ungated run**, not the
capture length, bounds T:

| set | register | median run | seconds | ≥56 hops |
| --- | --- | --- | --- | --- |
| piano1/2 | bass | 57 hops | 1.32 | 99–100 % |
| piano1/2 | tenor | 57 hops | 1.32 | 94–96 % |
| piano1/2 | treble 52–75 | 27–28 hops | 0.63–0.65 | 17–25 % |
| piano1/2 | **high 76–87** | **0 hops** | **0.00** | **0 %** |

The top octave never opens the gate — its partial amplitude sits below
`noise_floor · K(1024)`. That is the same gate the band uses, so the band is
already frozen there today and the coarse read carries that register.

### E-O — ring policy

28 hops (the treble's measured limit) vs 56, same captures:

| set | band | availability 28 → 56 | median split 28 → 56 | per-capture \|Δ\| |
| --- | --- | --- | --- | --- |
| piano1 | tenor | 71 % → 100 % | 7.96 → 6.02 ¢ | 2.67 ¢ |
| piano2 | tenor | 83 % → 98 % | 9.69 → 5.37 ¢ | 4.87 ¢ |
| piano1 | treble | 100 % → 100 % | 5.72 → 4.71 ¢ | 1.61 ¢ |
| piano2 | treble | 100 % → 100 % | 5.71 → 4.34 ¢ | 1.22 ¢ |

The short ring is **biased high** (survivorship — close pairs merge, only wide
ones get reported) and less available. Longer is better on both axes within the
measured range.

### E-Q — the 1024-sample window is under-filtered for a 1024-sample hop

Taking one Goertzel output per hop *is* a decimation, so the window is the
anti-alias filter. Alias-free decimation by `H` needs the Hann half main lobe
`2·f_s/N` below the baseband Nyquist `f_s/(2H)`, i.e. **N > 4H**:

| N | half main lobe | vs 21.53 Hz Nyquist |
| --- | --- | --- |
| 1024 | 86.1 Hz | **aliases** |
| 2048 | 43.1 Hz | **aliases** |
| 4096 | 21.5 Hz | critically filtered |

So the treble path (1024 window, 1024 hop) is **4× under-filtered**: content
21.5–86 Hz from the reference folds back essentially unattenuated. At C#4's 2nd
partial a semitone is 32 Hz, so a neighbouring key sounding lands squarely in the
fold zone. Confirmed: an interferer at +30 Hz produced a spurious line within
1 Hz of its predicted fold (−13.07 Hz) in 100 % of trials.

The 4096 window is the theoretically correct filter — its first null sits exactly
at the baseband Nyquist — but it did **not** remove the spurious line in the same
test (Hann sidelobes still admit a strong out-of-band interferer). Only presence
was measured, not the folded line's *strength*, which is what decides whether it
out-ranks a genuine string. **The window choice is therefore an open measurement,
not a settled fix.**

### E-R — erroneous lines, matched against the full-rate DFT

Every zoom line matched to the full-rate DFT's peaks by nearest neighbour; a line
with no full-rate peak within 0.5 Hz is unexplained:

| set | register | lines | matched | unmatched | median \|d\| |
| --- | --- | --- | --- | --- | --- |
| piano1 | tenor | 416 | 88 % | 12 % | 0.039 Hz |
| piano1 | treble | 219 | 95 % | 5 % | 0.049 |
| piano2 | tenor | 3976 | 88 % | 12 % | 0.048 |
| piano2 | treble | 2039 | 91 % | 9 % | 0.063 |
| piano2 | bass | 2133 | 78 % | **22 %** | 0.020 |
| piano2 | high | 164 | 79 % | **21 %** | 0.097 |

**5–12 % of lines in the feature's home register are unexplained**, and about a
quarter in the bass and top octave. Matched lines agree to 0.02–0.06 Hz. This is
an upper bound on the zoom's error rate — the full-rate reference used a crude
local-max picker capped at three peaks, so some "unmatched" lines may be real
peaks its own picker missed.

### E-S — availability once a gated hop breaks the ring

E-I ran on unbroken rings. With restart semantics enforced (longest surviving
run, minimum 20 hops):

| set | register | run ≥20 | ≥1 line | ≥2 | ≥3 |
| --- | --- | --- | --- | --- | --- |
| piano1 | tenor | 100 % | 100 % | 73 % | 32 % |
| piano2 | tenor | 99 % | 99 % | 67 % | 28 % |
| piano1 | treble | 53 % | 53 % | 51 % | 34 % |
| piano2 | treble | 52 % | 52 % | 47 % | 26 % |
| piano2 | high | 7 % | 7 % | 6 % | 3 % |

The restart costs roughly 10–15 points of ≥2-line availability in the tenor and
treble against E-I's unbroken-ring figures.

### E-T / E-U — the window question, settled against the theory

E-Q's decimation argument says the 4096-sample window is the correct anti-alias
filter and 1024 is 4× under-filtered. Two measurements were run to act on that.

**E-T — is aliasing what produces the unexplained lines?** Tenor and treble,
both windows, scored on the E-R unmatched rate:

| set | band | window | lines | unmatched | ungated hops |
| --- | --- | --- | --- | --- | --- |
| piano2 | tenor | 1024 | 1762 | 6 % | 98 % |
| piano2 | tenor | 4096 | 1977 | 6 % | 99 % |
| piano2 | treble | 1024 | 1486 | 6 % | 58 % |
| piano2 | treble | 4096 | 1552 | 5 % | **67 %** |
| piano1 | treble | 1024 | 176 | 3 % | 56 % |
| piano1 | treble | 4096 | 196 | 2 % | **63 %** |

**Hypothesis refuted:** the window barely moves the unmatched rate, so aliasing
is *not* what produces the unexplained lines. It does buy real availability —
7–9 points more ungated hops in the treble, from the halved Neyman–Pearson
threshold a longer window earns.

This run also corrects E-R: allowing the full-rate reference five peaks instead
of three roughly halves the unmatched rate (12 % → 6 % in the tenor). **Most
"unmatched" lines were peaks the crude reference picker had not listed**, so the
estimator's true error rate is nearer 5–6 %.

**E-U — the null in the 4096 configuration**, which had never been tested:

| case | want | 1024 @30 | 1024 @56 | 4096 @30 | 4096 @56 |
| --- | --- | --- | --- | --- | --- |
| null, τ 1.5 s, SNR 40 | 2 | 0 % | 0 % | 0 % | 0 % |
| null, τ 0.4 s, SNR 40 | 2 | 0 % | 0 % | 0 % | 2 % |
| **null, τ 1.5 s, SNR 15** | 2 | **0 %** | **0 %** | **45 %** | **4 %** |
| 2 strings 2.0 Hz | 2 | 34 % | 100 % | 38 % | 100 % |
| 3 strings 0/2.0/4.5 | 3 | 0 % | 100 % | 0 % | 100 % |

**The 4096 window breaks the null** at short rings and modest SNR — 45 % false
second lines. Its 75 % overlap makes consecutive baseband samples correlated, and
correlated reference cells are exactly what the CFAR threshold assumes away.

**Decision: keep the 1024 window.** The theoretical argument and the availability
measurement both favoured 4096 and the null test refutes it. The failure is
diagnosable rather than fundamental — with 75 % overlap the effective independent
reference count is ≈N/4, not the N/2 the Hann-correlation halving assumes — so
4096 could be revisited by re-deriving that factor and re-running this table. It
is not free, and it is not needed for v1.

### E-P — a mechanism that does not earn its place

**Decay compensation** (flattening the exponential before transforming, to undo
Lorentzian line-broadening) is inert once the CFAR geometry is right: identical
null, no synthetic accuracy benefit, and unchanged real-data availability
(67/67 %, 75/76 %, 87/86 % by register). Do not build it.

## 6. Design consequences

1. The approach works: 2–3 strings resolved, agreement with a full-rate DFT to
   ~0.03 Hz, reproducibility of 0.04–0.08 ¢ across strikes in tenor and treble.
2. CFAR uses a **sliding local reference window with main-lobe guard cells**,
   W = 16, q = 0.50 — *not* `coarse_read`'s flanking geometry.
3. Rayleigh merge at 2 bins; the display states `2/T` as its current resolution.
4. The ring grows to a cap; publish from the point the Rayleigh criterion is met.
   **This makes the readout latency depend on the answer**: a 4 Hz split resolves
   at T ≈ 0.5 s, 2 Hz at ≈1.0 s, 1 Hz at ≈2 s, because resolving two lines δ
   apart needs T ≳ 2/δ. Far-apart strings give fast feedback; the endgame is the
   slow part. Note the two latencies differ — the *position* of the strongest
   line is available almost immediately, and only the "how many strings, how far
   apart" question waits.
   **Hazard:** before the ring is long enough, two separated strings report as
   **one line**, which reads as "clean". The display must therefore always carry
   its current resolution ("clean to ±3 ¢"), or it will actively mislead at
   exactly the moment the tuner is deciding they are done.
5. When the panel is unavailable the feature degrades to the existing workflow —
   the band and coarse read still show pitch against target, so the tuner tunes
   one string with a mute and finishes the unison by ear. That is the field's own
   documented practice (TuneLab and PianoMeter both prescribe it), not a failure
   mode. Three levers could shrink the unavailable region and are untested: a
   re-strike hint (the ring restarts on the gate, so a firmer blow buys hops),
   the 4096-sample window's ~6 dB in the treble (E-Q), and Prompt O's rework of
   the D3 gate whose ambient-σ misspecification is what closes the top octave.
6. **The discriminator has to ship** — without it the bass display would label a
   false beat as a unison on essentially every bass key of both instruments.
7. Register scope: tenor and treble are the feature's home; the top octave
   resolves to "unavailable" on its own, with no key threshold.
8. `candan_bias_correction` needs a match arm for the ring length.
9. Estimated cost: 12 references × one ~56-point complex FFT ≈ 2.3 k butterflies
   per hop, against ~24.6 k for the existing 8192-point bass FFT — of order
   **10 %** of one transform already in the budget. Plus a 32-element order
   statistic per candidate. Ring memory 12 × 56 complex f32 ≈ 5.4 KB.
   **Estimated by operation count, not measured** (§7).

## 7. Placement, and why the latency does not reach the pipeline

**Latency is not blocking.** The 1.3 s is *observation* time — how much signal
must accumulate before the answer exists — not the duration of any operation.
Per hop the work is bounded and small: 12 complex pushes, plus one ~56-point
transform per reference. The real-time constraint is per-hop execution time
against the 23.2 ms callback, and that is untouched by how long the ring has been
filling.

**It is a tap** by [`01`](../internals/01-architecture.md)'s deletability test:
it reads a target the UI nominated and writes only `FrameOutput`. Nothing in the
chain consumes it, so gating, detection and measurement stay bit-identical. That
is a reviewable property, not an assurance.

**The ring needs no persistence.** It is a fixed array on the component, reset on
retarget or a gate break. It also does **not** need to enter the capture dump:
the ring is *derivable* from `audio.raw` by recomputing the Goertzel offline,
which is exactly how the measurements in §5 were produced. No new dump format.

**Three coherent placements**, in preference order:

1. **All on the DSP thread** (recommended). Cost is of order 10 % of one FFT
   already in the budget. If measurement says otherwise, the transform can be
   decimated — consecutive estimates share (N−1)/N of their data and are
   therefore near-identical, so re-running every hop buys nothing a reader can
   see. Derive any such stride from that correlation, do not pick one.
2. **Ring on the DSP thread, transform on the Worker.** The Worker is
   capture-triggered, not continuous, so this needs a new streaming crossing —
   `02` §6's reuse test and a documented charter. Heavy; the escape hatch only if
   (1) measures badly.
3. **Ship the ring over crossing #2 and transform in the GUI.** Rejected: `01`
   is explicit that `app.rs` holds no DSP. Worth recording that the usual
   objection does *not* apply — a ring snapshot is complete, so a dropped frame
   costs one update rather than destroying an accumulated quantity — but the
   layering rule decides it regardless.

Two things to measure in the port, neither of which a butterfly count captures:
µs per hop in `--release` for the whole bank, and the cache behaviour of 12 rings
(12 × 56 complex f32 ≈ 5.4 KB touched per hop). `FrameOutput` already carries
16 KB of magnitudes, so the payload is not the concern; the scattered per-hop
writes might be.

## 8. Limitations

Ordered by how much they could change the design.

- **The bass attribution is inferred, not proven.** Established: the bass second
  lines are real spectral content, not aliased neighbouring partials, not
  constant in cents, and poorly reproducible. *Not* established: what they are.
  Candidates, none excluded — polarization false beats (wound strings are
  notorious); **longitudinal string modes** (Conklin), which land at frequencies
  unrelated to the transverse series; **sympathetic resonance from a neighbouring
  key**, which in the deep bass is devastating as a confound because a semitone
  at 55 Hz is only ~3.3 Hz and therefore lands *inside* the ±21.5 Hz baseband —
  the same suspicion Prompt E was queued to investigate; soundboard/bridge mode
  coupling; or a genuinely wide bichord on these neglected instruments.
- **The 4096-sample bass configuration was never tested synthetically.** Every
  null and resolution trial used the 1024 window, which is critically sampled at
  the hop rate. The deep bass uses `goertzel_bass` with the same 1024 hop, so its
  baseband is 4× oversampled and its noise correspondingly correlated — exactly
  the assumption the CFAR null rests on. This is a plausible contributor to the
  bass behaviour above and must be tested before any bass claim.
- **The reference is assumed correct.** All real-capture runs used the *measured*
  partial frequency as `f_ref`, which centres the baseband. In the app `f_ref` is
  the curve target and an out-of-tune string sits off-centre; past ±21.5 Hz the
  lines alias. The reference-offset sweep was not run.
- **The discriminator needs a test, not a threshold.** The 25 % figure used for
  reporting is an undrifted constant and does not meet
  [`04`](../internals/04-algorithms-and-models.md)'s bar. The analytical form is
  available: under the unison hypothesis the cents split is constant across n, so
  fit a constant and test the residuals against the estimator's own measured line
  σ (E-B: ≈0.05 Hz per line ⇒ ≈0.07 Hz per split, converted to cents per partial)
  as a χ² goodness-of-fit with a stated P_fa — the same shape as the project's
  other gates, and no magic number.
- **Unison state is unrecorded, and 1.5 s cannot supply it.** Nothing in the
  capture sets says how far apart any note's strings actually were — not a
  measurement, not even an aural note; `06-capture-sets.md` records deviation
  from ET and is silent on unisons. Nor can it be recovered from the audio:
  1.5 s bounds *any* method to ≈2/T = 1.33 Hz, which at C#4's 2nd partial is
  4.2 ¢. That covers the feature's working range but not the endgame, where a
  set unison is well under 1 ¢ — so the captures cannot distinguish "clean" from
  "out of resolution", which is the one distinction the display must be honest
  about. Resolving 0.5 Hz needs ≈4 s; 0.2 Hz needs ≈10 s. **The fix is longer
  recordings, not better estimation** — plus a mute test (record one string, then
  two, then open), which measures each string individually and is decisive.
- **Two instruments, both out of tune.** Per the n = 2 rule these captures
  validate; they cannot select a configuration.
- **1.5 s captures cap T at 1.3 s**, and the treble's own sustain caps it at
  0.65 s. Any ring cap above 1.3 s is an extrapolation from synthetic signals,
  which are stationary by construction — precisely the assumption at risk.
- **Non-stationarity is unmodelled.** Real strings' pitch falls in the first tens
  of ms after the strike, and Weinreich coupling exchanges energy between strings
  over the note. Synthetic tones have neither.
- **Coupled-string physics is unmodelled.** The synthetic three-string cases are
  independent exponentials, not coupled oscillators.
- **`MAX_LINES = 3` is assumed.** Candidates are magnitude-sorted and the CFAR
  loop stops at the first rejection, so the cap means "the three strongest
  admitted lines" — right for a three-string unison. The unexamined case is a
  false beat *on top of* a three-string unison, where the three shown could mix
  strings with a false-beat partner.
- **≈5–6 % of reported lines in the tenor and treble are unexplained** (E-T,
  correcting E-R's 12 %), and ~20 % in the bass and top octave. Aliasing is
  ruled out as the cause (E-T); what remains is unattributed. The strongest line
  is reliable; the weakest of three is the one to qualify.
- **Cost is an operation count, not a Rust measurement** in `--release` against
  the 23.2 ms callback budget.
- The guitar set is out of scope for this feature (single strings, six captures).

## 9. Open decisions

*Every item below was closed by the build; ADR 0012 records how. Kept as the
list of what was genuinely open at the end of the measurement phase.*

- Module and struct naming (`strobe/unison.rs`, `Unison`) — note the file
  resolves *lines*, and a false beat presents identically, so the doc comment
  must state what it measures while the name states what it is for.
- Display: markers on a **cents** axis (per the existing convention — positions
  in cents, rates in Hz) with the beat rate in Hz as the numeric readout.
- ~~Whether the discriminator's verdict is user-visible~~ — **decided
  2026-08-06: visible.** A later "advanced mode" toggle may suppress it.
- ~~Whether the panel is hidden above key 75~~ — **decided 2026-08-06: shown as
  unavailable, and with no key threshold at all.** Availability is a runtime
  condition — the ring has not filled enough to meet the Rayleigh criterion —
  not a register rule. In the top octave that resolves to "unavailable" almost
  always (E-M: median longest ungated run of 0 hops; E-S: ≥1 line in 6–14 %),
  which is the same outcome a key constant would give, without the constant.
  A register constant here would also be the exact mistake ADR 0011 §7 and the
  Prompt O note warn against.
- Bass policy is deferred to the bass-attribution prompt, not decided here.
- The ring cap value, and whether to raise it above 1.3 s once longer captures
  exist.
- `FrameOutput` payload: lines for all 12 references (12 × 3 × 2 f32 ≈ 288 B) or
  a subset.
- ~~Whether to select the 4096-sample window in the treble~~ — **decided
  2026-08-06: no, keep 1024** (E-U). Revisitable only by re-deriving the CFAR's
  effective reference count for a 75 %-overlapped baseband.
- **DFT size.** The measurements used a 56-point transform, i.e. the natural
  Fourier bins of the record. A power-of-two ring (64 hops = 1.49 s) would allow
  a radix-2 transform but was not measured; zero-padding 56 → 64 is *not*
  equivalent, because padded bins are interpolated rather than independent and
  the CFAR null assumes independence. Either measure 64, or use an arbitrary-size
  mixed-radix transform (`rustfft` supports 56 = 8 × 7). The `rustfft` planner
  must be constructed once at startup and held on the component — the hot path
  allocates nothing.

## 10. References

- Rohling, H. (1983). *Radar CFAR Thresholding in Clutter and Multiple Target
  Situations.* IEEE Trans. AES-19(4). — the admission gate; §V is the criterion
  that sets the rank. Already ported in `peaks.rs`; audit 13.
- Candan, Ç. (2015). *Fine resolution frequency estimation from three DFT
  samples: case of windowed data.* Signal Processing 114. — the refiner; already
  ported in `spectral.rs`; audit 3.
- Lyons, R. (2010). *Understanding Digital Signal Processing*, ch. 13, "The Zoom
  FFT". — the demodulate-decimate-transform structure.
- Weinreich, G. (1977). *Coupled piano strings.* JASA 62(6). — why unisons beat,
  and why the beat is not stationary.
- Conklin, H. A. (1996). *Design and tone in the mechanoacoustic piano.* JASA —
  longitudinal modes, a candidate for the bass lines.
- Randall, R. B. & Antoni, J. (2011). *Rolling element bearing diagnostics — a
  tutorial.* MSSP 25(2). — the envelope-spectrum route this note does not take.
