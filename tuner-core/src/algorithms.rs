//! # Algorithms — Stateless DSP Building Blocks
//!
//! This module re-exports the four core algorithm submodules used throughout
//! the tuner pipeline. All functions in these modules are **stateless** —
//! they take input buffers and return computed values with no side effects.
//!
//! | Submodule | Purpose |
//! |---|---|
//! | [`fft`] | Forward FFT, Hann windowing, spectrum magnitude extraction |
//! | [`pitch`] | YIN and pYIN pitch detection with parabolic interpolation |
//! | [`power`] | RMS, EMA, CSD, and NINOS2 — power and spectral metrics |
//! | [`tuning`] | 88-key note mapping, cent deviation, frequency lookup |

pub mod fft;
pub mod pitch;
pub mod power;
pub mod tuning;
