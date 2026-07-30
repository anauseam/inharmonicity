//! # Algorithms — Stateless DSP Building Blocks
//!
//! Pure, stateless math organized by primary output. All functions take input
//! buffers and return computed values with no side effects.
//!
//! | Submodule | Domain | Returns |
//! |---|---|---|
//! | [`spectral`] | Time ↔ frequency transforms | Complex spectra, magnitudes, CSPE freqs |
//! | [`peaks`] | Peak extraction + masking; bounded CFAR-gated coarse read | [`models::SpectralPeak`] lists, coarse frequency |
//! | [`mat`] | MAT adjustive (f₀, B) estimator | Frequency (Hz), B, partials |
//! | [`twm`] | Two-Way Mismatch scoring | TWM error (Hz) |
//! | [`discovery`] | Split discovery search (ADR 0005) | Key index, scale, error |
//! | [`metrics`] | Signal property measurement | RMS, EMA, CSD, NINOS2 scalars |
//! | [`curves`] | The four tuning-curve engines (a)–(d) | [`models::TuningCurve`] |
//! | [`rigaud`] | Rigaud parametric inharmonicity **and tuning** model | B_ξ fit, ρ_φ, erf, F₀ |
//! | [`giordano`] | Giordano sensory-dissonance recipe | Dissonance, octave-scan optima |
//! | [`whittaker`] | Whittaker smoother + shared banded LS solver | Smoothed vectors, λ, solutions |
//!
//! [`models`]: crate::models
//!
//! The tuning-curve quartet ([`curves`], [`rigaud`], [`giordano`],
//! [`whittaker`]) is cold-path: those functions allocate and run on profile
//! update/load, never inside the DSP hot loop
//! (`docs/design/tuning-curve-design.md`). [`curves`] is the orchestrator — it
//! composes the other three, exactly as [`discovery`] composes [`twm`].

pub mod curves;
pub mod discovery;
pub mod giordano;
pub mod mat;
pub mod metrics;
pub mod peaks;
pub mod rigaud;
pub mod spectral;
pub mod twm;
pub mod whittaker;
