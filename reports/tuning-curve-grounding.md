# What the tuning curve is grounded on

Rationale for the curve layer, moved out of `ARCHITECTURE.md` by the
documentation refactor. It is not a measurement report and has no
pre-registration — it is the *argument* for why the curve is shaped as it is, and
which parts of it rest on measurement versus on a model.

The decisions it refers to are recorded in
[report 0007](0007-tuning-curve-regularization-geometry.md),
[report 0008](0008-giordano-layer-fidelity-derived-weights.md) and
[report 0009](0009-repeat-capture-noise-decomposition.md); their evidence
is in the `reports/` bodies of the same numbers.

Two claims get conflated when people ask whether a tuning curve is "right", and
this project can make only one of them. **Building a curve fitted to a measured
piano is a solved mechanism. Deciding which of the curves it can build is the
best one is not, and may not be solvable at all.** This section separates them,
because the first is defensible in detail and the second is honestly open.

### Where the instrument is characterised, and how strongly

The measure of how much a given piano — rather than the model — determines its
own curve is how far the shipped engine (d) departs from engine (a), the pure
parametric prior. Measured on instrument 2, a Young Chang F-108B upright:

| register | median \|d − a\| | max | what drives it |
| --- | --- | --- | --- |
| bass A0–B2 | **13.64 ¢** | 30.50 ¢ | 24–32 partials/key, $B$ repeatable to 0.14 % |
| tenor C3–G#4 | 1.86 ¢ | 4.89 ¢ | 0.16 % repeatability |
| mid A4–A#5 | 0.74 ¢ | 1.11 ¢ | 0.48 % repeatability |
| upper B5–G#6 | 1.12 ¢ | 1.14 ¢ | shrinkage handing over to the prior |
| top A6–C8 | 0.52 ¢ | 0.97 ¢ | model; 0.02 ¢ at C8 |

Read plainly: **the bottom five and a half octaves are this instrument's curve;
the top two are a model.** The residual in the top register is not information —
it is the smoother's tail reaching up from where the data stops.

Everything instrument-specific enters through four paths: the per-key measured
$B$ (inverse-variance shrunk, report 0009), the bass asymptote $\xi = (s_B, y_B)$
fitted by L1 to this piano's keys, the interval rows engine (d) builds wherever
both endpoints carry trustworthy $B$, and the amplitude-informed display partials.
The treble asymptote and the octave-type curve $\rho_\varphi$ are the two things
that are _not_ instrument-specific.

### How a measurement becomes curve $B$: two structures, not one

**Admission.** A capture enters the curve when it is trusted (manual mode, not a
partial unison), its $B$ is finite and positive, it carries at least two
partials, and Rigaud's Eq. 20 can solve an $F_0$ from them. That test is binary
and it is the only binary test: a 3-partial C8 is admitted exactly as a
32-partial A1 is. How much each is _believed_ is decided downstream, and
continuously.

**Structure 1 — the $B_\xi$ model (Rigaud Eqs. 7–8).** Each bridge's $B$ is an
exponential in key number, so on a log axis each is a straight line, and the
model is their sum:

$$B_\xi(m) = e^{s_B m + y_B} + e^{s_T m + y_T}$$

The treble pair $(s_T, y_T)$ is fixed at the paper's cross-piano values ("Why
the top octave is modelled, not measured", below). The bass pair is fitted to the
instrument by least absolute deviations in $\ln B$ (Eq. 29 — an L1 fit, chosen
by the paper because it behaves like a median: one wild key barely moves it)
over **every admitted key, with no cutoff.** Treble keys enter and cannot move
it, not because they are blocked but because the model gives them no lever: a
key's residual only responds to $(s_B, y_B)$ in proportion to the bass line's
share of $B_\xi$ there, and on instrument 2 that share is

| A0 | A1 | A2 | A#2 | A3 | A4 | A5 | A6 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 98 % | 90 % | 56 % | 52 % | 16 % | 2.6 % | 0.4 % | 0.1 % |

so the fit is effectively decided by A0–A3 and the crossover sits at B2.
Admitting everything costs nothing and avoids inventing a register cutoff, which
[`layering.md`](../docs/internals/layering.md) forbids. With no admitted keys
at all the model falls back to the medium-piano default pair. Two parameters:
this is the smooth backbone, not the curve.

**Structure 2 — the per-key blend (report 0009; ours, not Rigaud's).** Every
admitted key's curve-side $B$ is the precision-weighted combination of its own
measurement and the model's value at that key:

$$\ln B_\text{curve} = w\,\ln B_\text{meas} + (1-w)\,\ln B_\xi,\qquad
w = \frac{\sigma_p^2}{\sigma_p^2 + \sigma_m^2}$$

This is the textbook inverse-variance combination — the posterior mean of a
Gaussian prior and a Gaussian measurement — and what makes it more than a
formula is that **both variances are measured, not chosen.** $\sigma_m$ is the
capture's own repeat scatter, which falls steeply with the number of partials
the capture held; $\sigma_p$ is how far this piano's real strings sit from the
smooth model, self-calibrated per instrument from the keys whose measurement
noise is negligible. Both laws and their constants are
[report 0009](0009-repeat-capture-noise-decomposition.md).

The weight then asks one question per key: is this capture's noise smaller than
the real per-key deviation it is trying to resolve? A 32-partial bass capture
answers yes decisively ($w \approx 0.998$); a 4-partial treble capture answers no
($w \approx 0.06$) and is handed back to the model almost entirely; a key with no
capture at all takes $w = 0$. Nothing switches at a threshold —
"measurement-dominated" ($w \ge \tfrac12$, about seven partials) only grades keys
for the consumers below, and $B$ itself stays continuous across it. This replaced
a hard partial-count cutoff, and with it the boundary artifacts report 0007 had
flagged.

One limit to state plainly: $\sigma_m$ is a repeat *precision*, not an accuracy.
A self-consistent wrong series — a comb the seed mis-numbered — repeats perfectly
and enters at $w \approx 0.999$. The blend does not guard against that and
cannot; MAT's seed tolerance upstream does.

**The §2 eviction.** With the blended $B$ at both ends of every octave, Eq. 6
gives the beatless stretch of each octave pair. To first order in $B$ it is
negative only when $B_L(4\rho^2 - 1) < B_U(\rho^2 - 1)$ — the upper key would
need more than four to five times the lower key's $B$ within one octave (and at
$\rho = 1$ it can never happen at all: a 2:1 octave on a stiff string is always
stretched). Real pianos rise by at most ~3× per octave in the treble and *fall*
going down the bass, so a negative stretch certifies a wrong $B$ rather than a
strange string. The procedure: among the pair's measurement-dominated keys,
evict the one further from the model (larger $|\ln(B/B_\xi)|$) — hand it back
to $B_\xi$, mark it excluded — then re-check every pair, because an evicted key
is the lower note of one octave and the upper of another, until no negative
pair remains (tolerance 0.01 ¢). Flag-and-exclude, never clamp. On instrument 2
it evicts nothing. Separately, `finish()` re-checks the *final* curve for
$d(m{+}12) < d(m)$ and flags it, without fixing it.

**Who consumes which.** Engine (a) reads the model only: its Eq.-6 chain runs on
$B_\xi$, and it is the backbone every other engine starts from. Engines (b),
(c) and (d) all read the blend — (b) and (c) in their per-key chains, and
(d) in every interval row (`interval_width_cents(curve_b[m], curve_b[u], …)`),
admitted only where both endpoints are measurement-dominated and weighted by the
measured partial amplitudes. The blend is what makes the shipped curve this
piano's; the model is what fills in wherever the blend has nothing to say.

### Why the top octave is modelled, not measured

$B$ is estimated from how far partial $n$ sits above $n f_1$, so a capture
holding more partials resolves it better. Above about A6 the partials are not
there to hold. Measured on instrument 2, relative to each key's own
fundamental, a treble string's second partial sits **28 dB down** and its
fourth **50 dB down**, while a bass string puts *more* energy into its upper
partials than into $f_1$. A treble string radiates almost everything through the
one component that carries no inharmonicity information at all — and that
35–55 dB swing, not the sample rate or the band edge, is the binding constraint.

So the top two octaves are modelled rather than measured. Three things make that
a designed outcome rather than a gap:

- **The handover is continuous.** The blend above already weights every key by
  how well its own capture resolved $B$, so a treble key contributes exactly in
  proportion to what it resolved and the model supplies the rest. There is no
  register threshold to sit on the wrong side of, and nothing switches.
- **The borrowed half of $B_\xi$ is the half that is standardized.** Rigaud
  fixes the treble pair $(s_T, y_T)$ across pianos and fits only the bass pair
  per instrument, and the reason is physical rather than statistical: treble
  string design in this range is not constrained by the size of the case, so it
  is common across instruments, while the size constraint lands on the bass
  bridge — which is exactly the pair we do fit. That borrowed pair agrees with
  Young 1952's independent physics-based derivation to 1.9 %, and both of our
  uprights read its slope within ~2 SE, in the direction their own estimator
  bias predicts. An error in that pair, of the size those two checks leave open,
  is worth **under ~3 ¢ at C8**. That is the sensitivity to the *asymptote
  parameters*, conditional on the model form holding for the instrument in hand;
  it is not a bound on how far a given piano's treble $B$ may sit from the model,
  which is the sweep below.
- **The readout adds no second error on top of it.** A strobe reference is
  $f_n^\ast = n f_0^\ast\sqrt{1 + B n^2}$, so for $n > 1$ a wrong $B$ displaces
  the reference by about $n^2\Delta B/2$ — an error independent of, and on top
  of, any error in the target itself. Above key 48 the strobe and the coarse read
  both target $n = 1$, where $f_1^\ast = f_{ET}\cdot 2^{d/1200}$ contains no
  $B$, so that second path is closed. What this does *not* buy is immunity in the
  target: a wrong treble $B$ moves $d(m)$, and $d(m)$ is what the operator tunes
  to.

What it costs is bounded, and it lands in the target rather than being
compounded by the readout: sweeping treble $B$ across 0.5×–2× of the model moves
engine (d)'s C8 target by 29.9 ¢ — a pitch the operator then tunes to. Why the
partials are missing, why a higher sample rate would not recover them, and what
has been measured against it are in
[report 0009](0009-repeat-capture-noise-decomposition.md) analysis 7; the
check of the borrowed asymptote against our own instruments is in
[faithfulness-audit-06](retiring/faithfulness-audit-06-b-prior.md).

### Why this cannot over-tighten a string

Tension goes as $f^2$, so a target $\Delta$ cents from a string's current pitch
implies $\Delta T/T = 2^{2\Delta/1200} - 1$. The curve's own targets span
**−19.9 ¢ (A0) to +37.5 ¢ (C8)**, so the curve by itself never asks more than
**+4.4 %** of a string already at ET pitch, and the bass targets are _below_
pitch, i.e. loosening. Measured against instrument 2's as-found state with the
shipped engine, the mean change is **about +0.5 % across the instrument**
(median move +0.1 ¢); the largest single move is a top-octave string at roughly
+9–11 %, almost all of it the string's own drift, on keys whose as-found pitch
scatters ±15 ¢ between repeat captures. Piano wire is designed to sit at roughly
half to two-thirds of its breaking tension, so even that worst case moves a
healthy string from ~60 % to ~66 %.

Most of any large move is the string's accumulated drift, not the curve. The
hazards this software does not change are a large overall pitch raise (which the
A440-only limit can silently prescribe on a flat piano), and over-pulling past
the target on short treble strings where a small pin rotation covers many
cents.

### $\rho$ has a hard floor and a soft ceiling

$\rho$ indexes _which_ partial pair is made beatless: partial $2\rho$ of the
lower note against partial $\rho$ of the upper, so $\rho = 1$ is the 2:1 octave,
2 is 4:2, 3 is 6:3.

**$\rho < 1$ asks for a partial below the fundamental**, which does not exist.
Numerically it also under-stretches into compressed octaves (at
$\rho \le 0.25$ the A0–A1 stretch goes negative), which is exactly the
condition the §2 detector treats as an artifact. `StretchPreset::Low`
therefore floors at 1, and any future arbitrary-$\rho$ control must too.

The upper bound is musical, not mechanical, and binds far sooner. $\rho = 3$
already places A7 at **+78 ¢** — absurd by any tuning standard — at a tension
excursion of only +9.5 %. Tension does not become the binding concern until
$\rho \approx 6$–8, where A7 sits one to two semitones sharp. **Clamp to
$\rho \in [1, 3]$ on musical grounds; breakage is nowhere near.**

Where $\rho$ actually binds is the treble, and there it is unconstrained by
data: engine (c)'s calibration accepts **zero $\rho$ points above F4** (key 44),
and running the Eq.-6 chain at fixed treble $\rho$ puts A7 at +24.2 ¢
($\rho = 1$, pure 2:1) / +29.3 (shipped) / +46.3 ($\rho = 2$, pure 4:2) — a
**17 ¢ span, larger than the whole $B$ uncertainty.** Rigaud's own ±1 variants
(`StretchPreset::{Low, Mean, High}`, §IV.C.2) cover exactly that range. They
exist in `CurveParams` and are deliberately **not yet wired**, because choosing
between them is a listening judgment: the control waits on the auralization path
rather than shipping as an unexplained knob. `Mean` is the conservative default.

One property to know before it is exposed: **the presets are not symmetric in
the treble.** `Low` is $\rho - 1$ floored at 1, and treble $\rho$ is already
1.11, so `Low` lands on the floor — A7 = +24.2 ¢ against `Mean`'s +29.3 ¢, a
5 ¢ step — while `High` reaches ≈ +47 ¢, an 18 ¢ step. In the bass, where
$\rho \approx 4.4$, the same ±1 is symmetric. The floor, not the preset, is
what compresses the low side up top.

### The limits, and what compensates for each

Four limits, each argued above:

- **Treble $B$ is not measurable** (its partials are 30–60 dB below the
  fundamental) — compensated by continuous inverse-variance shrinkage toward a
  borrowed asymptote that has itself been checked.
- **The readout does not compound that uncertainty** — in that register the
  strobe and the coarse read target a partial that carries no $B$, so a wrong
  treble $B$ moves the target itself but is not multiplied again on the way to
  the reference the operator tunes against.
- **A bad capture cannot silently poison the curve** — a measurement implying a
  negative octave stretch is definitionally an estimator artifact, and the §2
  eviction hands that key back to the fit.
- **The octave type $\rho$ is not measurable even in principle** — it encodes
  which interval you prefer to make beatless, and it is worth more at the top of
  the piano than the whole $B$ uncertainty.

That last one is not compensated, and it feeds straight into the open question below.

### What is still open

Curve _selection_, and it is a sharper problem than "we need more data".

The engines split into two families in the bottom octave. At A0: the
octave-chain engines land at **(a) −50.4 ¢, with (b) and (c) within a cent of it**; the
multi-interval ones at **(d) −19.9 (shipped), (d) pure-12ths −14.3**. The
mechanism is plain — (a)–(c) walk the Eq.-6 chain, each step setting the
beatless width for the prescribed $\rho \approx 4.4$ down there, which with the
bass's large $B$ yields 11.2–13.6 ¢/oct of stretch; (d) least-squares a
compromise across octaves, twelfth, double octave and tempered fifths/fourths
at once, landing at 3.55 ¢/oct. **A ~31 ¢ disagreement on the lowest notes.**

It is not noise: report 0009 analysis 4 resampled which capture feeds each key over
24 draws and measured (d)-Balanced's bass curve SD at **0.02 ¢** — the gap is
three orders above it.

**Both available criteria are circular, in opposite directions.** Beat-rate
coherence favours (d) decisively (bass 2:1 median 0.16 Hz against (a)'s 0.68;
4:2 0.11 against 1.08) — but 2:1/4:2/6:3 are (d)'s own objective. Leave-one-key-out
favours the chain engines just as decisively ((d) 14.68 ¢ bass against (b)'s
0.42) — but its reference _is_ the Eq.-6 chain value. Each metric is one
family's objective. A cross-check on intervals in neither objective — major
thirds, sixths, minor thirds — cannot reach: across the temperament region all
five engines are indistinguishable (8.29–8.87 Hz median third rate, 1–2
reversals), and in the deep bass those intervals are not aural tests.

So no further computation on the existing data resolves it. Three things could:
an **aural reference tuning** measured and compared (the ground truth
the tuning-curve design note, §11 in git history at `b45adc0`, records
as absent), a **listening test** — which is what `synth.rs` and the `auralize`
harness exist for — or a beat-salience measure belonging to neither family's
objective, which has not been designed.

**What the GUI offers is a subset of what the worker computes.** The bundle
keeps all five engines — they cost little, and they are how the disagreement
above stays visible to the offline harnesses — but a selector asking the operator
to choose among five curves asks them to settle a question this project has not
settled. Engines (b) and (c) are therefore not offered as tuning targets while
their validity is open; (d)-Balanced is the shipped one.
