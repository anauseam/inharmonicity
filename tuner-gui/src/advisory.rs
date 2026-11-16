//! # Per-key advisories — what a curve's [`CurveKeyFlags`] tell the user
//!
//! Which flags mark a measurement as suspect and which are informational, and
//! the words for each, so a flag reads the same wherever it is shown.
//
// Only `b_above_physical_range` and `negative_stretch` are suspect; the blend's
// `excluded` marks keys whose B moves no target.
// report 0024

use tuner_core::models::CurveKeyFlags;

/// How a flag should read on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The measurement is doubted: red ✗, recapture recommended.
    Suspect,
    /// Context about how the curve treated this key. Never styled as an error.
    Informational,
}

/// One line about one key, ready to render.
#[derive(Debug, Clone, Copy)]
pub struct Advisory {
    pub severity: Severity,
    /// What the curve found, in one line.
    pub reason: &'static str,
    /// What to do about it; `None` where there is nothing to do.
    pub hint: Option<&'static str>,
}

/// Whether this key's measurement is doubted — the red-✗ predicate.
pub fn is_suspect(flags: &CurveKeyFlags) -> bool {
    flags.b_above_physical_range || flags.negative_stretch
}

/// The suspect flag to show first, or `None` when the key is fine. The physical range
/// comes first: it names the capture.
pub fn suspect(flags: &CurveKeyFlags) -> Option<Advisory> {
    advisories(flags)
        .into_iter()
        .find(|a| a.severity == Severity::Suspect)
}

/// Every advisory for a key, suspect ones first.
pub fn advisories(flags: &CurveKeyFlags) -> Vec<Advisory> {
    let mut out = Vec::new();
    if flags.b_above_physical_range {
        out.push(Advisory {
            severity: Severity::Suspect,
            reason: "Measured B is above what real strings reach: the capture misread its partials.",
            hint: Some("Recapture this key."),
        });
    }
    if flags.negative_stretch {
        out.push(Advisory {
            severity: Severity::Suspect,
            reason: "The curve runs backwards across this key's octave.",
            hint: Some("Recapture this key and its octave partner."),
        });
    }
    if !flags.measured {
        out.push(Advisory {
            severity: Severity::Informational,
            reason: "No measurement yet; the strobe reads the fundamental only.",
            hint: None,
        });
    }
    out
}

/// Per-key suspect marks for a whole curve.
pub fn suspect_keys(flags: &[CurveKeyFlags; 88]) -> [bool; 88] {
    core::array::from_fn(|k| is_suspect(&flags[k]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The blend's flags never reach the red ✗, however they combine.
    #[test]
    fn only_the_two_suspect_flags_mark_a_key() {
        let quiet = CurveKeyFlags {
            measured: true,
            curve_b_fallback: true,
            excluded: true,
            giordano_excluded: true,
            ..CurveKeyFlags::default()
        };
        assert!(!is_suspect(&quiet));
        assert!(suspect(&quiet).is_none());
        assert!(advisories(&quiet).is_empty());

        let unmeasured = CurveKeyFlags::default();
        assert!(!is_suspect(&unmeasured));
        assert_eq!(advisories(&unmeasured).len(), 1);

        for flags in [
            CurveKeyFlags {
                b_above_physical_range: true,
                ..quiet
            },
            CurveKeyFlags {
                negative_stretch: true,
                ..quiet
            },
        ] {
            assert!(is_suspect(&flags));
            assert_eq!(suspect(&flags).unwrap().severity, Severity::Suspect);
            assert!(suspect(&flags).unwrap().hint.is_some());
        }
    }
}
