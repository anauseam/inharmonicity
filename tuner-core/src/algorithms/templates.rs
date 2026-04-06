//! # SparseTemplate Generator
//!
//! Generates structural matched-filters for 88 piano keys using the 
//! two-asymptote β model and Rayleigh amplitude weighting.
//!
//! The Rayleigh distribution models the physical spectral energy shift
//! across the keyboard: bass strings radiate peak energy from upper partials
//! (3rd–4th), while treble strings radiate from the fundamental.

/// Maximum number of partials per template.
const MAX_PARTIALS: usize = 12;

/// A pre-computed spectral fingerprint for a single piano key.
///
/// Encodes the expected FFT bin positions and Rayleigh-weighted amplitude
/// values for the first N partials of this key under a standard upright
/// piano inharmonicity profile. All weight vectors are L2-normalized
/// at build time for direct cosine similarity scoring.
#[derive(Debug, Clone, Copy)]
pub struct SparseTemplate {
    /// MIDI-style index: 0 = A0 (27.5 Hz), 87 = C8 (4186 Hz).
    pub key_index: u8,
    /// Equal-temperament fundamental frequency (Hz).
    pub f0_et: f32,
    /// Nominal inharmonicity coefficient from two-asymptote β model.
    pub beta_nominal: f32,
    /// Number of valid entries in `bins` and `weights`.
    pub partial_count: usize,
    /// Expected bin index for each partial (up to 12).
    pub bins: [usize; MAX_PARTIALS],
    /// L2-normalized Rayleigh weights for each partial (up to 12).
    pub weights: [f32; MAX_PARTIALS],
}

// ─── Inharmonicity Model Constants ───────────────────────────────────────────

const BETA_LOW: f32 = 3.5e-4;   // low-register inharmonicity ceiling
const BETA_K: f32 = 0.045;      // exponential decay rate across keyboard
const BETA_HIGH: f32 = 1.5e-5;  // high-register inharmonicity floor

// ─── Rayleigh Weight Profile Constants ───────────────────────────────────────

/// Key index of A0 (first key on 88-key piano, 0-indexed).
const KEY_A0: f32 = 0.0;
/// Key index of C4 (middle C, key 39 in 0-indexed).
const KEY_C4: f32 = 39.0;
/// Maximum Rayleigh peak partial index (bass floor — weight peaks at 4th partial).
const N_PEAK_MAX: f32 = 4.0;
/// Minimum Rayleigh peak partial index (treble ceiling — weight peaks at fundamental).
const N_PEAK_MIN: f32 = 1.0;

// ─── Rayleigh Helper ─────────────────────────────────────────────────────────

/// Computes the Rayleigh peak partial index for a given key.
///
/// Linearly interpolates from n_peak=4 at A0 to n_peak=1 at C4.
/// Keys above C4 are clamped to n_peak=1 (fundamental-dominant).
///
/// This models the physical reality that bass strings radiate most
/// energy from upper partials, not the fundamental.
#[inline]
fn n_peak_for_key(key_index: u8) -> f32 {
    let k = key_index as f32;
    if k >= KEY_C4 {
        return N_PEAK_MIN;
    }
    N_PEAK_MAX - (N_PEAK_MAX - N_PEAK_MIN) * ((k - KEY_A0) / (KEY_C4 - KEY_A0))
}

/// Computes the Rayleigh weight for partial index `p` at the given `n_peak`.
///
/// w_p = (p / n_peak²) * exp(-p² / (2 * n_peak²))
#[inline]
fn rayleigh_weight(p: f32, n_peak: f32) -> f32 {
    let n_pk_sq = n_peak * n_peak;
    (p / n_pk_sq) * (-p * p / (2.0 * n_pk_sq)).exp()
}

// ─── Template Builder ────────────────────────────────────────────────────────

/// Returns the theoretical expected beta (inharmonicity coefficient) for a given piano key.
/// 
/// Uses a two-asymptote β profile typical of medium to large uprights and grands.
pub fn get_expected_beta(key_index: u8) -> f32 {
    BETA_LOW * (-BETA_K * key_index as f32).exp() + BETA_HIGH
}

/// Builds the full 88-key SparseTemplate array for a standard upright piano.
///
/// Uses two-asymptote β model:
///   β(key) = β_low * exp(-k * key_index) + β_high
///
/// Partial bin positions are calculated using the inharmonic partial formula:
///   f_n = n * f0_ET * sqrt(1 + β * n²)
///
/// Weights follow a Rayleigh distribution whose peak transitions from partial 4
/// (at A0) to partial 1 (at C4), modeling the physical spectral energy shift
/// across the keyboard. All weight vectors are L2-normalized before storage.
pub fn build_templates(sample_rate: u32, window_size: usize) -> [SparseTemplate; 88] {
    let mut templates = [SparseTemplate {
        key_index: 0,
        f0_et: 0.0,
        beta_nominal: 0.0,
        partial_count: 0,
        bins: [0; MAX_PARTIALS],
        weights: [0.0; MAX_PARTIALS],
    }; 88];

    let hz_per_bin = sample_rate as f32 / window_size as f32;
    let nyquist = sample_rate as f32 / 2.0;
    let max_valid_bin = window_size / 2 - 1;

    for key_index in 0..88_u32 {
        // Equal Temperament F0 (A0 = 27.5 Hz at index 0)
        let f0_et = 27.5 * 2.0_f32.powf(key_index as f32 / 12.0);
        
        // Two-asymptote beta model
        let beta = get_expected_beta(key_index as u8);
        let n_pk = n_peak_for_key(key_index as u8);
        
        let mut bins = [0usize; MAX_PARTIALS];
        let mut weights = [0.0f32; MAX_PARTIALS];
        let mut count = 0;

        for n in 1..=MAX_PARTIALS {
            let n_f = n as f32;
            let f_n = n_f * f0_et * (1.0 + beta * n_f * n_f).sqrt();
            
            if f_n >= nyquist { break; }
            
            let bin = (f_n / hz_per_bin).round() as usize;
            if bin > max_valid_bin { break; }
            
            let weight = rayleigh_weight(n_f, n_pk);
            
            bins[count] = bin;
            weights[count] = weight;
            count += 1;
        }

        // ── L2 Normalize ──
        let mut norm_sq = 0.0_f32;
        for i in 0..count {
            norm_sq += weights[i] * weights[i];
        }
        let norm = norm_sq.sqrt();
        if norm > 1e-10 {
            for i in 0..count {
                weights[i] /= norm;
            }
        }

        templates[key_index as usize] = SparseTemplate {
            key_index: key_index as u8,
            f0_et,
            beta_nominal: beta,
            partial_count: count,
            bins,
            weights,
        };
    }

    templates
}

// ─── Template Matcher ────────────────────────────────────────────────────────

/// Scores a magnitude spectrum against all 88 templates using cosine similarity.
///
/// Because all weight vectors are L2-normalized to unit norm at build time,
/// the scoring simplifies to `dot_product / ‖magnitudes‖`.
///
/// Returns `(key_index_0_to_87, target_f0_et, target_beta, cosine_similarity_score)`.
pub fn match_template(templates: &[SparseTemplate; 88], magnitudes: &[f32]) -> (usize, f32, f32, f32) {
    let mut best_score = -1.0_f32;
    let mut best_key = 0;
    
    // Pre-calculate the L2 norm of the magnitudes vector
    let mut mag_norm_sq = 0.0_f32;
    for &m in magnitudes {
        mag_norm_sq += m * m;
    }
    let mag_norm = mag_norm_sq.sqrt();
    
    if mag_norm < 1e-6 {
        return (0, templates[0].f0_et, templates[0].beta_nominal, 0.0);
    }
    
    let inv_mag_norm = 1.0 / mag_norm;

    for template in templates {
        let mut dot_product = 0.0_f32;
        
        for i in 0..template.partial_count {
            let bin = template.bins[i];
            if bin < magnitudes.len() {
                dot_product += magnitudes[bin] * template.weights[i];
            }
        }
        
        let score = dot_product * inv_mag_norm;
        
        if score > best_score {
            best_score = score;
            best_key = template.key_index as usize;
        }
    }
    
    let best = &templates[best_key];
    (best_key, best.f0_et, best.beta_nominal, best_score)
}
