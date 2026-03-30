//! # Algorithms — Stateless DSP Building Blocks
//!
//! Pure, stateless math organized by primary output. All functions take input
//! buffers and return computed values with no side effects.
//!
//! | Submodule | Domain | Returns |
//! |---|---|---|
//! | [`spectral`] | Time ↔ frequency transforms | Complex spectra, magnitude vectors |
//! | [`pitch`] | YIN / pYIN pitch detection | Frequency (Hz), confidence |
//! | [`dpyin`] | Decimated pYIN (bass register) | Frequency (Hz), confidence |
//! | [`scout`] | Rough frequency neighborhood | Frequency (Hz) |
//! | [`twm`] | Two-Way Mismatch pitch detection | Frequency (Hz), confidence |
//! | [`metrics`] | Signal property measurement | RMS, EMA, CSD, NINOS2 scalars |
//! | [`tuning`] | Tuning math | Cent deviations, compensated frequencies |
//! | [`inharmonicity`] | B-coefficient calculation | B coefficient (deprecated) |

pub mod dpyin;
pub mod inharmonicity;
pub mod spectral;
pub mod pitch;
pub mod twm;
pub mod metrics;
pub mod tuning;
