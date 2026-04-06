//! # Engine (Thread 2) — Fundamental Frequency Detection
//!
//! The "Brains" of the pipeline. The Engine is orchestrated by the `AudioPipeline`
//! after the signal has been validated by the Gatekeeper. Its sole responsibility
//! is to process the signal and extract the exact fundamental frequency.
//!
//! ## Sequence of processing:
//!
//! 1. Stage 1: Correlate against 88 SparseTemplates on the pre-computed magnitude spectrum.
//! 2. Route conditionals (is_bass) based on the cosine similarity crossover.
//! 3. Stage 1.5: Phantom partial mask (bass only).
//! 4. Sub-octave probe: O(1) check for 2× octave confusion in bass register.
//! 5. Stages 2+3: Delegate to MAT (Median-Adjustive Trajectories) for partial extraction and F0 refinement.

use crate::algorithms::{mat, templates};
use crate::audio::{BASS_WINDOW_SIZE, WINDOW_SIZE};
use crate::pipeline::ProcessingFrame;

/// Fractional energy threshold for the sub-octave probe.
/// Both the 3rd and 5th sub-octave partials must exceed this fraction
/// of the winning template's fundamental bin magnitude to trigger an octave bounce.
const SUB_OCTAVE_PROBE_THRESHOLD: f32 = 0.25;

/// Bass/treble register crossover key index.
/// Key 39 = C4 (261.6 Hz, middle C). Notes identified below this by the 8192-pt
/// bass FFT use the bass path; notes at or above use the treble path for its
/// superior temporal resolution.
const CROSSOVER_KEY: usize = 39;

/// Result of a successful pitch detection frame.
#[derive(Debug, Clone)]
pub struct PitchResult {
    pub f0: f32,
    pub partial_freqs: [f32; 12],
    pub partial_ns: [u32; 12],
    pub partial_count: usize,
    pub suspend_beta_update: bool,
}

/// The Fundamental Frequency ($f_0$) Engine.
///
/// Simply acts as a designated router between pure algorithm modules.
pub struct Engine {
    pub sample_rate: u32,
    pub inharmonicity_b: Option<f32>,
    templates_treble: [templates::SparseTemplate; 88],
    templates_bass: [templates::SparseTemplate; 88],
    /// Scratch space for partial frequencies measured by MAT.
    pub mat_partial_freqs: [f32; 12],
    /// Scratch space for partial harmonic indices measured by MAT.
    pub mat_partial_ns: [u32; 12],
}

impl Engine {
    /// Creates a new Engine with default algorithms.
    pub fn new(sample_rate: u32) -> Self {
        let templates_treble = templates::build_templates(sample_rate, WINDOW_SIZE);
        let templates_bass = templates::build_templates(sample_rate, BASS_WINDOW_SIZE);

        Engine {
            sample_rate,
            inharmonicity_b: None,
            templates_treble,
            templates_bass,
            mat_partial_freqs: [0.0; 12],
            mat_partial_ns: [0; 12],
        }
    }

    /// Executes the primary DSP detection loop for a single frame.
    pub fn process(
        &mut self,
        frame: &mut ProcessingFrame,
        is_silence: bool,
        is_new_onset: bool,
    ) -> Option<PitchResult> {
        if is_silence || is_new_onset {
            return None;
        }

        // ── Stage 1: Dual-Track Template Correlation ──────────

        let mag_count_treble = WINDOW_SIZE / 2;
        let mag_count_bass = BASS_WINDOW_SIZE / 2;

        let (key_treble, f0_treble, beta_treble, score_treble) = templates::match_template(
            &self.templates_treble,
            &frame.treble_magnitude_buffer[..mag_count_treble],
        );

        let (key_bass, f0_bass, beta_bass, score_bass) = templates::match_template(
            &self.templates_bass,
            &frame.bass_magnitude_buffer[..mag_count_bass],
        );

        // ── Register Crossover ──
        // Cosine similarity scores are not comparable across different FFT dimensions
        // (the coarser treble FFT concentrates energy into fewer bins, inflating scores).
        // Instead, use the bass path's key identification to decide: the 8192-pt FFT has
        // 4× the frequency resolution and reliably discriminates across the full spectrum.
        // If it identifies a bass-register key, use the bass path. Otherwise, use treble
        // for its superior temporal resolution on higher-frequency signals.
        let is_bass = key_bass < CROSSOVER_KEY;

        // DEBUG: Crossover diagnostics
        eprintln!(
            "[ENGINE] treble: key={} f0={:.1}Hz score={:.6} | bass: key={} f0={:.1}Hz score={:.6} | route={}",
            key_treble,
            f0_treble,
            score_treble,
            key_bass,
            f0_bass,
            score_bass,
            if is_bass { "BASS" } else { "TREBLE" }
        );

        let (mut f0_et, mut beta_nominal, winning_key, active_magnitudes) = if is_bass {
            let mags = &mut frame.bass_magnitude_buffer[..mag_count_bass];
            // ── Stage 1.5: Phantom Partial Mask (bass register only) ──
            // crate::algorithms::phantom::apply_phantom_mask(
            //     mags,
            //     f0_bass,
            //     beta_bass,
            //     self.sample_rate,
            //     BASS_WINDOW_SIZE,
            // );
            (f0_bass, beta_bass, key_bass, &*mags)
        } else {
            (
                f0_treble,
                beta_treble,
                key_treble,
                &frame.treble_magnitude_buffer[..mag_count_treble],
            )
        };

        // ── Sub-Octave Probe (bass register, key ≥ 12 only) ──
        // Checks whether the winning template is an octave-up false positive
        // by looking for the 3rd and 5th partial energy of the sub-octave key.
        if is_bass && winning_key >= 12 {
            if let Some((sub_f0, sub_beta)) =
                self.sub_octave_probe(active_magnitudes, winning_key, f0_et)
            {
                f0_et = sub_f0;
                beta_nominal = sub_beta;
            }
        }

        // ── Stages 2 + 3: Guided Trajectory & MAT Evaluation ────────────
        let mat_result = mat::detect_pitch_mat(
            active_magnitudes,
            self.sample_rate,
            f0_et,
            beta_nominal,
            is_bass,
            &mut self.mat_partial_freqs,
            &mut self.mat_partial_ns,
        )?;

        let (f0, partial_count, suspend_beta_update) = mat_result;

        // DEBUG: MAT output diagnostics
        eprintln!(
            "[MAT] seed_f0={:.2}Hz → refined_f0={:.2}Hz | partials={} | suspend_beta={}",
            f0_et, f0, partial_count, suspend_beta_update
        );

        // NOTE: We see MAT scews the fundamental frequency, so we use the template's f0
        // as the final f0.This will be fixed in the future.
        Some(PitchResult {
            f0: f0_et,
            partial_freqs: self.mat_partial_freqs,
            partial_ns: self.mat_partial_ns,
            partial_count,
            suspend_beta_update,
        })
    }

    /// Sub-octave energy probe for 2× octave confusion detection.
    ///
    /// After the template matcher selects a bass key, this function checks
    /// whether the sub-octave key (one octave below) has substantial energy
    /// at its 3rd and 5th inharmonically-stretched partial positions. If both
    /// exceed `SUB_OCTAVE_PROBE_THRESHOLD` of the winning template's
    /// fundamental bin magnitude, the sub-octave is confirmed as the true note.
    ///
    /// Uses ±1 bin local maximum for spectral smearing robustness.
    ///
    /// Returns `Some((sub_f0_et, sub_beta))` if the probe confirms the sub-octave,
    /// `None` if the original template result stands.
    fn sub_octave_probe(
        &self,
        magnitudes: &[f32],
        winning_key: usize,
        winning_f0_et: f32,
    ) -> Option<(f32, f32)> {
        let sub_key = winning_key - 12;
        let sub_template = &self.templates_bass[sub_key];
        let sub_f0 = sub_template.f0_et;
        let sub_beta = sub_template.beta_nominal;

        let hz_per_bin = self.sample_rate as f32 / BASS_WINDOW_SIZE as f32;
        let max_bin = magnitudes.len().saturating_sub(2); // Leave room for +1 neighbor

        // Reference: energy at the winning template's fundamental bin
        let winner_bin = (winning_f0_et / hz_per_bin).round() as usize;
        if winner_bin < 1 || winner_bin > max_bin {
            return None;
        }
        let ref_energy = local_max_3(magnitudes, winner_bin);
        if ref_energy < 1e-10 {
            return None;
        }

        let threshold = ref_energy * SUB_OCTAVE_PROBE_THRESHOLD;

        // Probe: 3rd partial of sub-octave key
        let f3 = 3.0 * sub_f0 * (1.0 + sub_beta * 9.0).sqrt();
        let bin3 = (f3 / hz_per_bin).round() as usize;
        if bin3 < 1 || bin3 > max_bin {
            return None;
        }
        let energy3 = local_max_3(magnitudes, bin3);

        // Probe: 5th partial of sub-octave key
        let f5 = 5.0 * sub_f0 * (1.0 + sub_beta * 25.0).sqrt();
        let bin5 = (f5 / hz_per_bin).round() as usize;
        if bin5 < 1 || bin5 > max_bin {
            return None;
        }
        let energy5 = local_max_3(magnitudes, bin5);

        // AND gate: both partials must exceed threshold
        if energy3 >= threshold && energy5 >= threshold {
            Some((sub_f0, sub_beta))
        } else {
            None
        }
    }
}

/// Returns the maximum magnitude across `magnitudes[bin-1..=bin+1]`.
///
/// Accounts for spectral leakage from windowing and β drift
/// that can push peak energy into adjacent bins.
#[inline]
fn local_max_3(magnitudes: &[f32], bin: usize) -> f32 {
    let left = magnitudes[bin - 1];
    let center = magnitudes[bin];
    let right = magnitudes[bin + 1];
    left.max(center).max(right)
}
