# ADR 0015 — The ambient-σ gates, measured

## Status

**Task 1 measured (2026-08-22); the replacement gate is not decided here.**
Sections 1–3 were written and fixed *before* the measurement ran (`07` §1);
§§4–11 are its results, revised the same day after a review pass that added the
known-answer control in §4 and corrected a threshold definition (§4.1). §12 is a
second pre-registration, written before its own run; §13 is its result. What
remains open is listed under **Open for decision**.

Supersedes nothing. This ADR is now the **only** record of the "Neyman–Pearson
partial gate: σ is mis-specified during sustain" entry: `suspected-issues.md` was
retired in commit `52df283`, and the ten references to it elsewhere in the repo
were repointed here. It also **amends
[ADR 0014](0014-unison-panel-against-isolation-truth.md) §8a/§8b**, whose
in-note noise figure §8 below revises.

## Context

Three hot-path detectors threshold against the **same** scalar — the calibrated
ambient-silence RMS held in `config.silence_threshold`:

| # | Site | Quantity tested | Window | Consumer |
| --- | --- | --- | --- | --- |
| 1 | `engine.rs:252` | FFT magnitude bins, `T = √(−σ²·0.375N·ln P_fa)` | 8192 | discovery peaks → TWM → M-of-N lock → every 87-capture baseline |
| 2 | `engine.rs:394` | Goertzel amplitude at the adaptive target, `T = σ·K(n)` | 1024/4096 | tracker partials → `measured_f0` → MAT seed |
| 3 | `strobe.rs:253` | Goertzel amplitude at a fixed reference, `T = σ·K(n)` | 1024/4096 | `band_cents`, the unison ring |

The retired `suspected-issues.md` entry named #2 and #3. **#1 shares the σ and is
added here**: it
is the closest analogue to ADR 0011's control (an ambient threshold on 8192-sample
FFT bins), it is the one that feeds every lock baseline, and — unlike #2 and #3 —
it is a *scan-then-threshold* detector, so ADR 0011 §5's search-loss correction
applies to it and to neither of the others.

### What is already established

- **ADR 0011 §4** — the same ambient threshold, run as a control against FFT bins
  in a bounded search, admitted **100 %** of ±400 ¢ deep-bass garbage; an OS-CFAR
  gate against local reference cells admitted **0 %** at unchanged median accuracy
  and lifted C8 availability 42 % → 100 %.
- **ADR 0014 §8a** — gate #3 measured directly on six 5 s treble captures: the
  threshold sits within a factor of two of the true local noise **at the strike**,
  then real noise falls 13–56× across the note while the signal falls 400–7000×,
  leaving the fixed threshold 7–19× too high by the hop it closes, shutting off a
  partial still 3–15× above the noise beside it.

The unifying statement is **static threshold, dynamic noise**, and the sign of the
error is set by register: leakage-rich bass keeps in-note noise above ambient for
the whole note (under-rejection); near-pure-tone treble lets it fall far below
ambient within a second (over-rejection).

### Why σ itself is not a number this ADR tries to get right

Kay's derivation is sound **given a white** H₀: for white noise the time-domain
per-sample σ is exactly the right per-bin σ after scaling by `Σw² = 0.375N`. Three
things break that here, and they are findings rather than caveats:

1. **Room noise is not white.** It is rumble-dominated — `audio.rs:83` records the
   DC blocker's corner at 35 Hz, *above* A0, and Prompt P measured the coarse read
   getting worse when α was raised because its reference cells reach into that
   band. A broadband RMS therefore over-states per-bin σ where the noise is quiet
   and under-states it where it is loud.
2. **In-note noise is emphatically not white.** It is a harmonic comb's skirts plus
   decay residue — the quantity that moves 13–56× inside one note.
3. **It is a slider.** `Message::SilenceThresholdChanged` (`tuner-gui/src/app.rs`)
   writes straight into the atomics a human can drag mid-session, so the advertised
   P_fa = 10⁻³ is attached to a user-movable control.

`07` §7 already names this case: a threshold-dependent question has no
threshold-free answer, and the way out is to report the family rather than a
crossing. So **σ is swept, not chosen** (§2 below), and every realized rate is
reported as a curve over it with the calibrated point marked. Nothing in the
result depends on picking a correct σ.

## 1. The two quantities, and why they are not the same σ

Per hop and per partial, both read from the freshest samples of the same buffer:

- **S** — the partial's real amplitude. Hi-res peak of a Hann-windowed,
  zero-padded (×4) magnitude spectrum over `N_T = 8192` samples, in the
  `4/N`-normalized physical amplitude units the Goertzel evaluators return. A
  coherent sinusoid's amplitude in these units is **window-independent**.
- **N_local** — the noise actually present beside that partial. Median magnitude
  over the two inter-partial gaps flanking the target, each gap taken from
  `f_{n−1} + L` to `f_n − L` and `f_n + L` to `f_{n+1} + L`, with
  `L = 2·fs/N_T = 10.77 Hz` the Hann main-lobe half-width (null-to-null/2), in the
  same units. Partial frequencies come from the capture's own measured series.

Incoherent noise amplitude scales as `1/√N`, coherent signal does not — that
asymmetry **is** the processing gain, and it is why `K ∝ 1/√n`. So N_local
measured at `N_T` is converted to what a gate's own window would read by

```text
  N_gate = N_local(N_T) · √(N_T / N_win)
```

exact for locally-flat noise (the CFAR homogeneity assumption), and cross-checked
against a direct same-window probe wherever the partial spacing permits one.

**Two SNRs follow, and they answer different questions:**

- `SNR_true = S / N_local(N_T)` — was the partial *physically* sounding, at the
  best resolution available offline.
- `SNR_gate = S / N_gate` — could the gate, in the window it actually uses, have
  been expected to see it.

### Measurability, declared in advance

A gap narrower than **3 zero-padded bins** (4.04 Hz at `N_T` = 8192 ×4) yields no
usable N_local. Where both gaps fail, the row is reported as **unmeasurable** and
excluded from every rate rather than defaulted. At A0 the gap is ≈ 6 Hz — 4.5
zero-padded bins but only ≈ 1.1 *raw* bins — so deep-bass rows are additionally
flagged **resolution-marginal**. This is the same inequality ADR 0011 records for
the coarse read below F3, one octave lower for the longer
window; it bounds what any same-window CFAR can do in the deep bass, which is a
Task 2 input, not a defect of this measurement.

Excluding only the main lobe (rather than a wider margin) leaves Hann sidelobes at
−31 dB inside the gaps. That inflates N_local, which deflates both SNRs, which
makes the gates look **better** than they are — a conservative bias for the defect
claim, in the same direction ADR 0014 §8a argued.

## 2. Pre-registered classification and decision rules

Fixed before the first run.

**Classification**, per (capture, hop, partial), on `SNR_gate`:

| band | label | a gate that PASSES | a gate that REJECTS |
| --- | --- | --- | --- |
| < 1 | **dead** | false alarm | correct |
| 1 … 3 | **ambiguous** | reported, scored in neither rate | reported, scored in neither rate |
| ≥ 3 | **live** | correct | **miss** |

The live cut of **3** is anchored, not invented: ADR 0014 §8a measured the treble
gate closing while partials were still 3–15× above the noise beside them, so 3 is
the bottom of an already-measured range on this project's own data (`07` §2). The
dead cut of **1** is definitional — a cell indistinguishable from its own
neighbourhood. The ambiguous band exists because classification near `SNR_gate = 1`
flips on measurement noise in N_local itself; folding it into either rate would
make the headline sensitive to the cut rather than to the gate.

**H₀ subset.** `P_fa` is a claim about *no signal present*, so the rate compared
against the nominal 10⁻³ is computed on rows that are dead **and** `SNR_true < 1`
— absent even at the best resolution. Rows that are dead but `SNR_true ≥ 1` (present,
below the gate's own noise) are reported separately as **gate-blind**: admitting
one is not a false alarm, but it is not a detection either.

**Reported per gate, per register, per instrument:**

1. realized false-alarm rate on the H₀ subset, against nominal 10⁻³;
2. miss rate on live rows;
3. `T / N_gate` — the threshold against the noise it should be tracking — as a
   distribution, and as a curve against **time-since-onset**;
4. the gate-blind and unmeasurable fractions, so the denominators are visible;
5. what the gate buys: admission rate on true pre-onset silence, and the miss rate
   restricted to dead treble partials during a bass-dominated sustain.

**σ is swept**, not fixed: every rate above is a curve over `silence_threshold`
across at least 1e−4 … 1e−2, with each capture's own recorded calibration marked.
The harness must use the capture's recorded value for the "as shipped" column —
running a value the app never held would measure a gate that does not exist — and
the recorded values differ per set (piano #1 2.909e−3; piano #2 2.788e−3 median,
to 7.481e−3).

**No search-loss correction is applied to gates #2 and #3**, and the writeup must
say why: Rohling's P_fa governs one cell under test, and those two gates test a
single target frequency per partial, taking no argmax. Gate #1 *does* scan, so its
realized rate is expected to exceed nominal on that ground alone and the two causes
must be reported separately.

**Decision gate.** Task 2 (the CFAR port) proceeds only if a gate shows a realized
rate off nominal by more than an order of magnitude in either direction, on **both**
piano #2 sets. If the gates are fine in practice, that is recorded as a refutation
and the entry closes — a cheaper and equally valid outcome.

## 3. Population

Per `06`'s consumption rules; captures are validation-only and cannot select a
configuration (`07` §5).

| set | captures | length | role |
| --- | --- | --- | --- |
| `diagnostics_piano2/` | 595, ≥ 5 repeats/key | 1.5 s | **primary** — register breadth with within-key variance |
| `Piano2_extended` dump | 586, of which 206 at 5 s (A0…C8) | 1.5 / 5 / 10 s | **primary** — the only records long enough for the time-since-onset curve |
| `diagnostics_piano_1/` | 87, one per key | 1.5 s | **secondary, labelled** — the set every lock baseline cites, hence the population gate #1 must be judged on |

The guitar set is **excluded**: at n = 6, one instrument and one tuning, it cannot
establish anything about the register-dependent behaviour that is the whole axis
here.

Both piano #2 sets are consumed through `regenerate_partials`, never raw
`analysis.json`. The extended set's 5 s records are read directly from disk, which
`06` sanctions for questions a 1.5 s record cannot answer, and this section is the
statement that it was done.

---

## Results — Task 1

Measured 2026-08-22 with `examples/pitch_ground_truth --np-set`. Nothing in
§§1–3 was changed after the first run. **Revised the same day** after a review
pass; §4.1 records what the review corrected and why the first numbers were
wrong.

## 4. Calibration — the harness against a case with a known answer

Before any figure below is quoted, the instrument is checked on **AWGN of known
σ = 1.0e−3** with partials 1–4 present and 5–8 declared but absent
(10 synthetic captures, `--np-set awgn`):

| quantity | reads | should read |
| --- | --- | --- |
| `σ_local` (recovered ambient σ) | 1.01–1.04e−3 | 1.0e−3 |
| `T ÷ T_ideal` at the correct σ | 0.96–0.99 | 1.00 |
| dead-partial pass rate, gates #2/#3 | 1.92e−3 (1 of 520) | 1.0e−3 |
| `σ_l@win`, note tail | 1.03e−3 | 1.0e−3 |
| `σ_l@win`, first bucket | 1.30e−3 | 1.0e−3 (**1.26× sidelobe inflation**) |

And the shipped code itself is compared against the harness's replica of it:
`Strobe::process` and the replica agree on **974 000 of 974 000** (hop,
reference) gating verdicts across the three sets — **0.000 % disagreement**, so
gate #3's figures are the shipped code's own. Gate #2 remains a replica (its targets
are the capture's measured partials rather than `predicted_partials · s_win`, and
no lock runs), so its numbers are "the tracker's amplitude test on this audio",
not "the tracker as the engine drove it".

### 4.1 What the review corrected

The control exists because the first revision of this ADR did not have one, and
two errors survived into it:

1. **`N_local` is a *median*; a P_fa = 10⁻³ threshold must sit 3.157× above it.**
   For Rayleigh bins `T/median = √(ln(1/P_fa)/ln 2) = 3.157` — which is
   [`cfar_multiplier`] at the median quantile, the same conversion the coarse
   read's OS-CFAR gate uses. Omitting it read every threshold as 3.157× better
   specified than it is, and produced the sentence "the shipped σ is 0.77× the
   bass's own value — nearly right". **It is ~4× too low there.** The corrected
   ratio is reported as `T ÷ T_ideal`, which the control pins at 1.0.
2. **One column was missing that factor after the fix** (a silent no-op in a
   text substitution), reading `σ_l@win` 3.157× low. Caught by the control, not
   by inspection.

Neither error touched a *ratio* taken within one column, so §7's within-note
figures were unaffected; both touched every absolute level. The register-spread
and spectral-vs-temporal conclusions are unchanged.

## 5. Populations

| set | captures | scored rows | unmeasurable | recorded σ |
| --- | --- | --- | --- | --- |
| `diagnostics_piano2/` | 595 | 323 069 | **0** | 2.7875e−3 (589 caps), 7.4812e−3 (6) |
| `Piano2_extended` | 587 | 620 901 | **0** | median **3.80e−2**, range 3.25e−3 … 9.84e−2 |
| `diagnostics_piano_1/` | 87 | 42 413 | **0** | 2.9089e−3 (all) |

A row is one (capture, hop, partial); 986 383 in total. **No row was
unmeasurable** — the pre-registered ≥ 3-bin gap rule never bit, including at A0.

`Piano2_extended` is the **live** app dump directory and it grew during this
session (586 → 589 `key_*`). Pinned at analysis time: **589 captures,
256 161 640 bytes**, SHA-256 of the sorted `(dir, size)` listing
`8c7af9f4d6c1350ced83ccf5016cfa7d5fe2ed5b4753c3f11c9b67638dd7bfd9`. 587 of them
carried a plausible cached fundamental and were scored.

### σ is not a constant, and that is the first result

The two piano #2 sets are **the same instrument**, and their σ differs by
**13.6×**. 3.80e−2 is a slider position, not a calibration output
(`Message::SilenceThresholdChanged`). On the same gate and the same piano the
realized miss rate goes **22.6 % → 52.7 %**.

## 6. The headline: one threshold, both failures, split by register

Gates #2/#3 at a **fixed** σ = 1e−3, so the register gradient separates from
whatever each session's slider held:

| register | piano #2 P_fa | miss % | extended P_fa | miss % |
| --- | --- | --- | --- | --- |
| bass | **9.29e−1** | 0.3 % | **7.80e−1** | 0.4 % |
| tenor | 0.00 | 14.1 % | 7.70e−3 | 6.9 % |
| treble | 0.00 | 35.7 % | 2.08e−3 | 26.8 % |
| high 76–87 | 0.00 | **56.4 %** | 0.00 | **64.7 %** |

Nominal P_fa is 1.00e−3. **The bass admits 78–93 % of dead partials — 780–930×
nominal — while the top octave rejects 56–65 % of partials standing ≥ 3× above
the noise beside them.** Same threshold, same hop, opposite failures, on both
piano #2 sets; piano #1 secondary agrees (bass 9.47e−1 on 94 H₀ rows,
high-octave miss 40.4 % on 396).

No σ fixes both. Pooled, reaching the advertised 1e−3 costs **~50 %** of live
partials on either set.

**Decision gate (§2): met on both piano #2 sets.**

### Two caveats on that table, both load-bearing

- **Gate #1's P_fa is not measurable this way and is reported as N/A.** H₀ is
  defined as a lobe maximum below a gap median *in the 8192 spectrum*, which is
  the very spectrum gate #1 thresholds — so every H₀ row is a guaranteed
  rejection. Gates #2/#3 read a different window, which is what makes their
  column a real dead-partial pass rate (control: 1.9e−3 against 1e−3 nominal).
- **The H₀ population is small and selected** (94–3 506 rows against 10⁴–10⁵
  live rows). It admits only quiet-lobe realizations — on white noise, 6 % of
  absent rows. The bass figures are real and reproduce across sets, but the
  well-powered half of this study is the **miss** rate, and the properly powered
  false-alarm measurement is the silence test in §10.

### The pre-registered live cut of 3 is too low, and the sweep says so

A correctly specified gate sits at 3.157× the noise median — so the cut of 3
admits partials such a gate rejects about half the time, conflating stranding
with the detection limit. Sweeping it (exploratory, `--np-live`), gate #2:

| set / register | ≥3× | ≥6× | ≥10× | ≥20× | ≥50× |
| --- | --- | --- | --- | --- | --- |
| piano #2 treble | 49.5 % | 43.7 % | 39.0 % | 29.2 % | 13.4 % |
| piano #2 high 76–87 | 70.3 % | 57.7 % | 49.4 % | 39.1 % | 16.6 % |
| extended treble | 76.8 % | 72.7 % | 69.2 % | 60.8 % | 44.8 % |
| extended high 76–87 | 83.1 % | 65.5 % | 59.6 % | 48.6 % | 33.3 % |

The conclusion survives and the magnitude does not: **39 % of top-octave partials
standing 20× above their own local noise are still rejected** on piano #2, and
49 % on the extended set. The pre-registered figure is ~1.8× inflated. ADR 0014
§8a's "3–15×" was not a clean anchor for this cut, because its noise estimate was
itself inflated (§8).

## 7. Rates against time since onset (Task 1 item 2)

Miss rate at the shipped σ, gate #2, piano #2 — the *rates* the task asked for,
not only the threshold ratio:

| register | .1–.25 s | .25–.5 s | .5–1 s | 1–1.5 s |
| --- | --- | --- | --- | --- |
| bass | 0.5 % | 1.0 % | 1.0 % | 2.0 % |
| tenor | 13.2 % | 17.3 % | 26.3 % | 34.4 % |
| treble | 37.3 % | 47.8 % | 51.5 % | 51.7 % |
| high 76–87 | 60.2 % | 67.8 % | 76.3 % | 56.8 % (n = 126) |

The gate does degrade as the note sustains — the claim the entry made — but it
**starts** degraded: the top octave is already missing 60 % of live partials in
the first quarter-second. The time axis is the smaller effect, which §8 measures
directly.

## 8. σ_local, and the correction that matters most

`σ_local` is the ambient σ that would place the threshold exactly at the
P_fa = 10⁻³ point for the noise measured beside the partial. Piano #2:

| register | .1–.25 s | .25–.5 s | .5–1 s | 1–1.5 s | `T ÷ T_ideal` span |
| --- | --- | --- | --- | --- | --- |
| bass | 1.14e−2 | 8.12e−3 | 4.91e−3 | 3.68e−3 | **0.26 → 0.81** |
| tenor | 8.04e−4 | 3.52e−4 | 2.56e−4 | 2.28e−4 | 3.47 → 12.20 |
| treble | 2.96e−4 | 1.73e−4 | 1.45e−4 | 1.41e−4 | 9.42 → 19.80 |
| high 76–87 | 2.03e−4 | 1.52e−4 | 1.37e−4 | 1.73e−4 | 13.70 → 20.41 |

The shipped σ is **2.7875e−3**, and it is right **nowhere**: ~**4× too low** in
the bass at the strike and ~**16–20× too high** in the top octave. The
calibration is not broken — it measures a broadband RMS, which room noise loads
at the bottom of the compass (`audio.rs:83` records the rumble that makes it so)
— and then applies that one number flat across a spectrum that is not flat.

### The defect is **spectral**, not temporal

| term | span |
| --- | --- |
| **across registers**, at one instant | **21× (1–1.5 s) to 56× (.1–.25 s)** |
| **within one note**, strike to tail | **1.4–4.7 %** → see below |

Within-note, from `σ_l@win` (the gate's own 23 ms window, which resolves the
attack the 186 ms probe cannot see), after removing the control's measured 1.26×
first-bucket sidelobe inflation: piano #2 bass **1.9×**, tenor **4.7×**, treble
**3.3×**, high **1.4×**. On ADR 0014 §8a's own six keys and 30 captures:
treble **2.9×**, high **2.2×**.

Not 13–56×. Three things make that hard to attribute to this probe:

1. **An independent control.** The same probe on **pre-onset silence** at the
   same frequencies agrees with the note's own tail within **0.51–2.10×** on
   every register of all three sets. The tail has already reached the room floor,
   which is why it stops falling.
2. **Two probe windows agree** where the noise is incoherent — `σ_local` and
   `σ_l@win` match to 1–3 % in tenor and above from 0.25 s on.
3. **The gaps exclude both lobes**, so what leaks in *inflates* N_local and would
   mask a fall rather than manufacture flatness — and the control quantifies that
   inflation at 1.26×.

**Why this reverses one of Prompt O's own conclusions.** The prompt ruled out a
startup per-bin spectral floor because "nothing measured before the strike can
follow" the in-note swing. That rests on the temporal term dominating. It does
not: it is smaller by roughly an order of magnitude, and the control shows the
room floor a startup measurement *would* capture sits within a factor of two of
the in-note noise for most of a note's life. A per-bin startup floor addresses
the 21–56× term and leaves 1.4–4.7×. That does not make it the best answer — a
local-reference CFAR gets both terms and needs no stored state — but it can no
longer be dismissed a priori, and it is far cheaper.

**With one qualification the prompt would have been right about**: the residual
concentrates in the **first ~0.5 s**, which is exactly the epoch gate #1 and the
M-of-N vote run in. A startup floor would be 2–5× too low there.

## 9. Structural findings

**The window rule cannot help — where the noise is incoherent.** `T ÷ T_ideal` is
identical at every window length by identity: `K(N) ∝ 1/√N` and incoherent noise
also falls as `1/√N`, so the two scalings cancel. Measured columns agreed to the
printed precision in tenor and above.

**But it does not hold in the bass**, and the earlier claim that it holds
everywhere was an overclaim. There the inter-partial gaps (6–20 Hz) sit inside
neighbouring skirts, so `N_local` is *coherent leakage*, which does not scale as
`1/√N` — the two probe columns diverge by up to **1.8×** (extended bass tail:
4.11e−3 vs 7.19e−3). That is precisely the register where R3's long window and
ADR 0011's reference-cell floor were needed, and it means **no same-window
reference can measure noise in the deep bass at 1024 or 4096 at all**. Any Task 2
design must say what the bass does instead.

**Gate #1 belongs in the entry.** Discovery's peak floor (`engine.rs:252`) is the
same threshold in disguise — `√(−σ²·0.375N·ln P_fa)` is exactly `σ·K(N)` once the
`4/N` normalization is applied. It shows the same register split (bass
P_fa 8.02e−1, high-octave miss 74.7 % at σ = 1e−3) and feeds TWM → the M-of-N
lock → every 87-capture baseline. The retired entry named two gates; there
are three.

## 10. What the gates buy — one credit inverted, one refuted

**True-silence rejection: 0.00 % admissions**, on 10 258 (hop, partial) probes
drawn from pre-roll hops passing the Gatekeeper's silence test — 7 556 extended,
2 425 piano #2, 277 piano #1, all four registers, zero in every cell.

**This is not the virtue the entry took it for.** Nominal P_fa = 1e−3 predicts
≈ 10 admissions; observing 0 (p ≈ e⁻¹⁰) is the *same over-rejection*, seen from
the silent side. The bar a replacement must clear is **P_fa ≤ nominal on
silence**, not zero.

The screen is load-bearing: scored over *all* pre-roll hops the figure is
**23–28 %**, because the pre-roll ring holds whatever was played before and these
sets were captured key by key. The unscreened number measures the previous note's
decay.

**Refuted: dropping dead high bass partials.** On piano #2 at its own σ, gates #2
and #3 **admit 19.1 % / 22.1 %** of dead bass partials at n ≥ 6 (68 rows). The
extended set shows 0.0 % of 895, but only because its σ sits 13.6× higher — the
same slider that costs it half its live partials.

**Also measured:** the tracker's non-physical-`f_inst` guard fired independently
of the amplitude test on **0 rows** in every set.

## 11. Threats to validity

- **n = 2 instruments.** Validation only (`06`, `07` §5). These figures
  characterize a defect; they cannot select a replacement.
- **Gate #2 is a replica**, verified only by its agreement with gate #3 (they
  differ by ≤ 0.1 pt everywhere, suggesting the adaptive centre is inert on these
  captures — inferred, not measured). Gate #3 is byte-faithful to the shipped
  bank; gate #1 is a per-cell stand-in for a detector that scans.
- **Every register figure is a median** over keys, strike strengths and repeats
  pooled. Per-key spread is not reported.
- **H₀ is small and selected** (§6) — the miss rates are the better-powered half.
- **Below 186 ms the 8192 probe is blind**; attack figures come from the gate's
  own window, which carries the control's 1.26× inflation and is undefined in the
  bass.
- **The silence screen uses instantaneous RMS**, not the Gatekeeper's EMA — looser
  than shipped, so the 0.00 % is if anything conservative.
- **`σ_local` carries Hann-sidelobe inflation** (§1), so every SNR here is a
  slight under-estimate. Conservative for the claim, not neutral.

## Open for decision

1. **Whether Task 2 proceeds, and to what.** The decision gate is met, so a
   replacement is warranted. §13 narrows the option set rather than widening it:
   the static-floor family is out on a structural ground, leaving a
   local-reference gate — which §9 says cannot serve the bass, where no
   off-partial frequency exists at 1024 or 4096. **A design must answer the bass
   separately**, and the honest options there are a longer window (R3 already
   does this), accepting an uncalibrated ratio test (what ships today), or
   leaving the bass on the coarse read, which already has a working CFAR.
2. ~~The per-bin startup floor is now scoreable offline.~~ **Measured — §12/§13.**
   It passes on register shape (spread 2.5–3.5× against A's 21–56×) and fails its
   false-alarm guard structurally. No core change was made.
3. ~~References to the deleted `suspected-issues.md` dangle.~~ **Resolved
   2026-08-22.** All ten were repointed here — the two harness doc-comments
   rewritten, and the prose in ADRs 0011, 0012 and 0014 kept in its historical
   tense ("then recorded in…") with a link to this ADR appended, so the record of
   what was believed when is preserved while the reference resolves.

## 12. Pre-registration — Candidate C, the per-bin startup floor

Written **before** the run, per `07` §1. §8 claims the dominant error term is
spectral and therefore reachable by a measurement taken before the note begins.
That claim is tested here on the shipped consumers rather than on `σ_local`.

**Candidate C.** One floor per *set*, not per capture: the per-bin median
magnitude of the 8192 ×4 probe spectrum over every silence-screened pre-roll hop
in that set, pooled across all its captures. Applied as `T_C = 3.157 · floor(f_n)`
— the same median → P_fa conversion everything else uses. This stands in for the
calibration's existing 2 s recording, which the app already takes and currently
reduces to one scalar.

**Compared at the same operating point against:**

| | threshold | `T ÷ T_ideal` |
| --- | --- | --- |
| **A** — shipped | `σ_rec · K(N)` | measured, 0.26 … 20.4 |
| **B** — oracle | `3.157 · N_local(this hop)` | ≡ 1 by construction |
| **C** — startup floor | `3.157 · floor(f_n)` | the question |

B is the bound a perfect local-reference CFAR approaches; it is not implementable
as such (§9: the bass has no same-window reference cells), and is here to scale
the comparison.

**Primary criterion, fixed now.** C passes if **both**:

1. its compass-wide spread of median `T ÷ T_ideal` — max ÷ min across the four
   registers — is **< 5×**; and
2. it beats A on median `|log₁₀(T ÷ T_ideal)|` in **every** register.

The 5× is anchored, not chosen: §8 decomposes the error into a 21–56× spectral
term and a 1.4–4.7× temporal one. A startup measurement can only address the
former, so its residual should land at the latter's scale; 5× is the top of that
measured range, rounded up.

**Disqualifying guard.** C's admission rate on **held-out** silence must be
≤ 1e−3. Because the floor is built *from* silence, testing on the same hops would
be circular, so pre-roll hops are split by parity: **odd-indexed hops build the
floor, even-indexed hops score it.** A floor that over-admits on silence it has
not seen fails regardless of criterion 1.

**Exploratory, reported but not deciding:** miss and dead-partial rates under C,
and the residual's distribution against time since onset — §8 predicts it
concentrates in the first ~0.5 s, which is where the M-of-N lock runs.

**What a pass does and does not mean.** It would establish that a cheap, already-
collected measurement removes the dominant term — it would **not** select C for
the hot path. n = 2 instruments cannot select (`07` §5), and `CLAUDE.md`'s
faithful-ports principle prefers a cited method (Rohling OS-CFAR) over a bespoke
assembly on equal evidence. The realistic outcome of a pass is a **hybrid**
worth designing: a startup floor where no same-window reference exists (the bass),
a local reference where one does.

## 13. Result — Candidate C **fails its guard**, and the reason generalises

**Criterion 1 passes, decisively.** The per-bin startup floor removes almost all
of the register term. Median `T ÷ T_ideal` (1.0 is correct), and median
`|log₁₀(T ÷ T_ideal)|` on the identical rows:

| set | register | A shipped | **C floor** | \|log₁₀\| A | \|log₁₀\| C |
| --- | --- | --- | --- | --- | --- |
| piano #2 | bass | 0.59 | 0.36 | 0.43 | **0.44** |
| | tenor | 10.51 | 0.77 | 1.02 | 0.12 |
| | treble | 18.21 | 0.92 | 1.26 | 0.07 |
| | high 76–87 | 18.79 | 1.27 | 1.27 | 0.23 |
| extended | bass | 5.69 | 0.41 | 0.76 | 0.39 |
| | tenor | 50.54 | 0.68 | 1.70 | 0.20 |
| | treble | 77.36 | 0.87 | 1.89 | 0.11 |
| | high 76–87 | 18.47 | 1.03 | 1.27 | 0.11 |

Compass spread of median `T_C ÷ T_ideal`: **3.50× (piano #2)** and **2.52×
(extended)**, against A's 21–56×. Criterion 1's bar was < 5×. **§8's claim that
the dominant term is spectral, and reachable before the strike, is confirmed on
the shipped consumers.**

**Criterion 2 fails in the bass**, on two sets of three: C is no better than A
there (piano #2 0.44 vs 0.43; piano #1 0.84 vs 0.40). Only the extended set shows
C ahead in every register. The bass is where §9 already said no same-window
reference exists — it is unhelped by either approach.

**The guard fails on every set, at every quantile tested — and that decides it.**
Held-out silence admission against a ≤ 1e−3 budget:

| floor quantile | piano #2 | extended | piano #1 |
| --- | --- | --- | --- |
| median × 3.157 (pre-registered) | 4.24e−2 | 1.17e−1 | 2.40e−2 |
| 0.99 | 4.49e−2 | — | — |
| **0.999** | **7.78e−3** | **1.02e−2** | **5.60e−2** |

Best case is **7.8× over budget**, and it does not improve with a tighter
quantile because it **cannot**:

> The floor was built from **256 / 668 / 40** silence hops. The finest tail an
> empirical per-bin quantile can resolve is `1/N` — **3.9e−3 / 1.5e−3 / 2.5e−2**.
> The measured held-out rates land exactly at that resolution limit. Calibrating a
> 1e−3 per-bin false-alarm rate from a startup recording would need ≳ 10⁴ hops,
> about **four minutes** of silence, against the 2 s the calibration takes.

That is a **structural** limit, not a tuning failure, and it is the sharpest thing
this section produces:

> **A startup floor pools samples across *time* — many hops at one bin. A
> local-reference gate pools across *frequency* — many bins in one window. Only
> the second has the sample count a 1e−3 tail needs in the time available.** And
> Rohling's OS-CFAR does not estimate the tail empirically at all: its P_fa comes
> from the *analytic* order statistic (Eq. 14), which is why ~32 reference cells
> suffice where 10⁴ hops would not.

**A second, independent reason the static family fails**, visible in the same
table: at the pre-registered median × 3.157 the realized rate is 42–116× nominal,
which is the Rayleigh conversion being wrong for this data — real room noise has a
**heavier tail than Rayleigh** (non-stationary: HVAC, distant sound, handling). A
static floor of any shape inherits that; a same-window reference re-measures it
every hop and does not.

### What this leaves

| candidate | register term (21–56×) | calibrated P_fa | the bass |
| --- | --- | --- | --- |
| A — shipped scalar | ✗ | ✗ (0 on silence, 780–930× in-note) | ✗ |
| C — per-bin startup floor | **✓ (to 2.5–3.5×)** | ✗ structurally | ✗ |
| CFAR local reference | ✓ (ADR 0011) | ✓ analytic | **no reference cells exist** |

No candidate covers the bass. That is now the sharpest open question for Task 2,
and it is a *resolution* problem rather than a threshold one — the same inequality
`N > 4·fs/f₀` that ADR 0011 recorded below F3.

**Prompt O's original instinct was right, for a reason it did not give.** It ruled
out a startup floor because "nothing measured before the strike can follow the
note". The decay is *not* what defeats it — that term is only 1.4–4.7×, and C
absorbs the far larger register term. What defeats it is that a 2 s recording
cannot calibrate a 10⁻³ tail. The conclusion stands; the argument for it is
different, and the difference matters, because it says the fix is **more reference
samples per decision**, not **fresher ones**.

## Artifacts & reproduction

```bash
cargo run --release --example regenerate_partials -- diagnostics_piano2 > p2.json
cargo run --release --example regenerate_partials -- <Piano2_extended dump> > p2ext.json
cargo run --release --example regenerate_partials -- diagnostics_piano_1 > p1.json
cargo run --release --example pitch_ground_truth -- \
  --np-set piano2 p2.json diagnostics_piano2 \
  --np-set piano2-extended p2ext.json <Piano2_extended dump> \
  --np-set piano1-secondary p1.json diagnostics_piano_1
# calibration and the live-cut sensitivity sweep
cargo run --release --example pitch_ground_truth -- --np-set awgn <synth>/regen.json <synth>
cargo run --release --example pitch_ground_truth -- --np-live 20 --np-set piano2 p2.json diagnostics_piano2
```

The extended set is not in the repository; `06` says where it lives, and §5 pins
the population it had at analysis time.

## References

- Kay, S. M. (1998). *Fundamentals of Statistical Signal Processing: Detection
  Theory*, Ch. 9 — the threshold whose σ is at issue.
- Rohling, H. (1983). Radar CFAR thresholding in clutter and multiple target
  situations. *IEEE Trans. AES* 19(4) — the local-reference alternative, and the
  median → P_fa conversion §4.1 restores.
- [ADR 0011](0011-coarse-spectral-readout.md) — the CFAR port that ships in the
  coarse readout, and the search-loss lesson that applies to gate #1 alone.
- [ADR 0014](0014-unison-panel-against-isolation-truth.md) §8a/§8b — the first
  direct measurement of gate #3, and the figure §8 above revises.
