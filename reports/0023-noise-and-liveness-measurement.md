# report 0023 — noise and liveness: the five consumers' gates, measured against known truth

**Phase I complete; Phases II and III pending.** Part 1 is the pre-registration:
the measurement protocol as it was accepted and fixed by hash before the first
run (SHA-256 `c22e7508089df37246be09fb80a10261f7e60db09b3c70059c8d7ab0b55bd05d`
over the rulebook, `0f152decf85e602f98348d0a75b156aca8826c26ac4c4172d7abd4418cc2e71e`
over its one-page overview, frozen 2026-10-07T21:23:07Z). Nothing in Part 1 was
changed after a number existed; a change the work forced is a numbered
amendment in Part 2 §6, with its reason. Part 2 is Phase I: the harness and the
truth builders, their calibration on a control with a known answer, and the
room's per-bin distribution read from the data that already exists. Nothing on
real notes is scored for a decision here; Phase II does that, and Phase III
runs the lock replay, the cost bench and the decision tables.

Two things in this report are worth copying. The control is run **before any
real number**: every harness mode must reproduce nominal on white noise of known
σ, and the figure it reproduces — 9.0 × 10⁻⁴ false candidates per bin at a
10⁻³ per-bin budget, the Hann geometry's own local-maximum rate — was predicted
by a synthetic derivation and is now a measurement of this port. And every
candidate carries its **exact null** rather than a Rayleigh formula: a floor
that is itself an estimate is thresholded by the finite-N law, a scan carries a
per-cell budget, and a test on phases is calibrated against simulated noise at
the window it runs on.

---

# Part 1 — the pre-registration

## 0. The question, per consumer

The earlier work asked "what is the noise a detector compares against", which
presupposes a noise estimate. Five consumers need different things, and only two
of them need a level:

| Consumer | Where it runs | What it decides | What it needs |
| --- | --- | --- | --- |
| **G1** discovery's peak picker | every hop, 8192-point Hann frame, no key named; strict local maxima above a threshold, the 64 strongest | which maxima are partials, over all bins, within a per-hop budget | a per-bin **decision** with a known false-peak rate; a floor is one way |
| **G2** the tracker's partial gate | every hop, Hann Goertzel at the adaptive target, 1024 or 4096 samples | is this partial still alive | a per-partial **decision**; a level is one way, a signal-side test another |
| **G3** the strobe's reference gate | as G2, at fixed references; a hard gate that holds the angle | as G2, and how much to trust the hop's phase | a decision or a **weight** |
| **BG** the background measurement (MAT) | one 2¹⁶ window at 0.67 Hz per bin, opened ≈ 116 ms after onset | which located peaks are partials, and the floor they are read against | a **level** per bin or valley, and an acceptance rule |
| **LK** the future likelihood | offline, then wherever a probabilistic estimator runs | the noise's per-bin law under the conditions | a **distribution**, with its tail and stationarity |

Conditions: (a) dense sustained lines; (b) the strike; (c) a background whose
level moves; (d) frequency-resolved; (e) a short calibration. Registers as
report 0015: bass (keys 0–27), tenor (28–51), treble (52–75), high (76–87).
Times since onset as report 0015: 0.1–0.25 s (the strike's tail, where discovery
and the M-of-N vote run), 0.25–0.5, 0.5–1, 1–1.5 s.

Two outcomes the earlier protocols did not allow are allowed here. For G2 and G3
a signal-side test that meets the bars stands on equal footing with the
estimators: the gate's *decision* may rest on it, with a floor beside it only
for the SNR that G3's weight and LK consume. And the shipped coarse read's
OS-CFAR is one comparison arm among others, not the reference design.

## 1. What the record already establishes

Inputs, not re-scored and not counted as confirming themselves.

- **The shipped rule.** One scalar, the silence threshold — 1.5 × the loudest
  calibration RMS or the operator's slider — scaled by Kay's factor at all three
  gates (`Engine::process`'s peak floor and tracker gate, `Strobe::process`).
  Two sessions of one piano held values 13.6× apart (report 0015 §5). As a
  per-bin σ it is ≈ 4× too low in the bass and 16–20× too high at the top; the
  register term is 21–56×, the within-note term 1.4–4.7× (0015 §8). The bass
  admits 78–93 % of dead partials; the top octave rejects 56–65 % of partials
  standing ≥ 3× above the noise beside them (0015 §6). On Gatekeeper-silent hops
  it admits 0 of 10 258 probes where nominal predicts ≈ 10: over-rejection, not
  a virtue (0015 §10).
- **A per-bin startup floor** (the per-bin median of pooled silence-screened
  pre-roll × 3.157) removes the register term — compass spread 2.5–3.5× — and
  fails its held-out silence guard at every quantile, 7.8× nominal at best: an
  empirical per-bin tail cannot resolve below 1/N from 256/668/40 hops, and
  pooled silence has a heavier tail than Rayleigh (0015 §13). What failed is the
  stored tail, not the per-bin shape.
- **The room's floor is where a note's tail ends up.** The pre-onset probe agrees
  with the note's own tail within 0.51–2.10× in every register (0015 §8); the
  residual concentrates in the first ≈ 0.5 s, 2–5× above the floor.
- **The bass.** At 8192 points A0's partials are 5.11 bins apart, 75 % of a
  reference window lies in a lobe and the selected cell is a lobe in 56–75 % of
  hops (audit 13); the gaps hold coherent leakage, so no same-window reference
  can measure noise in the deep bass at 1024 or 4096 at all (0015 §9).
- **The shipped CFARs** are exact in their law and calibrated as composites:
  realised AWGN rate 0.00097–0.00102 with a search-loss budget P_fa/M (report
  0011 §5; audit 13). The first revision of report 0015 omitted the median →
  P_fa conversion and read every threshold 3.157× better than it was (0015 §4.1).
- **BG's products.** From A6 up, 24–27 % of located partials sit at exact
  multiples of f₁ — nonlinear products of the chain or the instrument — and one
  poisons K − 1 of MAT's pairs; the open lever is MAT's ±f₀/4 search band
  (report 0009 Analysis 7). The attack is inert for B (0009 Analysis 9).
- **Nothing observes the room.** Under the silence threshold the Gatekeeper
  returns early and zeroes its metrics; the harness must compute what the hot
  path does not.
- **Synthetic derivations, provisional until this port reproduces them**: the
  finite-N multipliers (Rohling 1983 Eq. 14; the Student-Rayleigh law of a floor
  averaged over N periodograms, Talbot & Thrane 2020); the Hann power correlation
  4/9 between adjacent bins and an effective bin count n/1.944; ν/2 = 20.8
  equivalent periodograms in a 79-frame calibration; the median's conversion
  ln 2 and the rank shift under signal; the F-test's exact null F(2, 2K − 2);
  the reciprocal-mean floor's constant 0.90 and density bias 1/(1 − p_eff); the
  one-periodogram false-tone rates of the tonality standards' rules (≈ 7 × 10⁻²)
  and of a published median peak test (≈ 2 × 10⁻² per maximum at a nominal
  10⁻³).

## 2. Data, truth and baselines

### 2.1 Data

- `diagnostics_piano2/` (595 captures, 1.5 s; primary), the extended store (5 s
  and 10 s records for the time axis; the mute-isolation set for the bass),
  `diagnostics_piano_1/` (87; secondary). Consumed through `cargo lab mat regen`,
  never the cached `analysis.json`; the 1.5 s rule for anything compared with
  reports 0009–0011; longer records read from disk and said so
  (`capture-sets.md`). The guitar set is excluded as in 0015 §3.
- **Silence**: each capture's pre-roll, screened by the Gatekeeper's EMA verdict
  — the shipped screen, not the harness's instantaneous RMS (0015 §11) — and
  any room recording made through the app's own stream (`cargo lab audio record`,
  DC-blocked, into a `silence_*/` directory). The existing sets were recorded
  without one, so their screened pre-roll stands in. One or two 2-minute room
  recordings per room, when convenient, serve the level's wander over seconds and
  minutes, a pooled tail at 10⁻³, and a direct test of whether 2 s is enough for
  the calibration (2 s windows cut from the record against the whole; expected
  ±1 dB per bin at 2 s, falling as 1/√T). They are not a prerequisite.
- **Hold-outs by time blocks**, never by hop parity: adjacent hops share seven
  eighths of their samples.
- **Confirmation instruments.** Report 0020 converted five external piano reads
  into the capture format: the Iowa Steinway B (close-miked), the MAPS
  Disklavier at 3–4 m and at 50 cm, and RWC's Yamaha, Bösendorfer and Steinway.
  They are validation data like the rest, read with piano #2, not after it
  (§6): every in-note quantity on every set; silence false alarms only where the
  pre-roll is a room (piano #2, MAPS ambient and close; not RWC, whose pre-roll
  is edited digital silence). One read of one instrument is exploratory.

### 2.2 Truth

No single truth serves every consumer and register, so each quantity names the
truth it is scored against.

- **T1 — the AWGN control.** White Gaussian noise of known σ with partials
  present and partials declared absent (0015 §4), one run per harness mode: the
  realised rate must land on nominal before a real number is quoted.
- **T2 — known partials in real noise.** Synthetic partials — the stiff-string
  series f_n = n·f₀·√(1 + Bn²) with the key's B, each capture's own measured
  amplitudes, exponential decays, optionally two or three strings per partial —
  added to a real noise bed, so the noise under and beside every partial is known
  exactly at every hop. Two beds: **T2a**, the room bed (screened pre-roll or a
  room recording), right for the sustain and for the leakage mechanics; **T2b**,
  the in-note bed (a real capture minus an offline sinusoidal model of its
  partials, Serra & Smith 1990's deterministic-plus-stochastic decomposition),
  which holds what silence lacks — hammer noise, decay irregularity, sympathetic
  strings, products, the room — at the cost of sinusoidal remnants, whose
  spectrum and coherence at each partial's frequency are reported before use.
  T2 is synthetic in its signal, so it cannot decide alone; the generator is
  never tuned to match the captures.
- **Fidelity of T2, once and pre-registered.** Every parameter the generator
  draws comes from a statistic the record has published, fixed before any
  estimator is scored; the synthetic set is compared with the real captures,
  once, on the quantities the estimators see, per register: the amplitude
  profile against partial number (0009 Analysis 7: the bass radiates more
  through its upper partials, n = 2 at +8 dB; the treble through f₁, n = 2 at
  −28 dB); the per-bin SNR of live partials and the share of dead ones (0015's
  harness); decay's within-note term and onset excess (0015 §8); beating from
  the isolation set (report 0014 §2–3); line density (exact by construction);
  the strike's first-bucket excess; the two-stage decay and the decay rate's
  rise with partial number (the extended set's long records); dynamics (0009's
  0.5–7.8 dB spread); products (exploratory); T2b's leftover. And one global
  check: **the shipped gates, run on T2, must behave as they do on the real
  captures** — the same false-alarm and miss rates by register and bucket within
  the record's scatter (0015 §6, §7). On a failure the generator's ranges are
  re-drawn from the record and the check is run again; the estimators are not
  consulted.
- **T3 — the record's oracle on real notes.** N_local, the median magnitude over
  the two inter-partial gaps at 4× zero-padding, from the capture's own measured
  series (0015 §1), with its known biases: Hann sidelobes inflate it (1.26× in
  the first bucket), it is a median (× 3.157 for a 10⁻³ threshold), and it needs
  gaps of ≥ 3 zero-padded bins — valid from the tenor up and **not in the bass**.
- **T4 — the bass.** T2 at bass spacings, and the isolation set: a solo string's
  partials against the same key's open note.
- **T5 — H₀ populations for false alarms.** (i) Gatekeeper-silent pre-roll hops;
  (ii) during notes, bins and probe frequencies at least one Hann main lobe from
  every predicted partial of the named key; (iii) T2's declared-absent partials;
  (iv) 0015 §2's H₀ subset on real notes (dead and SNR_true < 1).

### 2.3 Baselines and arms common to every stage

- **A** — the shipped scalar at each capture's recorded σ, and at a fixed
  σ = 10⁻³ to separate the register gradient from the session's slider.
- **B** — the oracle, 3.157 × N_local (T ÷ T_ideal ≡ 1 by construction) where T3
  is valid; the exact noise level on a synthetic set.
- **C** — the shipped coarse read's OS-CFAR (`peaks::coarse_read`), as one
  frequency-local arm with its search-loss budget; parity by `gates verify`.
- **N** — no gate: every local maximum and every probe admitted, the floor every
  rate is read against.

## 3. Candidates, by consumer

Six families separate noise from signal — by time, by frequency, by exclusion,
by distribution shape, by a signal model, by predictability — and a seventh
tests the signal itself. First pass: one representative per family per
consumer; variants only where the family passes.

**G1, the picker.**

| Family | Representative | Variants, second pass |
| --- | --- | --- |
| time (a floor learned between notes) | the calibration's per-bin shape with a gated running update on silent hops (Nitzberg 1986's clutter map; Boll 1979), finite-N multiplier from ν/2, one per-hop scalar correction | a fitted power-law shape while bins mature; a censored mean at the same rank; the order-statistic clutter map per band, k = 3M/4 (Shan et al. 2002); a self-gated band update (Hendriks et al. 2009) |
| frequency (the same hop's bins) | arm C, the low-rank order statistic in flanks with the search-loss budget | Laurenti, De Poli & Montagner 2007's reciprocal-mean floor (3-bin pre-average, k₁ = 0.90), valid from an 8-bin spacing (key 9); neighbour bins |
| exclusion | forward consecutive mean excision on the whitened frame (Vartiainen et al.) | Yeh & Röbel's Rayleigh test; the biweight; Laurenti's α = 4 / K = 3 spike pass |
| distribution shape | an empirical null on the standardised log-power (Efron 2004), a scalar on the floor's shape | a trimmed scale |
| per-frequency F-test | Thomson 1982's harmonic F-test at NW = 3 (K = 5) | NW = 2, 4 |
| signal side (a second stage after any floor) | Zivanovic, Röbel & Rodet 2008's peak-shape classifier | phase predictability across hops |
| model-based | — | a line fit with CFAR on the residual, exploratory, bass only |

**G2 and G3, the gates at a known frequency.**

| Family | Representative | Variants |
| --- | --- | --- |
| time | the per-bin floor read at the target bin, converted to the Goertzel's window (0015 §1's rule), multiplier from Stage B | the running variants above; a log-domain update |
| frequency | the same Goertzel at neighbouring frequencies, predicted partials excluded (Johnson et al. 2010's practice) | a rest-of-band ratio with a numerical Hann null |
| model-based | the APES residual with (f₀, B) from MAT and the tracker's frequencies (Nørholm, Jensen & Christensen 2016) | a prediction-residual form at one window's delay, bias stated |
| signal side (no noise estimate) | **frequency consistency**: the tracker's f_inst against the target over the last k hops | amplitude continuity; phase predictability |
| the decision's form | a hard gate | acquire/hold hysteresis; for G3 a soft weight in the rate fit |

**BG, the background measurement.**

| Family | Representative | Variants |
| --- | --- | --- |
| frequency (the valleys at 0.67 Hz) | the median along frequency with the Rayleigh conversion (Rigaud's form, the oracle's construction) | Laurenti's reciprocal mean (+1.4 dB at A0, derived); a truncated-ML frequency-only form |
| acceptance margin | ISO 3744's 6 dB / 15 dB as partial-to-noise power ratios 2.98 and 30.6 | Oppenheimer 2016's steadiness margin, M = 1: ΔL₀ 5.2 dB, ΔL_min 7.7 dB; energetic subtraction with its median caveat |
| product exclusion before MAT's pick | — | peaks within a derived window of n·f₁ in the treble, exploratory |
| model-based | — | the F-test's reshaped spectrum, exploratory |

**LK, the distribution.**

| Quantity | Method |
| --- | --- |
| the per-bin law within a stationary block | the periodogram standardised by the block's averaged periodogram, against its exact law (Talbot & Thrane 2020; Sulis et al. 2017), on the pre-roll and T5(i); blocks of 1 s, 10 s, the whole record |
| the level's wander | each bin's block-to-block level SD; Oppenheimer's steadiness M = μ²/σ² against the block's ν/2 |
| heavy tails | the finite-variance test, A(1) and C(1) only (Skowronek, Zimroz & Wyłomańska 2024), null re-simulated for exponential powers, blocks ≥ 500 hops or the pooled pre-roll |
| the kurtosis bands | ±2Φ⁻¹(α)/√M for a stationary background, twice that for a Gaussian-driven non-stationary one (Antoni 2006): outside the first, inside the second is *level not stationary* |
| the tail | exceedance counts at the Rayleigh quantiles p = 0.1, 0.01, 0.001 with binomial intervals; 0.001 only with minutes of silence |
| the location read three ways | mean, median/ln 2, log-domain mode, on screened and unscreened pre-roll: the shift is the screen's effect, measured |
| a two-component law | a two-Gaussian log-energy mixture, only if the count shows the need |

## 4. Quantities, the same for every arm

- **Q-A, estimation error against truth** (primary). Per bin and hop,
  20 lg(estimate ÷ truth) on T2 (exact) and T3 (the oracle, tenor up): median
  and 5–95 % by register × bucket; and **coverage**, the fraction of hops where
  the truth lies inside the estimator's own stated interval. A candidate with
  no stated uncertainty is scored on error alone and says so. For signal-side
  arms Q-A does not apply and Q-B decides.
- **Q-B, decision quality** (primary). Realised false-alarm rate on each T5
  population against nominal, per register — for G1 the count of candidate
  peaks per hop, for G2/G3 the fraction of probes admitted; on T2, Nunes et al.
  2007's γ (detections), ζ (false alarms) and κ = γ − ζ with their
  reference-alignment rule. Miss rate on live partials at SNR_gate ≥ 3 (0015 §2,
  anchored to 0014 §8a), reported also at ≥ 6, 10, 20 and at the ISO 3744
  margins (partial-to-noise power ratios 2.98 and 30.6, converted to amplitude
  over the noise's median magnitude: √(R / ln 2) = 2.07 and 6.64; Part 2 §6
  records the transcription error this line carried into the first runs).
- **Q-C, time.** T ÷ T_ideal as a distribution per register and as a curve over
  0015's buckets; the first 0.25 s separately; for every arm that updates
  during notes, the self-masking time (Delyon's Eq. 6: 8.7 hops at F = 0.02 for
  a 40 dB partial, derived); recovery after a level step, exploratory.
- **Q-D, consumer outcomes.** G1 → the lock replay at sixteen phases of the hop
  grid on both pianos, per register, against the baseline's own scatter (report
  0020); G2 → MAT's seeding and f₀ repeatability; G3 → the band rate's scatter
  and the gated fraction over the note; BG → 0009 Analysis 7's yardstick (the
  treble's repeat log-SD of B, 0.137 as shipped, 0.102 for the single-pair
  estimator; keys beating the prior, 9 of 25; no bass or mid key moving more
  than its repeat SD); LK → calibration: the claimed law's exceedance counts
  within their binomial intervals, per band, blocks and pooled.
- **Q-E, cost.** Per-hop microseconds and state bytes from `cargo bench -p
  tuner-core` for anything meant for the hot path.

Every median- or quantile-based arm carries its conversion to a mean
equivalent, shown on T1 before any real number.

## 5. Nulls and controls, fixed before scoring

- Per bin within a stationary block: iid exponential power (Rayleigh magnitude),
  standardised by the block's average; adjacent Hann bins correlated 4/9 in
  power, effective count n/1.944.
- An estimated floor's threshold: the finite-N law (F(2, 2M) or Rohling's
  order-statistic law), never the known-σ law; a ratio that includes its own
  cell: a numerical Hann null, not the Beta law of independent bins.
- The F-test: F(2, 2K − 2), exact; its per-hop local-maximum count on noise is
  the G1 figure.
- Signal-side tests: uniform phase increments (random phase), the Rayleigh law
  for amplitude continuity; where the hops overlap, the null is simulated at the
  window.
- Search loss for anything that scans: a budget P_fa/M over the bins it
  searches (0011 §5), reported as a per-hop count as well.
- Semi-synthetic runs state the injected SNRs, decays, B and beating; the
  generator is committed before scoring and not revised against the captures.

## 6. The decision rules, fixed now

Per consumer the deciding quantity is the consumer's own outcome (Q-D); rules 1
and 3 are required with it, rule 2 is required for BG and LK and reported for
the gates, rule 5 is a filter. A candidate is **kept for the owner's decision on
a consumer** if, on both piano-#2 sets and on the MAPS ambient read, and without
failing on any other external set where the quantity is readable:

1. its realised false-alarm rate on every T5 population it is scored on lies
   within one order of magnitude of nominal in either direction (0015 §2's gate)
   and above the rate whose binomial probability given nominal is under 1 %
   (over-rejection fails too, 0015 §10);
2. on T2 its Q-A median error lies within its own stated uncertainty in every
   register where it is claimed, its compass spread of median error is under 5×
   (0015 §12's anchor), and no register is worse than arm A beyond A's scatter;
3. its miss rate at ≥ 3× beats A in every register by more than its own scatter
   across repeats (`CONTRIBUTING.md`, the estimator's own scatter), with
   T ÷ T_ideal's first-bucket value reported beside it;
4. for G1, the lock replay shows no register losing more keys than the
   baseline's scatter (0020's margin);
5. its per-hop cost leaves the chain within the hop by the margin the owner
   names (a product judgment).

For G2/G3 a **signal-side arm** is kept on rules 1, 3 and 4 alone; "no noise
estimate for the gates" is a permitted finding. For **BG**, kept if Q-D's treble
log-SD improves beyond the pooled-repeat scatter with no bass or mid key moving
beyond its repeat SD, and its acceptance rule's false-partial rate on T2 meets
rule 1. For **LK**, a law is adopted per band only when its exceedance counts at
p = 0.1 and 0.01 lie inside their binomial intervals on blocks of ≥ 500 hops or
on the pooled pre-roll; 0.001 only where minutes of silence exist.

**Across registers.** One mechanism per consumer meeting rules 1–3 in all four
registers is the goal. A split by register is admitted only if no single
candidate passes everywhere, and then its boundary is derived from a stated
inequality (the partial spacing in bins against the window: the record's own
N > 4·f_s/f₀ and Laurenti's 8-bin bound are the forms), never tuned on the sets.

**The external sets** are read together with piano #2, each with its own σ fixed
before the read, as 0020 did. A candidate that passes on piano #2 and MAPS
ambient but fails on another external set goes to the owner with the failure
named and the set's caveat beside it. Anything that passes on one set, or on
T1/T2 only, is *suggestive* and queues confirmation. Two instruments cannot
select: the output is per-register counts, which keys move, and curves over the
operating point; the choice is the owner's.

## 7. The bass, a separate question

Pre-registered population: the longer window where the hop allows it; the
uncalibrated ratio test that ships; the coarse read's reference; a
silence-learned per-bin floor whose bass semantics are "room noise, not leakage"
(stated as such); the between-predicted-partials reference once a key is named;
Thomson's F-test at NW = 3 and 4; a line fit with CFAR on the residual,
exploratory, with its cost; a prediction-residual form at one window's delay on
the isolation set, its +1.8 to +3.0 dB stated; and the signal-side f_inst
consistency test, which asks whether a bin at a dead partial's frequency
advances at the neighbour's frequency. Truth: T4. The deciding quantity is the
dead-partial admission rate on 0015 §6's H₀ subset (78–93 % as shipped) beside
the live miss rate. The bass is answered when a candidate meets rules 1–3 there
as elsewhere; if none does, the derived register split is the fallback, with
the bass's own winner named.

## 8. Stages, and the order of work

Three phases, one fresh session each: **Phase I** = Stage A (the harness, the
truth builders, one mode per arm, parity with the shipped code, the silent-hop
spectra the Gatekeeper never computes; bar: every mode reproduces nominal on
T1) and Stage B (the distribution from existing data; it sets every finite-N
multiplier used after it and is LK's first read). **Phase II** = Stages C–F as
one measurement pass over every consumer and register: G2/G3 in the tenor and
treble, sustain then onset; G1 with the search-loss budget and the per-hop
count; the bass, all consumers; BG on the 2¹⁶ window. **Phase III** = Stage G
(combinations only if the strike or the drift is left unaddressed) and Stage H
(the lock replay at sixteen hop phases on both pianos for every G1 arm that
passed rules 1–3; parity; the bench — last, because a gate that passes it while
failing rule 1 has merely moved the lock's behaviour). Variants run inside a
stage only for a family whose representative passed.

## 9. What the owner decides, and what is measured instead

- **The per-hop cost — measured.** The hop is 23.2 ms; the target is a midrange
  phone, so the bar is a proxy: the total hot-path cost under **10 % of the hop
  on the development machine**, standing in for under half the hop on a device
  about five times slower (a stated assumption, replaced by a bench on a phone
  when one exists). The shipped hot path is far inside it (the strobe bank at
  0.4 %, report 0012 §9).
- **The picker's target** (a per-bin rate or a per-hop count) — not chosen in
  advance: the operating point is swept and every metric computed at every
  point, and the point the lock replay prefers is reported with the rest beside
  it (a threshold-dependent question has no threshold-free answer,
  `CONTRIBUTING.md`).
- **Registers** — one mechanism is the goal; the split is the derived fallback.
- **The hysteresis hold ratio** — a small family is run and reported, the one
  Stage C can anchor to the measured floor marked.
- **The chain's own floor** needs no decision: it cannot be recorded on an
  internal microphone, so Stage B bounds it from the quietest bins of the room
  recordings and reports which bins sit on it.
- For the owner: the 2-minute room recordings, one or two per room, whenever
  convenient; and the tree slot for the lab code this report describes.

---

# Part 2 — Phase I

## 1. What was built, and where

Lab only: nothing under `tuner-core` or `tuner-gui` changed, and the two
parity checks the protocol names hold on this tree — `cargo lab engine lock
diagnostics_piano2` gives 568 / 595 (bass 134/137, mid 296/298, treble 138/160)
and `diagnostics_piano_1` 82 / 87, as report 0020 recorded; `cargo lab gates
verify` agrees with the shipped `peaks::coarse_read` on 100.0000 % of hops with
Δf = 0 at both analysis sizes on all three capture sets.

| What | Mode | Where |
| --- | --- | --- |
| the AWGN control (T1) | `cargo lab gates truth t1 <OUT>` | `gates/truth/mod.rs` |
| known partials in real noise (T2), both beds, the fidelity tables | `cargo lab gates truth t2 …` | `gates/truth/inject.rs` |
| one arm over one or more dump sets, `gates ambient`'s columns | `cargo lab gates arms --arm <NAME> --set <LABEL> <REGEN> <ROOT> …` | `gates/arms/mod.rs` (the `Arm` trait, the driver, the tables); one file per family under `gates/arms/` |
| the room's per-bin distribution (Stage B) | `cargo lab gates distribution --set <LABEL> <ROOT> <SIGMA> … [--recording DIR]` | `gates/distribution.rs` |
| the exact laws the tables compare against | — | `gates/stats.rs` |
| the lock replay at phases of the hop grid, with a G1 arm | `cargo lab engine replay <ROOT> [--phases 16] [--peak-arm NAME] [--regen REGEN] [--out CSV] [--known-key] [--carry]` | `engine/replay.rs` |

**The driver.** Every hop of every capture runs as the pipeline runs it: the
8192-sample window on the hop grid, its Hann FFT in the units `extract_peaks`
thresholds, the Gatekeeper's own verdict on the hop (replayed over the full
event at the capture's logged thresholds), the register window's Goertzel at
the capture's measured partials (G3) and at the tracker's adaptive target (G2,
re-centred on the hops the shipped gate admits, the trajectory the instrument
took), and one 2¹⁶-point window at the record's start (BG). The arm sees the
same `HopCtx` the consumers see and answers per bin (G1), per probe (G2, G3) or
per partial (BG). A row is written per (capture, hop, partial) with the
register, the time bucket, the estimate, the truth where known, the threshold,
the decision, SNR_gate, SNR_true and the error in dB; a per-hop row counts the
picker's false candidates off every predicted partial of the key.

**The pre-roll.** The kept record is located inside the full event by an exact
match of its opening samples, not by the suffix test the earlier harness used:
on piano #2 the record begins 20 480 samples (464 ms, the 348 ms pre-roll plus
the 116 ms to `Stable`) into the full event in every capture checked, and the
suffix test is satisfied only by captures whose note decayed before the record
filled. Report 0015 §10's silence population (2 425 probes on piano #2) was
therefore the set's early-decayed captures, which `--preroll strict`
reproduces (64 captures, 119 hold-out hops). A screened silent window carries
the Gatekeeper's `Silence` verdict and ends at least four hops before the
strike, which is the full event's −20 dB onset where that precedes the record;
on piano #2 it does, by five to six hops, so a capture keeps a median of four
screened windows, and the aligned silence population is 2 155 windows on 588
of 595 captures, 10 463 G3 probes.

**Hold-outs** are by time blocks: a capture with sixteen or more silent windows
splits within itself, the earlier half building, the later half scoring, with
an eight-hop gap between them so no scored window shares a sample with a
learned one; a shorter pre-roll (every real capture's) makes whole captures the
blocks, alternating in the order the session recorded them. A running arm learns
from build hops only.

**Units and conversions.** A level in the raw 8192-point frame goes to a
Goertzel's `4/n` units at window `n` by `(4/8192)·√(8192/n)` (report 0015 §1's
processing gain); the control checks the chain: arm A at the true σ reads
T ÷ T_ideal 0.96–1.01 in every register at every window.

**The operating point** is a parameter of every arm (`--pfa`, 10⁻³ by
default), so Phase II can sweep it.


## 2. The control: every mode against a known answer

`cargo lab gates truth t1` writes 100 captures of white Gaussian noise at
σ = 10⁻³ on ten keys (3, 15, 27, 39, 51, 63, 71, 78, 83, 87), partials 1–4
present at 30 dB per bin, 5–8 declared and absent (fewer where Nyquist cuts
them), 43 hops of pre-roll, and beside each capture the noise alone. Its
`analysis.json` records what the app's calibration would have recorded, 1.5 ×
the loudest pre-roll hop RMS = 1.56 × 10⁻³, because the Gatekeeper's screen
runs at that value in the app; the true σ stays in the truth, and `A@1e-3` is
the shipped rule at the true σ. The noise's level per bin is known in closed
form (its median magnitude `σ√(0.375 N ln 2)`), so the control's **exact
threshold** is Kay's at the true σ, and its own realised rate on each
population is the nominal every arm is read against: for a Goertzel probe that
is the operating point itself; for the picker it is the Hann geometry's rate
of local maxima above the threshold — **9.02 × 10⁻⁴ per bin at 10⁻³**, which the
earlier synthetic derivation put at 9.04 × 10⁻⁴ and this port now measures.

Realised false-alarm rates, 95 % Clopper–Pearson intervals, against nominal:

| arm | G1 lobe search (18 240) | G1 scan, note bins (19.9 M) | G1 scan, silence (4.9 M) | G2/G3 absent (17 920) | G2/G3 silence (8 640) | BG absent (320) |
| --- | --- | --- | --- | --- | --- | --- |
| exact threshold (nominal) | 60 → 3.29e-3 | 9.02e-4 | 8.90e-4 | 1.0e-3 | 1.0e-3 | 2 → 6.3e-3 |
| **A@1e-3** (true σ) | 60, inside | 9.02e-4 [8.89, 9.16] inside | 8.90e-4 [8.64, 9.17] inside | 9.49e-4 [5.5, 15.2] inside | 1.74e-3 [9.7, 28.6] inside | 1 inside |
| **A** (recorded, 1.56σ) | 0, **below** | 1.5e-7 **below** | 0 **below** | 0 **below** | 0 **below** | 0 inside |
| **B** (oracle) | 60, inside | 9.02e-4 inside | 8.90e-4 inside | 9.49e-4 inside | 1.74e-3 inside | — |
| **C** (per-bin OS-CFAR) | 0, below | 1.10e-5 [0.96, 1.26] **below** | 7.2e-6 **below** | — | — | — |
| **N** (no gate) | 0.846 | 0.276 | 0.276 | 1.0 | 1.0 | 1.0 |
| **1** (running floor) | 52 → 2.85e-3 inside | 9.06e-4 [8.93, 9.19] inside | 8.87e-4 [8.61, 9.14] inside | 6.1e-4 [3.1, 11.0] inside | 1.39e-3 [7.2, 24.2] inside | 1 inside |
| **finst** (frequency consistency) | — | — | — | 5.74e-4 [2.6, 10.9] inside; bass 3.4e-4, tenor 7.7e-4, treble 5.1e-4, high 1.02e-3 | — | — |

Intervals are in units of 10⁻⁴ where abbreviated; "below" and "above" mean
outside the interval on that side. Arm 1 declines until eight silent hops have
been learned, so its denominators are smaller. Every floor arm here and in §5
takes the analytic Hann ν/2 per frame; §4 measures it on the control at 1.009
of that value.

What the table says, beyond the verdicts:

- **A as recorded over-rejects everywhere by design.** The calibration rule's
  1.5× puts the shipped threshold 1.56σ above the noise it was measured on, and
  on white noise that is 0 admissions in 18 240 lobes, 3 in 19.9 million bins:
  report 0015 §10's "0 of 10 258" in kind. The same rule at the true σ is
  exact. This is the shipped gate's calibration, not a harness artefact.
- **The oracle and the exact threshold coincide on the control**, as they
  must, and the running floor sits within 0.5 % of them on the picker once it
  has learned: its T ÷ T_ideal is 1.01–1.05 by register.
- **Arm C realises 1.1 × 10⁻⁵ per bin at a 10⁻³ operating point.** Its budget
  is per band: `P_fa/M` over the band's independent cells plus the halving of
  the reference count (report 0011 §5, audit 13), calibrated to one false
  reading per thousand *reads*, which per bin is two orders of magnitude
  stricter. As a per-bin picker gate it is a different operating point, which
  Phase II's sweep of `--pfa` puts on the same axis as the others.
- **The frequency-consistency arm needs a numerical null.** Its statistic is
  Rayleigh's on the last eight wrapped phase increments; at the 1024-sample
  window the increments of noise are nearly independent and the simulated 10⁻³
  point (Z ≥ 5.77) sits beside Greenwood & Durand's series (5.73), while at the
  4096-sample window the hops overlap three quarters and the point moves to
  7.61. With the series alone the bass realised 6.8 × 10⁻²; with the simulated
  null, 3.4 × 10⁻⁴.
- **The no-gate arm measures the geometry itself**: 0.276 of bins are local
  maxima (the derivation's 0.2762), and 84.6 % of absent partials' lobes hold
  one.

Three things the control taught about the harness, each fixed before the
numbers above and each a trap for anyone repeating the measurement. A Gatekeeper
screen whose threshold equals the noise's own RMS passes the quieter hops and
biases every floor learned from them, which is why the control records the
calibration rule's threshold. A hold-out window that overlaps its own build
window by seven eighths under-reads the false-alarm rate by a tenth, hence the
eight-hop gap. And a floor's equivalent independent periodograms must be summed
over contiguous runs of hops, not computed as if every hop learned were one
overlapped run: the latter puts the threshold 20 % high and the realised rate
25× low.


## 3. T2 and its fidelity

**The generator.** `cargo lab gates truth t2 <REGEN> <ROOT> <OUT> [--bed
room|residual] [--long-root DIR --long-regen REGEN] [--per-key N]
[--preroll-hops 15] [--seed S] [--keys …] [--strike-gain-db ×4]` mirrors each
capture of a `mat regen` dump: every located partial (n ≤ 32) at the capture's
own measured frequency and Worker amplitude, the amplitude inverted through the
Worker's reading (the scalloping at the rounded bin and the envelope's weighting
under the 2¹⁶ Hann window, so a stationary partial on its bin re-reads a_n to
0.000 dB, p5–p95 ±0.001 dB); each partial under a two-stage decay in dB (early
slope, late slope, knee) drawn per (register, partial) inside the quartiles of
the fits on the extended store's 206 long records (1 596 two-stage fits of 1 667
partials); the key's strings — one to key 11 (report 0013), two to D3, three
above — split by an as-found unison split drawn in report 0014 §2's range
(0.88–3.67 ¢ below A4, 9.2–18.5 ¢ from A4), the middle string at f_n and the
power shared equally; four partials past the last located one declared absent
(T5(iii)). Two beds. **T2a**: the set's screened pre-roll segments, distinct
(the mirrored capture's own first, then its nearest serials), tiled with
√raised-cosine crossfades that keep the mean power, plus a white strike from the
record's first sample under +20 dB·e^(−t/20 ms) + 5 dB·e^(−t/150 ms) over the
bed's RMS, scaled per register (below); `noise.raw` holds bed and strike.
**T2b**: the mirrored capture's own full event less one sinusoidal model per
located partial — the 4096-point phasor tracked hop by hop at the tracker's
adaptive target, fitted by least squares to a two-stage dB decay and a quadratic
phase, resynthesised sample by sample from the onset to one hop past the room
floor. The model is fitted rather than interpolated because the track as
measured carries the noise inside the window's band with it: subtracted hop by
hop it notches the bed 15–21 dB at f_n, the six-parameter fit leaves −0.9 dB
(both asserted in tests). Each set writes capture directories in T1's format,
`truth.json` in the regen schema with every drawn parameter and bed segment,
`fidelity.md`, `fidelity_decay.csv` and, for T2b, `fidelity_residual.csv`;
264 captures take 3 s.

**The fidelity tables, once**, on T2a of piano #2, three captures per key (264),
seed 20261009, decays from the extended store (`sets/t2a/fidelity.md` in the run
record). Medians over captures; the declared profile equals the real one by
construction, so the row that carries information is the Worker's re-reading of
the synthetic record, bed, strike, beating and decay included.

| 20 lg(a_n / a_1), dB | n = 2 | 3 | 4 | 5 | 6 |
| --- | --- | --- | --- | --- | --- |
| keys 0–47, real | +8.0 | +3.3 | +4.4 | +2.7 | −2.5 |
| keys 0–47, synthetic re-read | +9.8 | +4.4 | +6.2 | −0.0 | −1.6 |
| keys 0–47, report 0009 Analysis 7 | +8.2 | +5.2 | +3.9 | — | +2.4 |
| keys 62–87, real | −29.7 | −44.5 | −47.3 | −48.6 | −52.7 |
| keys 62–87, synthetic re-read | −27.6 | −38.1 | −46.0 | −45.6 | −48.8 |
| keys 62–87, report 0009 Analysis 7 | −28.3 | −40.7 | −49.8 | −53.0 | −56 to −60 |

By register the re-read sits within 2 dB of the real profile in the bass and
treble through n = 6, 4–6 dB under it in the tenor at n = 2–6, and 4–10 dB over
it in the high register at n ≥ 3, where the declared level is below the room
bed at those bins and the Worker re-reads the bed.

| decay, median [IQR] | early dB/s | late dB/s | knee s |
| --- | --- | --- | --- |
| bass, real whole record (950) | −14.2 [−25.9, −6.7] | −6.1 [−9.6, −3.9] | 0.74 [0.39, 1.00] |
| bass, real first 1.5 s | −13.6 [−29.8, −4.9] | −7.1 [−13.9, −1.7] | 0.56 [0.30, 0.84] |
| bass, synthetic re-fitted on 1.5 s (2 675) | −20.3 [−39.0, −9.1] | −6.8 [−11.3, −3.4] | 0.60 [0.33, 0.84] |
| tenor, real whole record (479) | −18.1 [−32.1, −6.9] | −8.8 [−14.2, −6.0] | 0.65 [0.30, 1.00] |
| tenor, real first 1.5 s | −15.6 [−36.6, +1.0] | −11.9 [−20.6, −4.2] | 0.56 [0.26, 0.88] |
| tenor, synthetic re-fitted on 1.5 s (1 635) | −24.8 [−49.7, −4.5] | −9.8 [−13.9, −4.7] | 0.51 [0.23, 0.79] |
| treble, real whole record (132) | −41.8 [−67.6, −31.6] | −17.7 [−28.5, −9.7] | 0.63 [0.33, 1.00] |
| treble, synthetic re-fitted on 1.5 s (507) | −52.0 [−71.0, −38.1] | −16.4 [−29.8, −7.9] | 0.58 [0.30, 0.79] |
| high 76–87, real whole record (35) | −71.9 [−113, −42.5] | −19.3 [−26.3, −0.5] | 0.37 [0.19, 0.70] |
| high 76–87, synthetic re-fitted on 1.5 s (160) | −77.2 [−144, −56.3] | −17.3 [−38.5, +2.5] | 0.26 [0.16, 0.44] |

The drawn decays reproduce the whole-record fits by construction; re-fitted on
the synthetic record's own 1.5 s they read 6–10 dB/s steeper early in the bass
and tenor than the real records' first 1.5 s do, with the late slopes and knees
within the real IQRs: the synthetic notes are more two-staged inside 1.5 s than
the real ones. 214 of 5 015 drawn decays came from the nearest partial number's
cell, their own holding fewer than four fits.

| strike: N_local at 0.1–0.25 s over the pre-roll's | bass | tenor | treble | high 76–87 |
| --- | --- | --- | --- | --- |
| real captures (0015 §8's method, same gaps) | 8.02 [6.98, 9.04] | 4.04 [2.97, 5.32] | 2.32 [1.87, 3.04] | 1.57 [1.31, 2.24] |
| synthetic, strike off (E₀) | 2.91 | 1.20 | 1.04 | 1.02 |
| synthetic, the two-term model at 0 dB (E₁) | 3.89 | 6.25 | 7.09 | 6.75 |
| **synthetic, re-drawn** | 6.01 [4.30, 8.27] | 4.02 [3.21, 4.96] | 2.42 [1.96, 2.84] | 1.65 [1.55, 1.79] |
| gain applied, dB | +9.23 | −4.03 | −10.58 | −14.95 |

The strike was the one table outside its range: the spec's broadband burst read
6–7× in the tenor, treble and high registers against the real 1.6–4× and 0015
§8's 2–5×, and 3.9× in the bass against 8×, where the synthetic partials' own
leakage into the gaps already gives 2.9× with no strike at all. Per §2.2 the
generator's range was **re-drawn once** from the record: a gain per register
with g² = (E² − E₀²) ÷ (E₁² − E₀²), the strike's power adding to the leakage's
— a hammer whose excess falls 24 dB from the bass to the top octave. The
re-draw lands the tenor, treble and high registers on the real medians and the
bass at 6.0× against 8.0×, the power-additive model under-correcting where
leakage and strike are comparable; there is no second re-draw. The real record
begins 116–139 ms after its strike and the synthetic one at its strike, so the
same bucket of record time lies later after the hammer in the real captures.

| unison drawn | strings | split, ¢ (min / median / max) | beat at f₀, Hz |
| --- | --- | --- | --- |
| bass (84) | 1, 2 | 1.01 / 2.46 / 3.64 | 0.04 / 0.12 / 0.25 |
| tenor (72) | 2, 3 | 0.89 / 2.65 / 16.75 | 0.10 / 0.32 / 4.80 |
| treble (72) | 3 | 9.26 / 14.20 / 18.13 | 3.4 / 8.5 / 18.7 |
| high 76–87 (36) | 3 | 9.29 / 15.03 / 18.13 | 13.3 / 24.6 / 39.2 |

Declared absent, T5(iii): 336 / 288 / 237 / 10 partials by register, of which
0 / 6 / 184 / 10 fall at n ≤ 12 where `regen::load` keeps them, so the
absent population is the treble's. With the strings at f_n(1 ± Δ/2), a
bichord's partial reads −10.6 dB at f_n's bin (p5 −41, p95 +1) and a trichord's
−4.8 dB wherever the split exceeds a bin — the spec's model, measured, and the
reason the re-read tenor profile sits under the declared one.

**T2b's leftover.** On the same 264 captures the residual keeps, at each located
partial's bin against the gaps beside it, a median of +21.7 dB [+11.9, +31.4]
in the bass (2 566 partials read, 2 450 above +1 dB), +31.0 dB in the tenor,
+18.1 dB in the treble and +8.5 dB in the high register, with a
magnitude-squared coherence of 0.19 / 0.38 / 0.56 / 0.72 between the original
and the residual there; the originals read +36 / +38 / +26 / +11 dB. One
sinusoid per located partial follows a single string (21–23 dB of suppression
on single strings, n ≤ 10) and not a unison (8–17 dB on bichords, 3–4 dB on
trichords, whose fitted glide is about 1 Hz/s): T2b's bed is **not noise at
the partials**, and its declared-absent partials sit on the real capture's
unlocated ones, which the residual keeps. T2b's hammer, by contrast, is the
real one: its first-bucket excess reads 8.01 / 3.87 / 2.38 / 1.60× against the
real 8.02 / 4.04 / 2.32 / 1.57×. T2b serves the pre-roll, the gaps and the
sustain's background; it does not serve T5(iii) or any read at a partial's bin,
and `truth.json` carries each partial's remnant and coherence so a consumer can
exclude it.

**The exact level on T2.** The rulebook says the noise under every partial is
known exactly at every hop; what is known is the realisation, and a level is a
statistic of it. The skeleton read the level as the per-bin median over the
whole noise file, which the strike and the bed's segment-to-segment changes
make wrong hop by hop (on the first scoring the exact rule itself admitted
17 % of absent lobe peaks). Phase I reads it **per hop**: the median of the
known realisation's 8192-point Hann spectrum over ±16 bins (33 bins, 178 Hz) at
that hop for G1, the scan and SNR_true; the same median of the realisation's
own `win`-point spectrum, in the 4/n units, for a G2/G3 probe at its 1024- or
4096-sample window (the 8192 frame averages a burst the 23 ms probe meets at
full strength); the median over ±32 bins of the 2¹⁶ window for BG. T1 keeps its closed form, and the construction is calibrated on it: with
`--exact-median` the control's exact level is read the T2 way, and against the
closed form it shows no bias (0.0 dB median in every register and bucket) and
a scatter of ±2.2–2.7 dB at 5–95 %, which widens every T2 Q-A spread by that
much and leaves its medians alone. The scatter has a price on the exact rule
itself: a threshold set on a jittering level is crossed more often, and on
white noise the exact rule read the T2 way fires at 1.77 × 10⁻³ per bin
against the closed form's 9.02 × 10⁻⁴, 2.3 × 10⁻³ on a Goertzel probe against
10⁻³ — about twice nominal from the construction alone. This is an executor's
reading of the rulebook's words, not an amendment of a bar.

**The global check: the shipped gates on T2a behave as they do on the real
captures.** Arm A at the recorded σ, G3, miss at SNR_gate ≥ 3 and the
false-alarm rate on screened pre-roll probes, by register, against the three
real sets (§2.2's bar: within the record's scatter):

| G3, arm A | bass | tenor | treble | high 76–87 | pooled |
| --- | --- | --- | --- | --- | --- |
| miss % at ≥ 3×, T2a | 3.2 | 28.5 | 55.8 | 85.1 | 21.4 |
| miss % at ≥ 3×, piano #2 / extended / piano #1 | 1.2 / 33.0 / 0.2 | 26.0 / 71.7 / 18.0 | 48.3 / 77.9 / 43.1 | 75.0 / 86.7 / 67.1 | 22.1 / 52.9 / 13.7 |
| silence P_fa, T2a | 0.168 | 0.0215 | 0 | 0 | 0.0688 |
| silence P_fa, piano #2 / extended / piano #1 | 0.170 / 0.022 / 0.082 | 0.027 / 0.004 / 0.006 | 0.001 / 0 / 0 | 0 / 0 / 0 | 0.061 / 0.010 / 0.035 |

Every register's row on T2a lies inside the three real sets' range on both
quantities, and the bass silence rate reproduces piano #2's to the third digit
(the T2a pre-roll is the real pre-roll). That range is a weak bar — 14 to 53 %
pooled — so the stricter reading is against piano #2 alone, the set T2a
mirrors: bass 3.2 against 1.2, tenor 28.5 against 26.0, treble 55.8 against
48.3, high 85.1 against 75.0, pooled 21.4 against 22.1. T2a misses more in the
treble and above than its model does, by 7–10 points. Part of that is the
truth: "live" means SNR ≥ 3 against the exact per-hop level on T2a and against
the oracle on the real sets, and the oracle reads high by its leakage, which
moves partials out of the live population on the real sets and not on T2a.
The check holds before and after the strike's re-draw (miss 20.0 → 21.4 %
pooled; the weaker treble strike leaves more partials above 3× in the first
bucket, which A still rejects). Arm A's
Q-A on T2a — 20 lg(estimate ÷ exact level) — reads +5 dB [−8, +20] in the bass
and +21 to +27 dB in the tenor and above, the shipped broadband σ against the
room's coloured per-bin level; against the oracle on the real sets it read −11
to −2 dB in the bass and +12 to +26 dB above, the bass difference being the
oracle's leakage bias there (0015 §9).

**What the exact rule itself realises on the real bed** (arm B is the exact
threshold on a synthetic set): on G1 its lobe search admits 1.56 % of absent
partials and its scan 4.5 × 10⁻³ of off-partial bins per hop (5.4 × 10⁻³ in the
pre-roll) against the Hann geometry's 0.33 % and 9.0 × 10⁻⁴ on white noise. About half
of that excess is the construction's own: the same rule read the same way on
white noise runs at twice nominal (above). The other half is the bed: the room
exceeds 3.157 × its local median about 2.5× more often than Rayleigh at 10⁻³,
from lines finer than 178 Hz and from change within the window. On G2/G3 at the probe's own window the exact rule admits 0.40 % of
absent partials during the note (4× nominal; 0.75 % in the first bucket, 0.34 %
from 0.5 s on, the strike's remainder) and **9.4 % of the screened pre-roll
probes** — 34 % at the fundamental, 16 % at n = 2, 10–11 % at n = 3–4, under
3 % from n = 8, the same in every one of the four pre-roll hops, 15 % in the
bass and 3 % in the treble, sixty times the off-partial scan rate in the same
hops. **The screened pre-roll holds the key's own partials, strongest at f₁:
the key's strings are ringing before the strike** — the previous repeat's tail
or the same strings sympathetically — at a level the Gatekeeper's broadband RMS
screen does not see. Report 0015 §10's "0 of 10 258" silence admissions and
this run's 5.9 % (6.1 % with the four-hop guard) on the aligned population are
the same threshold on two populations: 0015's suffix test kept only the
captures whose note had decayed inside the record, treble notes almost
entirely, while the aligned population is every capture, the bass included,
whose strings ring longest; the admissions here are the tail, not noise over a
threshold. For Phase II this redefines T5(i): a silence probe at the key's own
partial frequencies is not an H₀ probe on a repeated key; the H₀ population in
the pre-roll is T5(ii)'s geometry (one Hann lobe from every predicted partial),
or the first capture of a key in a sitting. The arms' silence columns in this
report are read with that in mind.

## 4. Stage B: the room's per-bin distribution

`cargo lab gates distribution` pools a set's screened pre-roll windows per bin —
the windows the arms score as silence, each ending four hops before the strike —
and reads six things per band of bins against the exact law for stationary
Gaussian noise through these overlapping frames: the **within-block law** (each
window's power against the mean of its block's other windows, the leave-one-out
ratio whose exact law through the 7/8 overlap is solved per block), the
**level's wander** between blocks (Oppenheimer's steadiness M = μ²/σ² of the
blocks' mean powers against the blocks' own ν/2, 1.0 for a level that does not
move), the **finite-variance** statistics A(1) and C(n, 1) of Skowronek et al.,
the **spectral kurtosis** per bin with Antoni's bands at the pool's own count of
independent frames, the **tail** of each bin's power over its pooled mean at
p = 0.1, 10⁻², 10⁻³, and the **location** three ways. A block is one capture's
pre-roll; ν/2 is Percival & Walden's equivalent count of independent
periodograms for Hann frames at an eighth-window hop. Run once on the control,
the three capture sets, and two MAPS ambient stores (one piano, one room and one
session each); no room recording existed on this tree, so the `--recording` path
is exercised by the control's pre-roll only.

| set | windows per capture | blocks | ν/2 per block | within-block rows inside | M ÷ (ν/2), bands ≥ 344 Hz | level spread, dB (stationary) | SK inside, ≥ 344 Hz | tail rows inside | 10⁻³ multiplier, ≥ 1376 Hz |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| T1 (control) | 32 | 100 | 8.63 | 34 / 37 | 0.99–1.04 | 1.5 (1.52) | 95–96 % | 26 / 27 | 0.985–1.002 |
| piano #2 | 4 (1–4) | 588 | 1.39 | 18 / 36 | 0.012–0.056 | 5.0–6.4 (4.4) | 0 % | 2 / 26 | 8.3–10.1 |
| extended | 3 (1–8) | 588 | 1.29 | 15 / 36 | 0.027–0.302 | 5.3–6.6 (4.7) | 0 % | 2 / 26 | 2.5–12.2 |
| piano #1 | 4 (3–4) | 87 | 1.39 | 30 / 33 | 0.14–0.59 | 4.2–5.4 (4.4) | 5–37 % | 5 / 23 | 4.9–6.7 |
| MAPS Am, ambient F | 11 (6–12) | 48 | 3.25 | 29 / 34 | 1.02–1.21 | 2.3–2.5 (2.6) | 90–97 % | 18 / 24 | 0.98–1.00 |
| MAPS Cl, ambient F | 11 (10–12) | 40 | 3.20 | 29 / 34 | 1.05–1.10 | 2.4–2.5 (2.6) | 88–97 % | 18 / 24 | 0.98–0.99 |

"Rows inside" counts band × p-point rows whose realised exceedance lies in its
95 % Clopper–Pearson interval around the exact nominal (about 5 % fall outside
by chance). The level spread is the standard deviation across blocks of a bin's
block-mean power in dB, with the value a stationary level would show from
sampling alone in brackets. The 10⁻³ multiplier is the pooled distribution's
0.999 quantile over the exponential law's −ln 10⁻³, the factor a known-mean
threshold would need. Per-band tables are in `out/distribution/` of the run
record, with one CSV of per-bin statistics per set.

**The instrument is calibrated.** On the control every law holds at its exact
nominal: the within-block ratio through the overlap (the exact law gives
1.13 × 10⁻³ at p = 10⁻² for 32 windows, and 34 of 37 rows sit on it), the
steadiness of a stationary level (M ÷ (ν/2) = 1.009 at 32 windows per block,
ν/2 = 8.630 analytic), the finite-variance region, the kurtosis bands at the
pool's own K, the tail to 7 × 10⁻⁶, and the location to 0.00 dB. The
multiplier a known-mean 10⁻³ threshold needs on white noise reads 0.985–1.034
per band, which is 1.

**The MAPS rooms are stationary Gaussian noise above 344 Hz** at the pooled
bins: M ÷ (ν/2) within 1.02–1.21 of 1, the spectral kurtosis inside Antoni's
band on 88–97 % of bins, the pooled 10⁻³ tail at 0.98–1.00 of the exponential
law. Below 344 Hz both stores' levels wander (M ÷ (ν/2) 0.20–0.80) and the
kurtosis calls 34–78 % of bins not Gaussian: rumble and hum, which the
finite-variance test flags as jumps in the same bands.

**The owner's two capture sets are not one distribution per bin across their
captures.** Between captures a bin's level moves with a standard deviation of
5–6.6 dB on piano #2 and the extended set, against 4.4–4.7 dB a stationary
level would show from sampling at 3–4 windows per block, so the between-capture
variance of the level is 5–80× the sampling variance (M ÷ (ν/2) 0.012–0.30);
the dB spread says a few decibels of movement for most captures, the linear
ratio says a heavy upper tail of much louder ones.
Pooled over captures, the spectral kurtosis reads "not Gaussian" at every bin
from 43 Hz to Nyquist (median SK 2–80 against a band of ±0.12): the signature
of a mixture of levels, not of a non-Gaussian process within a capture. The
pooled tail says the same in the units Phase II will use: a 10⁻³ threshold set
on a bin's mean over the set's pre-rolls would have to sit 8–18× the
exponential law's −ln p above it on piano #2 (2.5–15× on the extended set,
4.9–6.7× on piano #1), which is the number the rulebook's slot 8 asked for, and
it is a property of pooling across captures, not of the noise at a bin. The
Gatekeeper's silence screen is part of the picture: it removes the pre-roll
windows whose 344–1376 Hz level is high, so the screened population's mean sits
2.3–2.8 dB under the unscreened one in those bands and within 0.3 dB elsewhere.
Piano #1's 87 captures, recorded in one sitting, wander least (M ÷ (ν/2) up to
0.59, 5–37 % of bins inside the kurtosis band).

**Why the level moves is not measured here**, and two confounds stand between
the statistic and the room. The sets span sittings: the calibration σ the app
logged ranges over 8.6 dB across piano #2's captures and 30 dB across the
extended store's, so the pool mixes ambient levels from different hours and
days. And the pool mixes keys: §3 shows a repeated key's pre-roll carries that
key's own ringing partials, so at any bin the captures whose key has a partial
there are louder than the rest — a mixture of levels across captures that the
kurtosis test cannot tell from a moving room, and a heavy upper tail by
construction. The MAPS stores, one sitting each and single strikes with silence
between them, have neither confound, which is part of why they read as
stationary. The separation is cheap and belongs at the head of Phase II:
group the captures by sitting (the logged σ, or the serials), leave each
capture's own partial lobes out of its pooled bins, and read the steadiness
again; if it recovers toward 1 within a sitting, the rooms are steady and the
movement is sittings plus strings, and the minute-long recordings answer the
rest.

**Within one capture the instrument has little power.** A real pre-roll holds
three or four screened windows (ν/2 ≈ 1.3–1.5), so the within-block rows at
p = 0.5 and 0.1 are inside or marginal while the 10⁻² and 10⁻³ rows above
1376 Hz exceed their nominal by 3–40× (piano #2, 5504–11008 Hz: 3.9 × 10⁻⁴
against 1.0 × 10⁻⁵ at p = 10⁻²). Either the level moves within the four
windows too — a rise before the strike that the four-hop guard does not clear —
or lines sit in those bins; the per-capture pre-roll cannot tell them apart,
and the minute-long room recordings the protocol's stage 0 asks for (now
recordable with `cargo lab audio record`, none on this tree) are what decides
it.

**What Phase II inherits.** A per-bin level learned from other captures — a
pooled floor, a room profile — carries a 5–6.6 dB standard deviation per bin on
these sets before any estimator's own variance, and its 10⁻³ threshold needs
the pooled multiplier above; the MAPS stores would accept such a profile and
these sets would not, whichever of the confounds above is doing the moving. A floor learned from the capture's own pre-roll
has four windows, whose finite-N law at 10⁻³ is F(2, 2·1.47), a multiplier of
about 24 on the mean against the known-σ law's 6.9 — the law arm 1 applies, and
the width its T ÷ T_ideal spread shows on the real sets. The arms are read
against the distribution measured here, not against an assumed one.

## 5. Arms available to Phase II, and the ones that are not

Every arm below ran on the control (T1), on T2a and on the three capture sets
with one driver (`out/final/` in the run record; the fifteen arms of the first
two tables on the binary with SHA-256 `10b28972…`, the three of the last on
`c231bc5b…`, which differ only by the arms added). The
first table is the verdict Phase I owes: whether the arm's realised false-alarm
rate on the control sits inside the 95 % Clopper–Pearson interval of its
nominal (§2's rule), what limit the control or the arm's own derivation puts
on it, and its cost per hop in release. The second table is the shape of the
arms on real notes, **exploratory**: the real sets' truth is the oracle (valid
from the tenor up, biased high in the bass by leakage), the "live" population
is defined against it, the silence columns carry the key's ringing strings
(§3), and nothing here is a decision; Phase II scores these with its decision
rules. The T2a column is scored against the exact level of §3.

| arm | consumers | T1 verdict | available to Phase II | the limit the control states | cost |
| --- | --- | --- | --- | --- | --- |
| **A** shipped scalar at the recorded σ | G1 G2 G3 BG | below nominal everywhere (0 / 18 240 lobes, 3 / 19.9 M bins, 0 / 8 640 probes) | yes, as the baseline it is | the calibration rule's 1.5× over-rejects by design (§2); on real rooms the broadband σ sits 20–26 dB over the per-bin level from the tenor up and under it in the bass | shipped |
| **A@1e-3** the same at the true σ | G1 G2 G3 BG | inside on every population (the exact threshold) | as T1's exact reference only | meaningless on a real room (its σ is white noise's) | shipped |
| **B** oracle / exact | G1 G2 G3 | inside (= the exact threshold on T1) | the bound every arm is read against, tenor up on real sets, everywhere on T2 | on T2 it is the exact gate at the probe's window (§3) | — |
| **C** shipped OS-CFAR per bin | G1 | below: 1.10 × 10⁻⁵ per bin, 0 lobes | yes, at its own operating point | its budget is `P_fa/M` per band (0011 §5), 80× stricter per bin than the others; declines without the key's spacing, which the picker lacks | in `peaks::coarse_read` |
| **N** no gate | G1 G2 G3 BG | 0.276 maxima per bin, 1.0 on probes | the geometry's reference only | — | — |
| **1** running per-bin floor | G1 G2 G3 BG | inside on every population; T ÷ T_ideal 1.01–1.05 | yes | declines until 8 silent hops are learned; the finite-N law F(2, 2M) with M summed over contiguous runs; a shape from few short runs puts the threshold high (the Gamma model's lighter lower tail, 0.98 of nominal in the agent's simulation) | ≈ 10 µs |
| **finst** phase-increment consistency | G2 G3 | inside: 5.7 × 10⁻⁴ [2.6, 10.9]; bass 3.4 × 10⁻⁴, high 1.02 × 10⁻³ | yes, as a signal-side test | needs eight hops of history (declines the first 8 of every probe) and a numerical null per window (Z ≥ 5.774 at 1024, 7.610 at 4096); a level it does not estimate | 2 Goertzels per probe |
| **ftest** Thomson's harmonic F-test, NW = 3, K = 5 | G1 (decision only) G2 G3 | G1 lobes inside (74 / 18 240); scans **above** by 6 % (1.064, 1.015 × 10⁻³ against the exact's 0.890, 0.902) — the control's own windows read 6 % high at every bin position (seed), the whole noise stream 1.001 × 10⁻³; G2/G3 inside | yes, with its reach stated | a noise-free line more than 0.273 bin (1.47 Hz) from a bin centre never reaches the 10⁻³ threshold, so the bin-centred scan **misses 45.7 % of T1's present partials at every SNR**; a 4× zero-padded grid is the second-pass variant; λ₄ = 0.946 at NW = 3 (the spec's "all > 0.99" was wrong) | 118 µs (five 8192-point FFTs) |
| **fcme** forward consecutive mean excision on the whitened frame | G1 G2 G3 | lobes inside; scan_silence inside; **scan below** at 0.962 of nominal (8.68 × 10⁻⁴); G2/G3 inside | yes | the same Gamma-model conservativeness as arm 1 on a young shape (0.39 of nominal at the first capture, 0.97 by the twentieth); three derived corrections (truncation `c(τ)`, whitening `κ(M_s)`, the combined finite-N `M`) each pinned by Monte Carlo; the spec's literal law ran 1.29× nominal and was not kept | 113 µs |
| **sb9** neighbour bins, numerical null | G2 G3 | absent **below** (3.7 × 10⁻⁴; the exact on the same probes 9.7 × 10⁻⁴), silence **above** (1.94 × 10⁻³ = the exact's own 13 / 6 720) | yes, with its null's scatter stated | the 40 000-trial null's 10⁻³ point scatters 2.2 % (SD) from seed to seed, the arm's 1024 draw sits +3 % over the converged value (a realised rate near 7.8 × 10⁻⁴); the converged `k` is +8 % over Rohling's independent-cell value (Hann cells one bin apart correlate 4/9); declines with fewer than four clear cells (C1 and C3 on T1) | 27 µs per probe at 1024, 68 at 4096; 458 µs per hop |
| **laurenti** reciprocal-mean floor | G1 BG | G1 **above** at about 2× everywhere (6.4 × 10⁻³ lobes, 1.83–1.94 × 10⁻³ per bin); BG inside | yes, with k₁ stated | the paper's k₁ = 0.90 is for independent bins; on Hann frames the estimator reads 0.852 of the mean magnitude, so the threshold sits 5 % low — a Hann-derived k₁ would be a derivation, not a tuning, and is Phase II's to make; G1 declines under a spacing of 8 bins (A0–E1 at 8192 points); T ÷ T_ideal 0.96–0.98 tenor up, 1.11 in the bass (the density bias) | one pass over the frame |
| **bgmed** gap median at 2¹⁶, Rayleigh threshold | BG | inside (1 / 320; the exact 2 / 320); T ÷ T_ideal 0.99–1.01 | yes | the oracle's construction at MAT's resolution; declines where both gaps hold fewer than three bins | one median per partial |
| **bgmed-iso6 / -iso15 / -opp** the same floor at the ISO and Oppenheimer margins | BG | 53 / 320, 0 / 320 and 36 / 320 of absent partials accepted — above, inside, above | as validity margins, which they are | 2.07×, 6.64× and 2.19× the median are acceptance margins on a measured level, not operating points; ISO's ratio is partial-to-noise, Oppenheimer's measured-over-background (one footing would put ISO 6 dB at 2.40×) | — |

Shape on real notes, exploratory (G3 unless marked; miss at SNR_gate ≥ 3× as
pooled % on piano #2 / extended / piano #1, then the high register 76–87 on
piano #2; silence = the admitted share of screened pre-roll probes on piano #2;
Q-A = median 20 lg(estimate ÷ oracle) in the 0.5–1 s bucket, tenor / treble;
"—" where the arm has no such row):

| arm | miss ≥ 3×, pooled | high 76–87 | silence | Q-A tenor / treble, dB | T2a: miss pooled, silence |
| --- | --- | --- | --- | --- | --- |
| A | 22.1 / 52.9 / 13.7 | 75.0 | 6.1 % (bass 17.0 %) | +20.8 / +25.7 | 21.4 %, 6.9 % |
| B (bound) | 0.5 / 1.2 / 0.6 | 2.8 | — (no oracle in silence) | 0 by construction | 0.4 %, 9.4 % (the ringing strings, §3) |
| C (G1) | 3.1 / 6.4 / 4.1 | 12.2 | — | +2.5 / +2.2 | 4.9 %, — |
| 1 | 5.2 / 6.9 / 1.9 | 52.1 | 3.2 % | +1.8 / +6.5; coverage 1.5–8 % | 4.5 %, 3.1 % |
| finst | 18.4 / 9.8 / 11.2 | 62.8 | — | — (H₀ admits 1.6 %) | 25.1 %, — |
| ftest (G3) | 40.0 / 19.6 / 27.1 | 83.5 | 4.9 % | — (decision only) | 33.9 %, 4.6 % |
| fcme | 2.8 / 5.7 / 2.3 | 44.5 | 4.7 % | −1.2 / +1.0; coverage 7–11 % | 6.9 %, 3.7 % |
| sb9 | 6.0 / 13.7 / 6.0 | 45.4 | 3.5 % | +2.9 / +1.1; coverage 62–73 % | 8.0 %, 2.3 % |
| laurenti (G1) | 1.7 / 2.3 / 1.3 | 27.4 | — | +0.3 / −0.3 | 1.1 %, — |

Three things stand out, all for Phase II to test rather than for this report to
decide. The frequency-local arms that read the noise beside the partial in the
same frame (laurenti, fcme, C, sb9) sit within a few dB of the oracle on real
notes and miss 2–6 % of live partials against the shipped gate's 22–53 %; the
time arm (1) does the same from the tenor up but reads 7–15 dB under the oracle
in the bass, where the oracle is the biased one. The top octave is every arm's
hard case: 27–84 % of its live partials are missed at ≥ 3×, against the
oracle's own 2.8 %. And the F-test's miss rates are the bin-centre reach of §2,
not a noise question. On the control the stated-interval arms cover the truth
at 81–91 % (sb9, 1, fcme); on real notes their coverage falls to 2–30 % (1,
fcme) and 46–78 % (sb9), which is the room's non-stationarity read through the
oracle's own bias, and the measurement Phase II's Q-A coverage starts from.

The three arms built last (the sub-agents that built them were cut off by a
session limit and resumed; their files were read in full before any number
below was taken):

| arm | consumers | T1 verdict | available to Phase II | the limit the control states | cost |
| --- | --- | --- | --- | --- | --- |
| **7** empirical-null scalar on the running floor | G1 G2 G3 | lobes inside (53 / 18 240), scan_silence inside (8.71 × 10⁻⁴); **scan below** at 0.950 of the exact (8.58 × 10⁻⁴); G2/G3 inside | yes | the location is read from the first quartile of the standardised bins and a present partial lifts it: T1's four 30 dB partials move δ̂ +0.035 dB on note hops against +0.005 dB on silent ones, which at the tail's log-slope of 6.8 predicts 0.946 of nominal; "the left half is immune" holds to first order only (a shift of about 1.16ε in ln power for a share ε of bins, +0.38 dB for an A0 note at 7 %); two derived corrections (the finite-floor null quartile `ln(M((4/3)^{1/M} − 1))`, and Var δ̂ from Bahadur's representation with Kibble's covariances at the Hann correlations, 0.158 dB SD) replace the spec's literal form, whose two errors roughly cancel on T1's schedule | 16 µs |
| **apes** APES residual at the probe's frequency | G2 G3 | absent 21 / 17 920 = 1.17 × 10⁻³ inside; silence 5 / 8 640 inside | yes, with its model-mismatch curve stated | the estimate rests on about four effective periodograms, so the known-σ multiplier the spec named realises 1.03 × 10⁻² at 1024 and the threshold is a numerical null at the probe's window (4.215 at 1024, 3.122 at 4096; Part 1 §5's rule); the residual carries any line the model misses in full: a 30 dB line reads +10.6 dB at 0.1 bin of model error, +23.8 dB at 0.5 bin — on T1 the tracker's f_inst error alone costs 35 % of live partials at ≥ 3× and +9 dB of Q-A, against 0.0 % and ±0.4 dB with the target's frequency exact (the author's counterfactual build) | 138–186 µs per probe; the null 11 s per set |
| **shape** Zivanovic–Röbel–Rodet peak classifier after arm A | G1 (second stage) | at the recorded σ identical to A (3 candidates in 19.9 M bins, all kept; 7 of 22 800 present partials dropped); at the true σ it removes 11.6 % of A's note-scan candidates (9.02 → 7.98 × 10⁻⁴ per bin), 10.2 % on the pre-roll, 4 of 60 absent lobes | yes, as a second stage, with its real-note cost stated | the spec misread the paper: Table 3's polynomials are the modulation indices of Eq. 14's noise patterns, and the limits come from the paper's model (NBD_max from the worst-case sinusoid, NDD_min/NDD_max from the signal patterns with the noise patterns added at the peak's SNR_p), which the arm tabulates and which reproduces the paper's Figure 5 at the paper's geometry; 81 % of white-noise candidates read under 10 dB, where the paper's fit stops and the limits are held; on real notes the high-SNR limits, built for vibrato-style modulation, reject 30–40 % of A's admitted real partials (too wide 36 516, off-duration 55 497 of 253 750 partial-lobe peaks on piano #2 at a mean SNR_p of 41 dB) | 258 µs per hop (three 32 768-point DFTs), 1 µs per peak |

| arm | miss ≥ 3×, pooled | high 76–87 | silence | Q-A tenor / treble, dB | T2a: miss pooled, silence |
| --- | --- | --- | --- | --- | --- |
| 7 | 2.1 / 4.7 / 2.1 | 40.1 | 6.6 % | −2.4 / −0.7; coverage 7–11 % | 6.0 %, 4.6 % |
| apes | 51.4 / 38.9 / 42.0 | 96.9 | 2.4 % | +28.8 / +21.2 | 45.5 %, 2.9 % |
| shape (G1) | 46.6 / 48.9 / 33.7 (A alone: 16.1 / 41.0 / 11.1) | 92.8 | — | A's | 37.3 %, — |

Arm 7's δ̂ on piano #2 reads −5.1 dB in median on note hops (−5.2 dB on the
silent ones): a floor pooled over the set's captures sits above a typical hop
by the between-capture movement §4 measured, and the scalar takes it out per
hop. On T2a and the real sets it holds arm 1's miss rates with a worse
false-alarm rate on silence (6.6 % against 3.2 %, the ringing strings read
through a lower threshold). APES on real notes is the card's stated failure
(model mismatch: the tracker's f_inst, beating, decay) and the sweep above is
its calibration curve; it is available to Phase II as the estimator whose bias
grows with SNR exactly as the amendment predicted, and as a test of how good a
frequency the tracker hands it. The peak-shape classifier removes a quarter of
the shipped scalar's false candidates and a third of its real partials; as the
paper's limits stand it is not a picker stage for the piano, and Phase II's
question for it is whether the limits re-derived on piano partials' own
modulation (beating at 0.04–39 Hz, §3) separate anything.

**The lock replay.** `cargo lab engine replay` runs each capture's full event
through the shipped front end at phases of the hop grid and locks it either
with `Engine::process` itself or with the lab's replica of its discovery state
machine, which takes a G1 arm's per-bin threshold where the engine applies its
one scalar; with an arm, the engine runs beside the replica on the same hops
and every (capture, cut) is compared. Its checks: at one phase it reproduces
`engine lock` capture for capture, 568 / 595 on piano #2 and 82 / 87 on
piano #1 with the identical failure lists (567 / 595 and 85 / 87 with
`--regen`); under arm A, the shipped scalar at the recorded σ, the replica
commits the same key on the same hop as the engine on 9 520 of 9 520 (capture,
cut) pairs on piano #2 and 1 392 of 1 392 on piano #1, the CSVs byte for byte.
At sixteen phases on piano #2 the yardstick is **564–571, mean 566.94,
σ 2.16** (no-lock 8–14, wrong 13–20; 555 captures pass at every cut, 19 at
none, 21 are decided by the grid, 15 of them in keys 75–87), and all 9 520
rows match run O's outcomes, commit frames and latencies to the CSV's
rounding. That is report 0020's *vote from the onset* column, which this tree
ships (253ce22); Part 1 §4 quoted its shipped-rule column (567.19 ± 2.04) as
the baseline, which was the wrong column for this tree, and Phase III's
yardstick is 566.94 ± 2.16. The shape of the swap, exploratory and deciding
nothing: with no gate (arm N) 524–529 passes (526.69 ± 1.49), the 128-maxima
cap binding on every hop; the running floor restarted per capture (arm 1)
5–258, because a fresh floor has fewer than eight silent hops before the strike
at most cuts and a declined hop votes A0; the same floor carried across
captures (`--carry`) 523–528 (525.81 ± 1.91); arm C at one cut 525 passes
without the key's spacing and 508 with it. 1.8 s per cut on piano #2, 2.5 s
with arm A.

## 6. Amendments

None. No bar of Part 1 was changed. Four things Phase I settled inside Part
1's words are recorded here so that no later reader mistakes them for
amendments:

1. **T2's exact level is per hop** (§3): Part 1 §2.2 says the noise is known
   exactly at every hop, which is the realisation; the level a threshold is set
   on is read from it as a 33-bin running median at the consumer's own window.
   The first scoring used a per-capture median and was discarded before any
   number from it was quoted. The construction was then calibrated on the
   control (§3): unbiased, ±2.2–2.7 dB, twice nominal on the exact rule from
   its jitter.
2. **The strike's one re-draw** (§3) is the re-draw Part 1 §2.2 provides for;
   the gain per register is derived from the record (the real captures' own
   first-bucket excess, measured by the same probe) and stated in the code with
   its derivation. The bass remains short of the real excess and no second
   re-draw is made.
3. **A transcription error in Part 1 §4, corrected.** The rulebook states the
   ISO 3744 margins as partial-to-noise power ratios 2.98 and 30.6 and nothing
   more; the amplitude conversion this report's Part 1 added read √(R ln 2)
   = 1.44 and 4.61 where the arithmetic gives √(R / ln 2) = 2.07 and 6.64, and
   the driver's "ISO 6 dB / 15 dB" miss columns were cut there in the first
   runs. The frequency-local arm's author caught it; the driver and the line
   are corrected and every number in Part 2 is from runs after the correction.
   The bar (the power ratios) did not move. After the error every number in
   Part 1 was re-checked against the frozen rulebook: of 100 distinct numbers,
   83 appear in it verbatim, 14 are the citations' years, two are the
   corrected multipliers above, and one is the register boundary 52 that the
   rulebook names through report 0015 and `capture::strobe_register` carries;
   none is unexplained. Part 1's prose was not re-read against the
   rulebook; it was carried in substance, not verbatim, by design.
4. **The pre-roll guard counts from the −20 dB onset** (Part 2 §1), not from the
   record's start: the record begins at the Gatekeeper's `Stable`, five to six
   hops after the strike on piano #2, and a guard counted from it admitted one
   or two strike hops into the silence population (9 of 800 screened windows on
   the first pass). Part 1 §2.2 T5(i) says Gatekeeper-silent pre-roll hops; the
   guard is how the harness honours the strike's lead.

## Limitations

- **The T2 exact level is a local median**, read over 33 Hann bins at the
  consumer's window. Calibrated on the control (§3): no bias, ±2.2–2.7 dB at
  5–95 % per hop, so every T2 spread (Q-A's 5–95 %, coverage) is wider by that
  than the truth; the exact rule read through it fires at about twice nominal
  on white noise from the jitter alone, and a line finer than the median's
  span (178 Hz at 8192 points, 690 Hz at 1024) is not in the level. Its bias
  on a coloured spectrum, the bass above all, is not measured; a coloured-noise
  control would measure it. The bed's own departure from Rayleigh around the
  median, the construction's share removed, is about 2.5× the law's rate at
  10⁻³ (§3), and is the floor under every T2 false-alarm figure.
- **T2a's hammer is a white burst scaled per register**; after the re-draw its
  excess matches the real one in three registers and reaches 6.0× against 8.0×
  in the bass. T2a's first bucket is therefore quieter than a real bass note's,
  and the synthetic record begins at its strike where a real one begins 116–139
  ms after it. First-bucket figures on T2a are read with that offset.
- **T2a's hammer was matched at one bucket only.** The re-draw fixed the
  gap excess at 0.1–0.25 s of record time per register; the excess at later
  buckets, and the burst's time course and spectral colour within a register,
  were not compared with the real captures. T2b keeps the real hammer (its
  excess reads 8.01 / 3.87 / 2.38 / 1.60× against the real 8.02 / 4.04 / 2.32 /
  1.57×), so a first-bucket question is read on T2b or on the real sets, and
  T2a's first 0.25 s serves Q-C's time axis, not the onset's noise.
- **T2b is not noise at the partials** (remnant +8 to +31 dB), so T5(iii) under
  T2b and any read at a partial's bin on T2b are unavailable; T2b serves the
  gaps, the pre-roll and the sustain's background only.
- **The screened pre-roll of a repeated key is not silence at that key's
  partials** (§3). Every "silence" column in this report carries the key's own
  ringing strings; Phase II's H₀ population in the pre-roll must be off the
  key's partials or on a key's first capture, and 0015 §10's silence figures
  are re-read accordingly.
- **A real pre-roll holds three or four screened windows** (ν/2 ≈ 1.4), so
  Stage B's within-capture tests have little power and the room's law within a
  capture — stationary or not, Gaussian or not — is open until the minute-long
  room recordings exist. Between captures the level's movement is measured
  (5–6.6 dB SD per bin) and that is the number Phase II inherits.
- **Two rooms, one session each** on the owner's side (piano #2 and the extended
  store are the same room weeks apart; piano #1 one sitting), and two MAPS
  stores as the stationary reference. Nothing here says what a third room does.
- **The control is white noise**; T1 calibrates the geometry and the
  conversions, not an arm's behaviour on a coloured floor. Arm C's per-band
  budget (1.1 × 10⁻⁵ per bin at a 10⁻³ operating point) and the F-test's
  bin-centre reach (0.273 bin at NW = 3) are operating-point facts the control
  surfaced; Phase II's sweep puts them on one axis.
- **The lock replay** is checked for parity against the shipped engine at one
  phase and for 0020's yardstick at sixteen; its arm-driven variant replicates
  the engine's state machine in the lab and a disagreement there is the
  replica's bug by rule. Nothing it produces in Phase I decides anything.
- **How far the arms were verified.** Each delegated module was read in full
  by the executor, its tests pass, and its arm sits inside nominal on the
  control or has its departure explained; its mathematics was checked for
  internal consistency and against the module's own Monte Carlo tests, not
  re-derived independently. That applies to the excision arm's whitening bias
  κ, the empirical null's Bahadur–Kibble variance, the APES expected
  covariance and the peak classifier's limit model. A reader who needs more
  than that starts from those four derivations.
- **Every floor arm took the analytic Hann ν/2** (Percival & Walden) rather
  than a measured one. The control measured the analytic value within 0.9 %
  (§4, M ÷ (ν/2) = 1.009 at 32 windows), and no real set yields a per-frame
  value the arms could take in its place.
- **Nothing was committed by the executor**; the owner commits the tree this
  report describes, and the run record under `docs/old/` is not in the
  repository.

## Reproduce

All in release; paths relative to the repository root. The regen dumps are
`cargo lab mat regen <SET> <OUT>` of each set (the run record keeps them as
`sets/p2.json`, `sets/p2ext.json`, `sets/p1.json`).

```bash
cargo build --release -p tuner-lab
L=target/release/tuner-lab

# the parity checks (568/595, 82/87; 100.0000 % at both sizes)
$L engine lock diagnostics_piano2 && $L engine lock diagnostics_piano_1
$L gates verify diagnostics && $L gates verify diagnostics_piano_1 && $L gates verify diagnostics_piano2

# the control (T1) and the arms on it
$L gates truth t1 sets/t1
for a in A A@1e-3 B C N 1 finst ftest fcme sb9 laurenti bgmed 7 apes shape; do
  $L gates arms --arm $a --set t1 sets/t1/truth.json sets/t1 --out out/t1
done

# T2a (the re-drawn strike is the default) and T2b, mirrored from piano #2
EXT=~/.local/share/inharmonicity/diagnostics/01a010a9-dda1-7dda-a35b-8a3d6978d53e
$L gates truth t2 sets/p2.json diagnostics_piano2 sets/t2a --bed room --long-root $EXT --long-regen sets/p2ext.json --per-key 3
$L gates truth t2 sets/p2.json diagnostics_piano2 sets/t2b --bed residual --long-root $EXT --long-regen sets/p2ext.json --per-key 3
$L gates arms --arm A --set t2a sets/t2a/truth.json sets/t2a --out out/t2a   # and each arm as above

# the real sets (arm A reproduces 0015 §5–7; --preroll strict reproduces 0015 §10's population)
$L gates arms --arm A --set p2 sets/p2.json diagnostics_piano2 --set ext sets/p2ext.json $EXT --set p1 sets/p1.json diagnostics_piano_1 --out out/real

# Stage B
$L gates distribution --set t1 sets/t1 1.56e-3 --set p2 diagnostics_piano2 - --set ext $EXT - \
  --set p1 diagnostics_piano_1 - --set maps_am <stores>/maps_ENSTDkAm_F - --set maps_cl <stores>/maps_ENSTDkCl_F - --out out/distribution

# the lock replay (parity at one phase, the yardstick at sixteen)
$L engine replay diagnostics_piano2 --phases 1
$L engine replay diagnostics_piano2 --phases 1 --peak-arm A
$L engine replay diagnostics_piano2 --phases 16
```

The protocol's freeze is checked with `sha256sum -c` over the two hashes in
this report's head; the seeds are the modes' defaults (T1 seed 20261008, T2
seed 20261009), so every synthetic set regenerates byte for byte.

## References

- Antoni, J. (2006). The spectral kurtosis: a useful tool for characterising
  non-stationary signals. *Mechanical Systems and Signal Processing* 20(2),
  282–307 — the STFT-form estimator (Eq. 40) and its variance for a stationary
  (Eq. 44) and a Gaussian-driven non-stationary background (Eq. 43).
- Boll, S. F. (1979). Suppression of acoustic noise in speech using spectral
  subtraction. *IEEE Trans. ASSP* 27(2), 113–120 — the noise profile learned in
  the pauses.
- Clopper, C. J. & Pearson, E. S. (1934). The use of confidence or fiducial
  limits illustrated in the case of the binomial. *Biometrika* 26(4), 404–413 —
  the exact interval every realised rate carries.
- Efron, B. (2004). Large-scale simultaneous hypothesis testing: the choice of
  a null hypothesis. *JASA* 99(465), 96–104 — the empirical null fitted to the
  bulk.
- Finn, H. M. & Johnson, R. S. (1968). Adaptive detection mode with threshold
  control as a function of spatially sampled clutter-level estimates. *RCA
  Review* 29(3), 414–464 — the cell-averaging predecessor of the OS-CFAR.
- Greenwood, J. A. & Durand, D. (1955). The distribution of length and
  components of the sum of n random unit vectors. *Annals of Mathematical
  Statistics* 26(2), 233–246 — the series for Rayleigh's test, the reference
  the frequency-consistency arm's numerical null is checked against.
- Hendriks, R. C., Heusdens, R., Jensen, J. & Kjems, U. (2009). Low complexity
  DFT-domain noise PSD tracking using high-resolution periodograms. *EURASIP J.
  Adv. Signal Process.* 2009:925870. DOI 10.1155/2009/925870 — the self-gated
  sub-bin update, a variant of the time family.
- Johnson, T. A. et al. (2010). Clinical test performance of distortion-product
  otoacoustic emissions using new stimulus conditions. *Ear & Hearing* 31(1),
  74 — the noise at a test frequency from the bins on either side.
- Kay, S. M. (1998). *Fundamentals of Statistical Signal Processing: Detection
  Theory*, Ch. 9 — the Neyman–Pearson magnitude threshold the shipped gates
  use.
- Laurenti, N., De Poli, G. & Montagner, D. (2007). A nonlinear method for
  stochastic spectrum estimation in the modeling of musical sounds. *IEEE
  Trans. Audio, Speech and Language Processing* 15(2), 531–541. DOI
  10.1109/TASL.2006.881685 — the reciprocal-mean floor, its constant k₁ = 0.90
  and its 8-bin spacing bound.
- Nitzberg, R. (1986). Clutter map CFAR analysis. *IEEE Trans. AES* 22(4),
  419–421 — the gated recursive per-cell floor.
- Nørholm, S. M., Jensen, J. R. & Christensen, M. G. (2016). Enhancement and
  noise statistics estimation for non-stationary voiced speech. *IEEE/ACM
  Trans. Audio, Speech and Language Processing* 24(4), 645–658. DOI
  10.1109/TASLP.2016.2514492 — the APES residual as a noise estimate; and the
  same authors, EUSIPCO 2013, on the cost of a mismatched harmonic model.
- Nunes, L. de O., Esquef, P. A. A. & Biscainho, L. W. P. (2007). Evaluation of
  threshold-based algorithms for detection of spectral peaks in audio. *5th AES
  Brazil Congress* — the γ, ζ, κ scoring and its reference-alignment rule.
- Oppenheimer, A. (2016). Statistical background noise correction for
  environmental sound level measurements (the steadiness M = μ²/σ² and the
  correction's validity margin), as read for the protocol.
- Percival, D. B. & Walden, A. T. (1993). *Spectral Analysis for Physical
  Applications*, Cambridge — the equivalent degrees of freedom of overlapped
  Hann frames (§6) and the discrete prolate spheroidal sequences (§8.3).
- Rohling, H. (1983). Radar CFAR thresholding in clutter and multiple target
  situations. *IEEE Trans. AES* 19(4), 608–621. DOI 10.1109/TAES.1983.309350 —
  Eqs. 14 and 17, the order-statistic law of arm C.
- Serra, X. & Smith, J. O. (1990). Spectral modeling synthesis: a sound
  analysis/synthesis system based on a deterministic plus stochastic
  decomposition. *Computer Music Journal* 14(4), 12–24 — T2b's residual.
- Shan, T., Tao, R., Wang, Y. & Zhou, S. (2002). Performance of order statistic
  clutter map CFAR. *ICSP 2002*, vol. 2, 1572–1575 — the order-statistic
  clutter map, a variant of the time family.
- Skowronek, K., Zimroz, R. & Wyłomańska, A. (2024). Testing for finite variance
  with applications to vibration signals from rotating machines. *Journal of
  Mathematics in Industry* 14:19. DOI 10.1186/s13362-024-00157-6 — the
  empirical cumulative even moment C(k, N) and the slope statistic A(N).
- Sulis, S., Mary, D. & Bigot, L. (2017). A study of periodograms standardized
  using training data sets and application to exoplanet detection. *IEEE Trans.
  Signal Processing* 65(8) — the exact law of a periodogram standardised by
  averaged training periodograms.
- Talbot, C. & Thrane, E. (2020). Gravitational-wave astronomy with an uncertain
  noise power spectral density. *Physical Review Research* 2, 043298 — the
  Student-Rayleigh law of a floor estimated from N periodograms, F(2, 2N).
- Thomson, D. J. (1982). Spectrum estimation and harmonic analysis. *Proc.
  IEEE* 70(9), 1055–1096. DOI 10.1109/PROC.1982.12433 — §XIII, Eqs. 13.5 and
  13.10, the harmonic F-test and its F(2, 2K − 2) null.
- Towsey, M. (2017). The calculation of acoustic indices derived from
  long-duration recordings of the natural environment. QUT technical report —
  the histogram mode in the log domain.
- Vartiainen, J., Lehtomäki, J., Saarnisaari, H. & Juntti, M. (2010). Limits of
  detection for the consecutive mean excision algorithms. *CROWNCOM 2010* —
  forward consecutive mean excision.
- Zivanovic, M., Röbel, A. & Rodet, X. (2008). Adaptive threshold determination
  for spectral peak classification. *Computer Music Journal* 32(2), 57–67 — the
  peak-shape descriptors.
- ISO 3744 §4.2.1.1 — the 6 dB and 15 dB background margins read as power
  ratios 2.98 and 30.6.
- Reports 0009 (Analyses 7 and 9), 0011 (§5), 0014 (§2–3, §8a), 0015 (the
  measured defect and the oracle), 0020 (the replay and its yardstick), 0012
  (§9, the bench), and audit 13 (the OS-CFAR's reference-cell anatomy) — the
  record every anchor above points at.

