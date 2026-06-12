# Discovery Algorithm Class: Peak-Domain Model-Based Candidate Scoring

## Status

Accepted

## Context

The Engine's Discovery phase must identify which key was struck from a single ~46ms
analysis hop, in real time, on the zero-allocation hot path. Before committing further
engineering effort to the Two-Way Mismatch (TWM) implementation (parameter calibration
via MOBO per ADR 0001, and the split coarse-to-fine discovery search), we evaluated
whether a different algorithm family — or a different time-frequency transform — would
serve the in-scope instrument family (ADR 0004) better.

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
  exists for honest fine-tuning (the same ground-truth argument as ADR 0001).
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
and the frequency exponent $p$) are calibrated by MOBO (ADR 0001) rather than
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

What is project-specific (and therefore validated empirically via the MOBO evaluator's
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
   (ratio) error — and is decided empirically by MOBO Arm 4 (ADR 0001), not by fiat.
3. MAT remains the Worker-side measurement algorithm; no measurement-lineage
   algorithm is promoted into Discovery.

## Consequences

- Alternative-family proposals (periodicity trackers, harmonic-kernel methods,
  ratio-voting, transform swaps, learned trackers) are settled by this ADR and ADR
  0004's scope unless one of the revisit conditions below is met.
- The MOBO evaluator gains a discrete-vs-refined ablation arm; TWM constants are
  calibrated under whichever discovery mode wins.
- Learned trackers (e.g., streaming PESTO) may be added to the offline evaluator as
  comparison baselines; they are not eligible for the hot path.

### Revisit conditions

- A labeled, in-scope acoustic corpus with trustworthy ground truth becomes available
  (weakens the ADR 0001 argument against learned/likelihood-trained models).
- The product expands to polyphonic or simultaneous multi-note analysis (the
  Duan/Emiya probabilistic class is designed for that regime; TWM is not).
- Discovery-phase residuals after scale refinement show systematic $B$-mismatch
  structure (motivates promoting a second refinement dimension — joint $(f_0, B)$ —
  i.e., a bounded MAT-like step inside Discovery).
