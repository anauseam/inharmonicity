# report 0005 — the discovery algorithm class, and the families that lost

Two parts, written fifteen months apart. The original analysis is a *literature*
evaluation rather than a measurement — it rules families out on model grounds
(what an inharmonic partial series does to a harmonic kernel), not on scores —
and it says so. The 2026-08-25 amendment is the closer thing to a measurement:
the Partial Frequencies Deviation lineage, previously placed by structure alone,
read against the shipped MAT path layer by layer.

## Context

The Engine's Discovery phase must identify which key was struck from a single ~46ms
analysis hop, in real time, on the zero-allocation hot path. Before committing further
engineering effort to the Two-Way Mismatch (TWM) implementation (parameter calibration
via NSGA-II per report 0001, and the split coarse-to-fine discovery search), we evaluated
whether a different algorithm family — or a different time-frequency transform — would
serve the in-scope instrument family (report 0004) better.

The hard requirements that any discovery algorithm must satisfy:

1. **Explicit inharmonicity:** $B$ must appear in the signal model, not as a widened
   tolerance around harmonic positions.
2. **Missing-fundamental robustness** down to A0, where Partial 1 carries no energy.
3. **Note-level identification** per hop (the sub-cent measurement is the Tracking
   phase's job, not Discovery's).
4. **Real-time, zero-allocation, analytically auditable** (the Topological Scrutiny
   Test in ARCHITECTURE.md).

## Analysis

### Families evaluated and rejected

- **Periodicity / lag-domain (autocorrelation, YIN, pYIN, MPM):** rejected at the
  model level. An inharmonic signal has no true period — stretched partials never
  realign — so the autocorrelation peak is smeared and pitch-biased, with the bias
  growing with $B$. A0 periodicity additionally requires lag windows longer than the
  entire 8192-pt bass FFT, and pYIN's HMM reintroduces the Viterbi path-persistence
  failure already excised from the Engine.
- **Harmonic-kernel spectral methods (HPS, cepstrum, SWIPE, log-frequency pattern
  correlation):** integer-ratio kernels decohere under the $\sqrt{1+Bn^2}$ stretch
  exactly where the energy is (high partials of bass strings). Inharmonizing the
  kernel per-candidate reconstructs peak-domain template scoring with extra
  computational cost (dense-spectrum correlation instead of sparse peak matching).
- **Ratio-voting (BaNa, Yang et al. 2014):** the heuristic cousin of candidate
  scoring. Assumes harmonic pairwise ratios (same stretch problem), targets coarse
  melody-tracking precision, and terminates in Viterbi smoothing. Its noise regime
  (0 dB SNR speech) is not ours; our failure modes (sub-harmonic density bias,
  unison beating) are not its.
- **Transform-stage alternatives (sliCQ/NSGT, zoom-FFT, reassignment, wavelets):**
  no transform repeals time-frequency uncertainty; sliCQ's low-frequency bands need
  windows comparable to our 186ms bass FFT to resolve bass semitones, so the tiling
  is reorganized but no information is gained. The pipeline already beats single-bin
  resolution where it matters via Jacobsen sub-bin estimation plus model pooling
  across partials (below). Reassignment/synchrosqueezing duplicate phase information
  the pipeline already exploits (Jacobsen in Discovery, Goertzel phase vocoder in
  Tracking).
- **Subspace high-resolution methods (ESPRIT/MUSIC, as used by Badeau/David/Richard
  for piano analysis):** genuinely higher-resolution peak estimation, but
  eigendecomposition cost confines them to offline use. If adopted at all, they
  compete with the planned CSPE upgrade in the Worker — not with Discovery.
- **Learned trackers (CREPE; PESTO, ISMIR 2023 / TISMIR 2025):** the streaming PESTO
  variant is genuinely real-time (<10ms, ~30k parameters), so latency is no longer
  the objection. The remaining objections are decisive: trained on harmonic corpora
  with general pitch-contour semantics (no concept of "which key, given this
  instrument's $B$"; no missing-fundamental bass semantics), not analytically
  auditable (fails the Topological Scrutiny Test), and no labeled in-scope corpus
  exists for honest fine-tuning (the same ground-truth argument as report 0001).
  Permitted role: offline comparison baseline in the evaluator only.

### Why peak-domain candidate scoring wins

The decisive argument is information-theoretic: pooling evidence across the 20–60
measured partials under a one-to-two-parameter model ($f_0$, optionally $B$) is the
only mechanism that beats the Gabor limit — effective resolution multiplies with the
partial count instead of being begged from the transform. Sparse peak-domain scoring
is the cheapest correct implementation of that pooling, and it is the only family that
directly reuses the pipeline's existing strengths (Jacobsen peaks, Neyman-Pearson
floor, critical-band masking).

In robust-statistics terms, TWM with the Duan ceiling is a bounded-influence
M-estimator over the pooled evidence. Its empirical constants ($q, r, \rho, \lambda$,
and the frequency exponent $p$) are calibrated by NSGA-II (report 0001) rather than
inherited from Maher & Beauchamp's wind-instrument dataset.

### The two-lineage structure: identification vs. measurement

The literature's inharmonic estimators split into two lineages that this architecture
deliberately keeps separate:

- **Measurement lineage** — inharmonic comb filters (Galembo & Askenfelt 1999) →
  Partial Frequencies Deviation (Rauhala et al. 2007) → Median-Adjustive Trajectories
  (Hodgkinson et al., DAFx-09). These jointly refine $(f_0, B)$ to high precision
  *given* an approximately known note. Their robustness comes from forward-direction
  evidence aggregation (comb energy; the median over pairwise $B$ estimates — itself
  a bounded-influence estimator, MAT's analogue of our Duan ceiling). They contain no
  machinery for rejecting a *wrong note hypothesis*.
- **Identification lineage** — TWM (Maher & Beauchamp 1994) and probabilistic peak
  models (Doval & Rodet; Duan et al. 2010; Emiya et al. 2010 for piano). The
  *two-way* error is the identification machinery: the forward error
  ($Err_{p \to m}$) punishes sub-harmonic candidates whose surplus predicted partials
  match nothing, and the reverse error ($Err_{m \to p}$) punishes harmonic/overtone
  candidates that leave measured peaks unexplained. The measurement lineage has no
  equivalent of this bidirectional test.

The pipeline already deploys each lineage where it belongs: TWM identifies in
Discovery; MAT measures in the Worker. This division is retained.

### Coarse-to-fine (split) discovery search

The two-stage search (discrete 88-key scan, then basin-clamped continuous scale
refinement of the top candidates) is a composition of standard, literature-grade
components rather than a published algorithm in itself:

- Continuous candidate search **is** canonical TWM — Maher & Beauchamp minimize over
  trial $f_0$ values (restricted via the measured peaks), not over a fixed note
  dictionary. The refinement stage restores this canonical property.
- Two-pass coarse-to-fine pitch search is standard practice: RAPT (Talkin 1995) runs
  a coarse decimated pass before a fine full-rate pass; SWIPE and YIN locate a
  coarse-grid optimum and polish it by local interpolation; Cano (1998) refines TWM
  candidate selection within SMS; PFD and MAT are themselves seed-then-refine loops.
- Grid bracketing followed by golden-section minimization is textbook numerical
  optimization, required here because error-vs-scale is piecewise (peak-to-partial
  nearest-neighbor associations switch discretely), so a pure unimodal line search
  is unsafe.

What is project-specific (and therefore validated empirically via the NSGA-II evaluator's
discrete-vs-refined ablation arm rather than by citation): using the 88-key ET grid as
the coarse stage — justified because the in-scope instruments are discretely pitched —
and the ±80-cent basin clamp, which guarantees refinement can only re-rank Stage A's
top candidates, never escape toward a sub-harmonic (1200 cents away).

## Decision

1. Discovery uses **peak-domain model-based candidate scoring**, with **TWM** as the
   scoring functional, over **inharmonicity-stretched per-key templates**, searched
   **coarse-to-fine** (88-key discrete scan → basin-clamped continuous scale
   refinement of the top-3 candidates).
2. The "score ratios instead of frequencies" question is resolved in two parts:
   the *search side* is adopted (scale refinement is exactly ratio/shape matching);
   the *error-metric side* is one parameter — $p = 1$ makes each TWM term a relative
   (ratio) error — and is decided empirically by NSGA-II Arm 4 (report 0001), not by fiat.
3. MAT remains the Worker-side measurement algorithm; no measurement-lineage
   algorithm is promoted into Discovery.

## Consequences

- Alternative-family proposals (periodicity trackers, harmonic-kernel methods,
  ratio-voting, transform swaps, learned trackers) are settled by this report and report
  0004's scope unless one of the revisit conditions below is met.
- The NSGA-II evaluator gains a discrete-vs-refined ablation arm; TWM constants are
  calibrated under whichever discovery mode wins.
- Learned trackers (e.g., streaming PESTO) may be added to the offline evaluator as
  comparison baselines; they are not eligible for the hot path.

### Revisit conditions

- A labeled, in-scope acoustic corpus with trustworthy ground truth becomes available
  (weakens the report 0001 argument against learned/likelihood-trained models).
- The product expands to polyphonic or simultaneous multi-note analysis (the
  Duan/Emiya probabilistic class is designed for that regime; TWM is not).
- Discovery-phase residuals after scale refinement show systematic $B$-mismatch
  structure (motivates promoting a second refinement dimension — joint $(f_0, B)$ —
  i.e., a bounded MAT-like step inside Discovery).

## Amendment 2026-08-25 — the measurement lineage, read

The lineage paragraph above placed Partial Frequencies Deviation (Rauhala,
Lehtonen & Välimäki 2007) by structure, without the paper. It and its 2025
descendant (Miljković et al., harp mPFD) have now been read and evaluated
against the shipped MAT path; the PDFs are in `resources/worker/`. The
lineage placement stands. What follows is the verdict the paragraph lacked.

**Rank by layer, not by name.** Every estimator in this lineage is three stacked
layers, and comparing whole algorithms hides which layer is binding:

| layer | MAT (shipped) | PFD | standing |
| --- | --- | --- | --- |
| frequency precision | CSPE super-resolution | 3-point parabolic interpolation on a zero-padded Blackman FFT | ours is finer |
| peak selection | predict from $(f_0,B)$, snap to the strongest peak within $\pm f_0/4$ | predict from $(f_1,\hat B)$, snap to the strongest peak within $\pm 0.4 f_1$ | the same mechanism; ours is the tighter band |
| combiner | median of the $K(K-1)/2$ pairwise $B$ (Eq. 8) | sign-majority of $\operatorname{sign}(D_{k+1}-D_k)$ driving a halving step in $\log \hat B$ | both rank-based, bounded-influence; parity |

PFD's claim is speed against Galembo's comb filter (58 s vs 946 s on 35 keys),
irrelevant to a Worker-side estimator; on synthetic tones its own Table 1 rates
the three methods equal (RMS 1.19e-6 vs 1.19e-6 vs 1.16e-6), and its real-tone
margin is against hand-read values with the footnote "the correct
inharmonicity value is unknown". It was validated on keys 1–35 only — the
register where MAT already has 24–32 partials and 0.4 % repeat noise (report 0009)
— and never where we lose, the treble. There, its band is wider than ours, and
band width is what admits the contaminant (primer §6). **Swapping MAT for PFD
is refuted on the paper's own evidence.** The earlier claim in this project's
notes that PFD "minimises an aggregate deviation" and is therefore not
bounded-influence was wrong; the combiner is a vote.

**The binding layer is peak selection, and no combiner repairs it.** A spectral
line at exactly $n f_1$ is not an outlier for a median to out-vote; it is a
valid measurement of $B = 0$, and enough of them carry the median with them
(primer §5: 12–23 % of stored partials above A6 sit there, on two independent
capture sets). MAT, PFD and mPFD all feed the combiner the same peaks. The only
thing that could beat the shipped path is a *different front end* — Galembo &
Askenfelt's inharmonic comb filter, which never indexes a line and so cannot be
told that a product at $4 f_1$ is partial 4, or Rigaud's NMF. Both are offline
and neither is cheap; the report 0006 note that only a different-front-end
estimator is an informative check is reaffirmed.

**mPFD is the right shape aimed at the wrong frequency.** Its contribution is a
front-end filter — predict where the contaminant is, delete candidates near it,
take the strongest survivor — with the exclusion window scaling as $k^3$, the
same exponent the path review derived for an adaptive band. But it predicts
Conklin phantoms at $f_i + f_j$, and on this instrument the contaminant is
measured to be harmonic distortion at $n f_1$ (37 : 3 by a two-sided test,
primer §5), 60–160 Hz from where mPFD would look. The candidate for a future mPFD probe
is therefore mPFD's structure with $n f_1$ as the predicted location, which
costs no new constant.

Decision 3 is unchanged. The revisit condition "a different front end" is added
below; it is not met.

### Revisit conditions (added)

- An offline comb-filter or NMF run on both instruments shows treble $B$ that
  MAT's repeat-capture upper tail does not reach (would motivate a second
  front end in the Worker, not a change of combiner).

## Appendix — why the ET grid fails and scale refinement fixes it

The quantitative derivations behind the split-search decision here and the
calibration in [report 0001](0001-nsga2-tuning.md). All numbers assume 44.1 kHz
and the current window sizes.

### 1. The discrete-grid problem

Canonical TWM (Maher & Beauchamp 1994) is a **one-dimensional continuous
minimization**: $Err_{total}(f_0)$ is defined over a continuous trial fundamental,
candidates are generated densely (from measured peaks and their submultiples), and the
output is the argmin — a *frequency*.

The Engine's discovery loop evaluates that error function at exactly 88 points: each
`KeyProfile` pins `f0_et` to the equal-tempered frequency, and the error is never
evaluated between them. The continuous minimization became a table lookup over a
lattice with 100-cent spacing, and the output collapsed from a frequency to a key
index. The scoring math is canonical; the *search domain* is not.

### 2. How the grid breaks TWM's guarantees

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
can remove** — which is why the refinement decision precedes the NSGA-II run.

**The seeding casualty.** Because the grid discards the continuous output, tracking is
seeded from ET predictions. The Goertzel phase vocoder's unwrap range at the
1024-sample hop is $\pm 1/(2\,t_{hop}) \approx \pm 21.5$ Hz; an ET seed for partial
$n$ of a mistuned note is off by $\delta \cdot n f_0$ (≈130 Hz for partial 10 of an A4
struck 50 cents flat). Those partials fail the SNR gate, never enter the adaptive EMA,
and tracking coverage silently collapses on exactly the instruments the tool targets.

### 3. Why scale refinement is "ratio matching," precisely

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

### 4. The error metric is a separate, empirical question

The TWM per-term error $\Delta f \cdot f^{-p}$ is only partially scale-free. At
$p = 1$ each term becomes $\Delta f / f$ — a pure relative (ratio) error. At the
canonical $p = 0.5$, a uniform mistuning contributes $\delta\sqrt{n f_0}$ per term:
covariant with absolute pitch, growing with $\sqrt{n}$. Moving to $p = 1$ is therefore
the "score in ratio space" proposal in metric form — but it also reweights bass-vs-
treble errors (a different psychoacoustic weighting than M&B's empirical choice), so
it is not free invariance. Per Decision 2 above, $p$ is decided by NSGA-II Arm 4, not
by fiat. Search-side invariance (Section 3) is adopted analytically; metric-side
invariance is one calibrated parameter.

### 5. Context: why TWM is rare in the literature despite fitting this problem

Three non-technical reasons, recorded so absence-of-popularity is not mistaken for
inferiority: (1) the field's benchmark problems are voice and melody — harmonic
signals where periodicity trackers (YIN-class) suffice and deep models now top the
leaderboards; TWM's niche (explicit inharmonic templates, monophonic, real-time
note-ID) is essentially instrument tuners and spectral-modeling analysis. (2) The
commercial products in exactly that niche (CyberTuner, Verituner, TuneLab) do not
publish their algorithms, so the niche's best practice is invisible to the literature.
(3) TWM ships with uncalibrated constants ($q, r, \rho$) and no published principled
calibration — practitioners historically preferred parameter-light YIN. The NSGA-II
program (report 0001) closes precisely this third gap.
