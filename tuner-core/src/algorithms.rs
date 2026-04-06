//! # Algorithms — Stateless DSP Building Blocks
//!
//! Pure, stateless math organized by primary output. All functions take input
//! buffers and return computed values with no side effects.
//!
//! | Submodule | Domain | Returns |
//! |---|---|---|
//! | [`spectral`] | Time ↔ frequency transforms | Complex spectra, magnitude vectors |
//! | [`pitch`] | Pitch detection algorithms | Frequency (Hz), confidence |
//! | [`templates`] | Sparse matched-filter templates | SparseTemplate configs |
//! | [`mat`] | MAT pitch + inharmonicity | Frequency (Hz), partials |
//! | [`twm`] | Two-Way Mismatch (inactive) | — |
//! | [`metrics`] | Signal property measurement | RMS, EMA, CSD, NINOS2 scalars |
//! | [`tuning`] | Tuning math | Cent deviations, compensated frequencies |
//! | [`inharmonicity`] | B-coefficient calculation | B coefficient (deprecated) |

pub mod inharmonicity;
pub mod mat;
pub mod metrics;
pub mod phantom;
pub mod pitch;
pub mod spectral;
pub mod templates;
pub mod tuning;
pub mod twm;
