# Discovery Search Analysis: Why the ET Grid Fails and Scale Refinement Fixes It

This is a design-analysis note, not an ADR. The decisions it supports are recorded in
[ADR 0005](../adr/0005-discovery-algorithm-class.md) (algorithm class and split
discovery) and [ADR 0001](../adr/0001-mobo-tuning.md) (parameter calibration). This
document preserves the quantitative derivations behind those decisions so the future
engine documentation can explain *why* rather than just *what*. All numbers assume
44.1 kHz and the current window sizes.

## 1. The discrete-grid problem

Canonical TWM (Maher & Beauchamp 1994) is a **one-dimensional continuous
minimization**: $Err_{total}(f_0)$ is defined over a continuous trial fundamental,
candidates are generated densely (from measured peaks and their submultiples), and the
output is the argmin — a *frequency*.

The Engine's discovery loop evaluates that error function at exactly 88 points: each
`KeyProfile` pins `f0_et` to the equal-tempered frequency, and the error is never
evaluated between them. The continuous minimization became a table lookup over a
lattice with 100-cent spacing, and the output collapsed from a frequency to a key
index. The scoring math is canonical; the *search domain* is not.

## 2. How the grid breaks TWM's guarantees

TWM's published robustness (missing fundamentals, spurious peaks, octave
discrimination) consists of statements about the error surface **at and around its
true minimum**. On a grid, a mistuned note is never evaluated at that minimum.

For a note mistuned by fraction $\delta$ from its nearest ET key ($\delta \approx
0.0293$ at 50 cents):

**Residual inflation.** Partial $n$ sits $\delta \cdot n f_0$ Hz from the grid
candidate's prediction. Residuals that should be near zero at the true minimum grow
linearly with partial index. With the $f^{-1/2}$ weighting ($p = 0.5$), the per-term
weighted error grows as $\delta\sqrt{n f_0}$.

**Reward defection.** The $-r$ reward fires when a strong peak aligns with a
prediction. On the basin shoulder, the true key's strong peaks no longer align, so the
bonus that should separate the correct candidate from the field vanishes.

**The mis-association bound.** Predicted partials are spaced ≈ $f_0$ apart, so
nearest-neighbor matching pairs a peak with the *wrong* predicted partial once
$\delta \cdot n f_0 > f_0/2$, i.e. for all

$$n > \frac{1}{2\delta}$$

| Mistuning | $\delta$ | Mis-association above |
| --------- | -------- | --------------------- |
| 25 cents  | 0.0146   | $n \approx 34$        |
| 50 cents  | 0.0293   | $n \approx 17$        |
| 100 cents | 0.0595   | $n \approx 8$         |

A mid-bass note with 30–60 active partials therefore has a large fraction of its
spectrum matched to wrong partials at 50 cents of mistuning — exactly the regime of an
out-of-tune piano or a pitch raise.

**Asymmetry → bass-lock.** The damage is selective. Worked example: D2
($f_0 = 73.4$ Hz) struck 40 cents flat ($\delta \approx 0.023$). Partial 20 lands
~34 Hz from D2's prediction — nearly half the 73-Hz partial spacing — and everything
above $n \approx 21$ mis-associates. Meanwhile A#0's predicted partials are spaced
~29 Hz apart, so *no peak anywhere in the spectrum is ever more than ~15 Hz from one
of its predictions*, in tune or not. The dense sub-harmonic candidate's error is
nearly mistuning-invariant while the true key's error climbs with $\delta$; the margin
shrinks monotonically until the ranking flips. Part of the observed bass-lock bias is
therefore a **grid artifact that no amount of parameter tuning on the discrete engine
can remove** — which is why the refinement decision precedes the MOBO run.

**The seeding casualty.** Because the grid discards the continuous output, tracking is
seeded from ET predictions. The Goertzel phase vocoder's unwrap range at the
1024-sample hop is $\pm 1/(2\,t_{hop}) \approx \pm 21.5$ Hz; an ET seed for partial
$n$ of a mistuned note is off by $\delta \cdot n f_0$ (≈130 Hz for partial 10 of an A4
struck 50 cents flat). Those partials fail the SNR gate, never enter the adaptive EMA,
and tracking coverage silently collapses on exactly the instruments the tool targets.

## 3. Why scale refinement is "ratio matching," precisely

A candidate key's stretched series $\{f_n\} = \{n f_{0,ET}\sqrt{1+Bn^2}\}$ is a
**shape**: multiplying every element by a scale factor $s$ leaves every internal ratio
$f_m/f_n$ unchanged and slides the shape along the frequency axis. In log-frequency
this is literal translation: $\log(s f_n) = \log s + \log f_n$. The shape is the
candidate's identity; $s$ is the continuous pitch variable.

Searching over $s$ therefore asks the scale-free question — does the spectrum's ratio
structure match this key's stretched pattern, wherever it sits? — instead of the
absolute-position question the grid asks. Minimizing over $s \in \pm 80$ cents across
88 keys is minimizing over a continuous $f_0$ axis tiled into 88 basins: canonical
TWM's one-dimensional search, reorganized by key. Nothing novel is added; the degree
of freedom Maher & Beauchamp always had is restored.

At the refined minimum every broken property recovers: residuals return to noise
level, the $r$-rewards re-engage for the correct candidate, mis-association disappears
(offsets are small for all $n$ at the basin floor), the margin over dense impostors
reopens (their scores were never mistuning-dependent; the true key's score drops back
to its floor), and the argmin again yields a frequency — which seeds the Goertzel
trackers inside their unwrap range.

Two safety properties of the basin-clamped form:

- Adjacent-key basins at ±80 cents barely overlap (semitone = 100 cents), and
  sub-harmonics are 1200 cents away — refinement can only re-rank Stage A's top
  candidates, never escape toward a new false lock.
- Error-vs-scale is **piecewise, not unimodal** (peak associations switch discretely
  as $s$ sweeps), which is why Stage B brackets with a coarse pre-grid before
  golden-section polishing.

## 4. The error metric is a separate, empirical question

The TWM per-term error $\Delta f \cdot f^{-p}$ is only partially scale-free. At
$p = 1$ each term becomes $\Delta f / f$ — a pure relative (ratio) error. At the
canonical $p = 0.5$, a uniform mistuning contributes $\delta\sqrt{n f_0}$ per term:
covariant with absolute pitch, growing with $\sqrt{n}$. Moving to $p = 1$ is therefore
the "score in ratio space" proposal in metric form — but it also reweights bass-vs-
treble errors (a different psychoacoustic weighting than M&B's empirical choice), so
it is not free invariance. Per ADR 0005 Decision 2, $p$ is decided by MOBO Arm 4, not
by fiat. Search-side invariance (Section 3) is adopted analytically; metric-side
invariance is one calibrated parameter.

## 5. Context: why TWM is rare in the literature despite fitting this problem

Three non-technical reasons, recorded so absence-of-popularity is not mistaken for
inferiority: (1) the field's benchmark problems are voice and melody — harmonic
signals where periodicity trackers (YIN-class) suffice and deep models now top the
leaderboards; TWM's niche (explicit inharmonic templates, monophonic, real-time
note-ID) is essentially instrument tuners and spectral-modeling analysis. (2) The
commercial products in exactly that niche (CyberTuner, Verituner, TuneLab) do not
publish their algorithms, so the niche's best practice is invisible to the literature.
(3) TWM ships with uncalibrated constants ($q, r, \rho$) and no published principled
calibration — practitioners historically preferred parameter-light YIN. The MOBO
program (ADR 0001) closes precisely this third gap.
