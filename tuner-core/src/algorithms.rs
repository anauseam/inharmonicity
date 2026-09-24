//! # Algorithms — stateless DSP building blocks
//!
//! Pure functions from input buffers to computed values. The tuning-curve
//! modules, [`curves`] and the [`rigaud`], [`giordano`] and [`whittaker`] layers
//! it composes, are cold-path: they allocate, and run on a profile change, never
//! in the hot loop.

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
