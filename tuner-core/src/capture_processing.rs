//! # Capture Processing Module
//!
//! Handles the processing of captured audio analysis frames for inharmonicity measurement.
//! This module provides different processing strategies for analyzing stable audio frames.

use crate::{
    inharmonicity::{KeyMeasurement, Partial},
    tuning,
};

/// Different processing operations that can be performed on captured frames
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessingOperation {
    /// Find the frame with the highest confidence (default strategy)
    BestConfidence,
    /// Average all frames (future implementation)
    Average
}

/// Processes captured frames using the specified operation strategy.
///
/// This function:
/// 1. Applies the specified processing operation to the buffer
/// 2. Creates a KeyMeasurement from the processed result
/// 3. Calculates the 'B' value for the measurement
/// 4. Returns the measurement for the caller to store
///
/// # Arguments
/// * `buffer` - The buffer of stable analysis results to process
/// * `operation` - The processing operation to perform
///
/// # Returns
/// * `Option<KeyMeasurement>` - The processed measurement if successful, None otherwise
pub fn process(buffer: Vec<crate::AnalysisResult>, operation: ProcessingOperation) -> Option<KeyMeasurement> {
    match operation {
        ProcessingOperation::BestConfidence => process_best_confidence(buffer),
        ProcessingOperation::Average => {
            eprintln!("[CAPTURE] Average processing not yet implemented");
            None
        }
    }
}

/// Processes frames using the "Best-Confidence" strategy.
///
/// This is the default and currently only implemented strategy:
/// 1. Finds the single `AnalysisResult` with the highest confidence in the buffer
/// 2. Uses that `best_frame` to create a `KeyMeasurement`
/// 3. Calculates the 'B' value for the measurement
fn process_best_confidence(buffer: Vec<crate::AnalysisResult>) -> Option<KeyMeasurement> {
    // 1. Find the frame with the highest confidence
    let best_frame = buffer
        .iter()
        .max_by(|a, b| {
            let conf_a = a.confidence.unwrap_or(0.0);
            let conf_b = b.confidence.unwrap_or(0.0);
            conf_a
                .partial_cmp(&conf_b)
                .unwrap_or(std::cmp::Ordering::Less)
        });

    if let Some(best_frame) = best_frame {
        // 2. Use this frame to perform the capture logic
        if let (Some(note_name), Some(freq)) =
            (&best_frame.note_name, best_frame.detected_frequency)
        {
            let key_index = tuning::get_key_index_from_name(note_name);

            // Create the fundamental partial (n=1)
            let mut all_partials = vec![Partial {
                number: 1,
                frequency: freq,
            }];

            // Create the overtone partials (n=2, 3, 4...)
            let overtone_partials = best_frame
                .partials
                .iter()
                .enumerate()
                .map(|(i, &freq)| Partial {
                    number: (i + 2) as u32, // find_partials starts at the 2nd partial
                    frequency: freq,
                });
            all_partials.extend(overtone_partials);

            // 3. Create and 4. Calculate 'B' value
            let mut measurement = KeyMeasurement {
                key_index,
                partials: all_partials,
                calculated_b: None,
            };
            measurement.calculate_b_value();

            eprintln!(
                "[CAPTURE] Processed measurement for {}: B={:?}",
                note_name, measurement.calculated_b
            );

            Some(measurement)
        } else {
            eprintln!("[CAPTURE] Process failed: Best frame had no stable note data.");
            None
        }
    } else {
        eprintln!("[CAPTURE] Process failed: No best frame found in buffer.");
        None
    }
}
