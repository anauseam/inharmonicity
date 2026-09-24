//! # Per-key advisories — what a curve's [`CurveKeyFlags`] tell the user
//!
//! Which flags mark a measurement as suspect and which are informational, and
//! the words for each, so a flag reads the same wherever it is shown.
//
// Only `excluded` and `negative_stretch` are suspect. Over both capture sets
// that pair fires on no key, while `curve_b_fallback` marks 21–22 of 88 (20 of
// them treble, plus every unmeasured key) and `giordano_excluded` 39–40: styled
// as errors, those two would paint most of the keyboard red.

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
    flags.excluded || flags.negative_stretch
}

/// The suspect flag to show first, or `None` when the key is fine. Both flags
/// at once is the ordinary case (an exclusion the final curve still violates),
/// and `excluded` is the more actionable of the two.
pub fn suspect(flags: &CurveKeyFlags) -> Option<Advisory> {
    advisories(flags)
        .into_iter()
        .find(|a| a.severity == Severity::Suspect)
}

/// Every advisory for a key, suspect ones first.
pub fn advisories(flags: &CurveKeyFlags) -> Vec<Advisory> {
    let mut out = Vec::new();
    if flags.excluded {
        out.push(Advisory {
            severity: Severity::Suspect,
            reason: "Measured B excluded — its octave implies a negative stretch.",
            hint: Some("Recapture this key."),
        });
    }
    if flags.negative_stretch {
        out.push(Advisory {
            severity: Severity::Suspect,
            reason: "The curve still runs backwards across this key's octave.",
            hint: Some("Recapture this key and its octave partner."),
        });
    }
    if flags.curve_b_fallback {
        out.push(Advisory {
            severity: Severity::Informational,
            reason: if flags.measured {
                "Curve B is prior-dominated here — normal in the treble, where \
                 few partials survive."
            } else {
                "No measurement yet; the curve uses the instrument's B fit."
            },
            hint: None,
        });
    }
    if flags.giordano_excluded {
        out.push(Advisory {
            severity: Severity::Informational,
            reason: "Contributed no ρ point to engine (c)'s calibration.",
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

    /// The informational flags must never reach the red ✗, however they
    /// combine: they cover most of the treble and every unmeasured key.
    #[test]
    fn only_the_two_suspect_flags_mark_a_key() {
        let informational = CurveKeyFlags {
            measured: true,
            curve_b_fallback: true,
            giordano_excluded: true,
            ..CurveKeyFlags::default()
        };
        assert!(!is_suspect(&informational));
        assert!(suspect(&informational).is_none());
        assert_eq!(advisories(&informational).len(), 2);

        let unmeasured = CurveKeyFlags {
            curve_b_fallback: true,
            ..CurveKeyFlags::default()
        };
        assert!(!is_suspect(&unmeasured));

        for flags in [
            CurveKeyFlags {
                excluded: true,
                ..informational
            },
            CurveKeyFlags {
                negative_stretch: true,
                ..informational
            },
        ] {
            assert!(is_suspect(&flags));
            assert_eq!(suspect(&flags).unwrap().severity, Severity::Suspect);
            assert!(suspect(&flags).unwrap().hint.is_some());
        }
    }
}
