//! # Algorithms — Stateless DSP Building Blocks
//!
//! Pure, stateless math organized by primary output. All functions take input
//! buffers and return computed values with no side effects.
//!
//! | Submodule | Domain | Returns |
//! |---|---|---|
//! | [`spectral`] | Time ↔ frequency transforms | Complex spectra, magnitude vectors |
//! | [`pitch`] | Pitch detection algorithms | Frequency (Hz), confidence |
//! | [`mat`] | MAT pitch + inharmonicity | Frequency (Hz), partials |
//! | [`twm`] | Two-Way Mismatch scoring | TWM error (Hz) |
//! | [`discovery`] | Split discovery search (ADR 0005) | Key index, scale, error |
//! | [`metrics`] | Signal property measurement | RMS, EMA, CSD, NINOS2 scalars |
//! | [`tuning`] | Tuning math | Cent deviations, compensated frequencies |
//! | [`inharmonicity`] | B-coefficient calculation | B coefficient (deprecated) |

pub mod discovery;
pub mod inharmonicity;
pub mod mat;
pub mod metrics;
pub mod peaks;
pub mod pitch;
pub mod spectral;
pub mod tuning;
pub mod twm;
