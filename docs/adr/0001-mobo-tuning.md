# TWM Parameter Optimization (MOBO)

This document explains the rationale and methodology for tuning the empirical constants within the Two-Way Mismatch (TWM) algorithm.

## The Parameters

The canonical TWM formulation by Maher & Beauchamp (1994) relies on three empirical constants:

- **$q$ (Amplitude penalty scaling):** Determines how harshly to penalize a predicted harmonic that matches a weak peak instead of a strong structural peak.
- **$r$ (Reward constant):** The "bonus" applied when a predicted partial perfectly aligns with a high-energy peak.
- **$\rho$ (Reverse error weight):** The weighting factor balancing $Err_{m \to p}$ (the penalty for unexplained peaks in the measured spectrum) against $Err_{p \to m}$ (the penalty for missing expected partials).

Maher & Beauchamp empirically selected the default values ($q=1.4, r=0.5, \rho=0.33$) based on a small dataset of brass and woodwind instruments. Pianos, however, exhibit extreme inharmonicity, missing fundamentals in the bass, and significant sympathetic resonance. Using the defaults leaves the engine vulnerable to false locks (e.g., locking to a sub-harmonic).

## Multi-Objective Bayesian Optimization (MOBO)

To adapt TWM specifically for piano acoustics, we use a Multi-Objective Bayesian Optimization (MOBO) framework.

### Why a Synthetic Dataset?

The optimization requires a dataset of approximately 10,000 frames. While real acoustic piano recordings seem ideal, they lack one critical component: **Absolute Ground Truth**.

To algorithmically evaluate if $q=1.6$ is better than $q=1.4$, the MOBO framework must know the exact, mathematically perfect fundamental frequency ($f_0$) and inharmonicity coefficient ($B$) for every frame. Manually annotating 10,000 real acoustic frames to sub-cent precision is impossible, and human error in the dataset would cause the MOBO to optimize for flawed data (garbage in, garbage out).

By using a generative synthetic dataset, we achieve:

1. **Perfect Ground Truth:** We mathematically define the exact $f_0$, $B$, and noise floor of every frame before generating the audio.
2. **Controlled Edge Cases:** We can programmatically generate edge cases that cause TWM to fail in the wild, such as extreme missing fundamentals (A0), heavy sympathetic tonal noise, and specific beating unisons.

_(Note: The standard ML practice is to optimize against the massive synthetic dataset, and then validate the resulting Pareto optimal parameters against a smaller, manually verified dataset of real acoustic recordings.)_

### The Objectives

MOBO allows us to optimize for multiple, often competing, goals simultaneously. Our primary objectives are:

- **Objective A (Accuracy):** Minimize the absolute number of false-locks across the dataset.
- **Objective B (Confidence):** Maximize the error-score distance (the margin) between the winning correct key and the second-best candidate.

### The Search Process

A brute-force grid search across a continuous 3D space ($q, r, \rho$) would take an immense amount of compute. MOBO solves this using a Bayesian surrogate model (like a Gaussian Process):

1. It tests a small set of random parameter combinations.
2. It builds a probabilistic "map" predicting how changes to $q, r, \rho$ impact Accuracy and Confidence.
3. It intelligently selects the next set of parameters to test by balancing **exploration** (searching unknown areas of the map) and **exploitation** (refining known good areas).

The output of the MOBO process is a **Pareto Frontier**—a set of optimal tradeoffs. We will select the parameter combination that provides the maximum stability for bass strings without degrading treble confidence, and hardcode those values into `tuner-core/src/algorithms/twm.rs`.
