//! # Musical Tuning Math Module
//!
//! This module provides the core musical tuning calculations for piano tuning applications.
//! It handles cent deviation measurements based on equal temperament tuning, with
//! planned support for inharmonicity-compensated target frequency calculations.
//!
//! ## Features
//! - Cent deviation calculations for tuning accuracy
//! - **Future**: Inharmonicity compensation for professional piano tuning
//!
//! ## Planned Inharmonicity Features
//! - Piano-specific inharmonicity curve calculation
//! - Stretch tuning compensation for different piano sizes
//! - Partial frequency analysis and adjustment
//! - Professional tuning curve generation

use crate::models::find_nearest_note_by_index;

/// Calculates the deviation from a target frequency in cents.
///
/// Cents are a logarithmic unit of pitch measurement where:
/// - 100 cents = 1 semitone
/// - 1200 cents = 1 octave
/// - Positive values indicate sharpness, negative values indicate flatness
///
/// # Arguments
/// * `freq` - Measured frequency in Hz
/// * `target_freq` - Target frequency in Hz
///
/// # Returns
/// * Cent deviation (positive = sharp, negative = flat)
pub fn calculate_cents_deviation(freq: f32, target_freq: f32) -> f32 {
    1200.0 * (freq / target_freq).log2()
}

/// Calculates inharmonicity-compensated target frequency for professional piano tuning.
///
/// **Note**: This function is planned for future implementation and currently returns
/// the equal temperament frequency. Inharmonicity compensation will account for:
/// - Piano string stiffness and inharmonicity
/// - Stretch tuning for different piano sizes
/// - Partial frequency adjustments
/// - Professional tuning curve generation
///
/// # Arguments
/// * `key_index` - Piano key index (0-87)
/// * `piano_type` - Type of piano (grand, upright, etc.) - future parameter
///
/// # Returns
/// * Target frequency with inharmonicity compensation (currently equal temperament)
///
/// # Future Implementation
/// This function will implement the inharmonicity calculations described in:
/// - Young's inharmonicity model
/// - Piano-specific stretch tuning curves
/// - Partial frequency analysis and compensation
pub fn calculate_inharmonicity_compensated_frequency(
    key_index: u8,
    _piano_type: &str, // Reserved for future piano type parameter
) -> f32 {
    // TODO: Implement inharmonicity compensation
    // For now, return equal temperament frequency
    let (_, freq) = find_nearest_note_by_index(key_index);
    freq
}
