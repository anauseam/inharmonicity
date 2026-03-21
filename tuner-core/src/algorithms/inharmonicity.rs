//! # Inharmonicity Algorithms
//!
//! Mathematical calculation of the inharmonicity constant ($B$) from partal frequencies.

use crate::models::KeyMeasurement;
use linreg::linear_regression;

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
/// # Deprecation Notice
/// **DEPRECATED**: This function is deprecated. After the pipeline migration happens, 
/// other algorithms will perform this role in a manner better in line with the new program structure.
///
/// # Returns
/// * `Some(B)` — The inharmonicity coefficient if at least 3 valid partials exist.
/// * `None` — If there are too few partials or the regression fails.
///
/// # Side Effects
/// Stores the result in `measurement.calculated_b` for later retrieval.
#[deprecated(note = "After the pipeline migration happens, other algorithms will perform this role.")]
pub fn calculate_b_value(measurement: &mut KeyMeasurement) -> Option<f32> {
    if measurement.partials.len() < 3 {
        return None; // Need at least 3 points for a meaningful regression
    }

    // Prepare the (x, y) data points for linear regression
    // x = n^2, y = (f_n / n)^2
    let (xs, ys): (Vec<f64>, Vec<f64>) = measurement.partials.iter()
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
            measurement.calculated_b = Some(b_value as f32);
            return measurement.calculated_b;
        }
    }

    None
}
