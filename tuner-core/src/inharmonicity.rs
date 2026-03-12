//! # Inharmonicity Measurement & Profiles
//!
//! This module handles the calculation and storage of the inharmonicity
//! constant ($B$) for individual piano keys. Inharmonicity causes the partials
//! of a piano string to be slightly sharper than perfect integer multiples of
//! the fundamental, described by:
//!
//! $$f_n = n \cdot f_1 \cdot \sqrt{1 + B \cdot n^2}$$
//!
//! where $n$ is the partial number and $B$ is the inharmonicity coefficient.
//!
//! ## Data Model
//!
//! - [`Partial`] — A single measured partial (number + frequency).
//! - [`KeyMeasurement`] — All partials for one key, plus the computed $B$ value.
//! - [`InharmonicityProfile`] — The complete set of measurements for a piano,
//!   serializable to/from JSON for profile persistence.
//!
//! ## B Calculation
//!
//! [`KeyMeasurement::calculate_b_value`] uses linear regression on the
//! transformed data points $(n^2, (f_n / n)^2)$. The slope/intercept ratio
//! gives $B$.

use serde::{Serialize, Deserialize};
use std::collections::BTreeMap;
use linreg::linear_regression;

/// A single measured partial (overtone) of a piano note.
///
/// The fundamental is `number = 1`. Overtones are `number = 2, 3, …`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Partial {
    /// The partial number ($n$). 1 = fundamental, 2 = first overtone, etc.
    pub number: u32,
    /// The measured frequency of this partial in Hz.
    pub frequency: f32,
}

/// Stores all measured partials for a single piano key, plus the computed
/// inharmonicity constant ($B$).
///
/// Created by the capture processing pipeline after the Gatekeeper triggers
/// a successful capture and the Worker runs partial extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyMeasurement {
    /// The 88-key piano index (0 = A0, 87 = C8).
    pub key_index: u8,
    /// All measured partials for this key (fundamental + overtones).
    pub partials: Vec<Partial>,
    /// The computed inharmonicity coefficient, or `None` if not yet calculated
    /// or if there were insufficient partials.
    pub calculated_b: Option<f32>,
}

/// The complete inharmonicity profile for a specific piano.
///
/// This is the top-level serializable object saved to and loaded from a JSON file.
/// It maps each measured key index to its [`KeyMeasurement`] data. A `BTreeMap`
/// keeps keys sorted automatically for clean serialization.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InharmonicityProfile {
    /// Maps a piano key index (0–87) to its measurement data.
    pub measurements: BTreeMap<u8, KeyMeasurement>,
}

impl KeyMeasurement {
    /// Calculates the inharmonicity constant ($B$) from this key's partials
    /// using linear regression.
    ///
    /// The method transforms each partial $(n, f_n)$ into a data point:
    /// - $x = n^2$
    /// - $y = (f_n / n)^2$
    ///
    /// A linear fit $y = \text{slope} \cdot x + \text{intercept}$ gives
    /// $B = \text{slope} / \text{intercept}$.
    ///
    /// # Returns
    /// * `Some(B)` — The inharmonicity coefficient if at least 3 valid partials exist.
    /// * `None` — If there are too few partials or the regression fails.
    ///
    /// # Side Effects
    /// Stores the result in `self.calculated_b` for later retrieval.
    pub fn calculate_b_value(&mut self) -> Option<f32> {
        if self.partials.len() < 3 {
            return None; // Need at least 3 points for a meaningful regression
        }

        // Prepare the (x, y) data points for linear regression
        // x = n^2, y = (f_n / n)^2
        let (xs, ys): (Vec<f64>, Vec<f64>) = self.partials.iter()
            .filter(|p| p.number > 0 && p.frequency > 0.0)
            .map(|p| {
                let n = p.number as f64;
                let f_n = p.frequency as f64;
                let x = n * n;
                let y = (f_n / n) * (f_n / n);
                (x, y)
            })
            .unzip();

        if let Ok((slope, intercept)) = linear_regression::<_, _, f64>(&xs, &ys) {
            if intercept.abs() > 1e-6 {
                let b_value = slope / intercept;
                self.calculated_b = Some(b_value as f32);
                return self.calculated_b;
            }
        }

        None
    }
}