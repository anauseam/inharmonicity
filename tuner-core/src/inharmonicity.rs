use serde::{Serialize, Deserialize};
use std::collections::BTreeMap;
use linreg::linear_regression;


/// Represents a single measured partial of a note.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Partial {
    pub number: u32,      // The partial number (n=1, 2, 3...)
    pub frequency: f32,   // The measured frequency in Hz
}

/// Stores all the measured partials for a single piano key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyMeasurement {
    pub key_index: u8,   // The piano key index (0-87)
    pub partials: Vec<Partial>,
    pub calculated_b: Option<f32>, // Store the B value after calculation
}

/// Represents the complete inharmonicity profile for a specific piano.
/// This is the top-level object you will save to and load from a file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InharmonicityProfile {
    // A BTreeMap is great here because it keeps the keys sorted automatically.
    // It maps a key_index (u8) to its measurement data.
    pub measurements: BTreeMap<u8, KeyMeasurement>,
}


impl KeyMeasurement {
    /// Calculates the inharmonicity constant 'B' for this key's measurements.
    pub fn calculate_b_value(&mut self) -> Option<f32> {
        if self.partials.len() < 3 {
            return None; // Need at least 3 points for a meaningful regression
        }
        
        // Prepare the (x, y) data points for linear regression
        // x = n^2, y = (f_n / n)^2
        let (xs, ys) : (Vec<f64>, Vec<f64>) = self.partials.iter()
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