# report 0001 — the NSGA-II methodology, in full

The parameter definitions, the argument for a synthetic dataset, the objectives
and the search procedure.

This is a *methodology*, not a result. The results are
[report 0006](0006-discovery-refinement-validation.md), and the operational layer
— harness, signal model, determinism, arm structure, selection protocol and a
threats-to-validity audit — is [the NSGA-II methodology](nsga2/README.md).

One thing worth reading here rather than in the head: the objective-B history.
An earlier *margin* formulation was **gamed** by the search, which drove the
per-frame median toward the normaliser floor; the ordinal form replaced it
because a rank statistic cannot be gamed that way. That is a concrete instance
of an optimiser exploiting a metric, and it is why the front is selected from on
real captures rather than by synthetic hypervolume.

## The Parameters

The canonical TWM formulation by Maher & Beauchamp (1994) relies on three empirical constants:

- **$q$ (Amplitude penalty scaling):** Determines how harshly to penalize a predicted harmonic that matches a weak peak instead of a strong structural peak.
- **$r$ (Reward constant):** The "bonus" applied when a predicted partial perfectly aligns with a high-energy peak.
- **$\rho$ (Reverse error weight):** The weighting factor balancing $Err_{m \to p}$ (the penalty for unexplained peaks in the measured spectrum) against $Err_{p \to m}$ (the penalty for missing expected partials).

Maher & Beauchamp empirically selected the default values ($q=1.4, r=0.5, \rho=0.33$) based on a small dataset of brass and woodwind instruments. Pianos, however, exhibit extreme inharmonicity, missing fundamentals in the bass, and significant sympathetic resonance. Using the defaults leaves the engine vulnerable to false locks (e.g., locking to a sub-harmonic).

## Multi-Objective Optimization (NSGA-II)

To adapt TWM specifically for piano acoustics, we use a multi-objective optimization framework. The optimiser is **NSGA-II** (a non-dominated-sorting genetic algorithm, via Optuna), *not* a Bayesian/Gaussian-process method. "NSGA-II" persists as the project shorthand (filenames, `twm_nsga2.db`, etc.) for historical reasons — read it as "multi-objective optimization," with the understanding that the search is evolutionary, not Bayesian.

### Why a Synthetic Dataset?

The optimization requires a dataset of approximately 10,000 frames. While real acoustic piano recordings seem ideal, they lack one critical component: **Absolute Ground Truth**.

To algorithmically evaluate if $q=1.6$ is better than $q=1.4$, the NSGA-II framework must know the exact, mathematically perfect fundamental frequency ($f_0$) and inharmonicity coefficient ($B$) for every frame. Manually annotating 10,000 real acoustic frames to sub-cent precision is impossible, and human error in the dataset would cause the NSGA-II to optimize for flawed data (garbage in, garbage out).

By using a generative synthetic dataset, we achieve:

1. **Perfect Ground Truth:** We mathematically define the exact $f_0$, $B$, and noise floor of every frame before generating the audio.
2. **Controlled Edge Cases:** We can programmatically generate edge cases that cause TWM to fail in the wild, such as extreme missing fundamentals (A0), heavy sympathetic tonal noise, and specific beating unisons.

_(Note: The standard ML practice is to optimize against the massive synthetic dataset, and then validate the resulting Pareto optimal parameters against a smaller, manually verified dataset of real acoustic recordings.)

### The Objectives

NSGA-II allows us to optimize for multiple, often competing, goals simultaneously. Our primary objectives are:

- **Objective A (Accuracy):** Minimize false-locks across the dataset — specifically the *separability* false-lock (does the true key win the all-88 argmin), which isolates the constants' ranking job from the K=3 recall path.
- **Objective B (Confidence):** Maximize the **ordinal** confidence — the fraction of the 87 impostors the true key out-scores. This replaced an earlier *margin* formulation (distance to the second-best candidate), which the search **gamed** by driving the per-frame median toward the normaliser floor; the rank-based form is immune to that.

### The Search Process

A brute-force grid search across the continuous parameter space would take an immense amount of compute. NSGA-II is an evolutionary multi-objective algorithm:

1. It evaluates an initial population of parameter combinations.
2. It ranks them by Pareto non-domination (plus a crowding-distance tie-break for diversity along the front).
3. It evolves the next generation by selection/crossover/mutation biased toward the non-dominated set — converging the whole population toward the Pareto frontier rather than a single optimum.

The output is a **Pareto Frontier**—a set of optimal tradeoffs. Final constants are then chosen by **real-capture validation** over the front (not by synthetic hypervolume), and hardcoded into `tuner-core/src/algorithms/twm.rs`.

> **See also.** The operational layer — harness, synthetic signal model, determinism/fingerprint, arm structure, selection protocol, and a threats-to-validity audit — lives in [the NSGA-II methodology](nsga2/README.md); outcomes and the (provisional) final config are in report 0006.
