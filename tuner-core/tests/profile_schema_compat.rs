//! Profiles written before a field was renamed must still load, their values
//! intact: without an alias, `serde(default)` replaces a saved value silently.

use tuner_core::models::ProfileSettings;

/// The sustain-stability threshold has been spelled three ways. Every earlier
/// spelling must still carry its operator's calibration into the shipped field.
#[test]
fn every_earlier_stability_threshold_spelling_still_loads() {
    for old_name in [
        "ninos2_stability_threshold",
        "participation_stability_threshold",
    ] {
        let older = format!(
            r#"{{
            "nhwrsf_threshold": 0.9,
            "{old_name}": 7.5,
            "engine": "MultiBalanced",
            "reference_mode": "Curve"
        }}"#
        );

        let s: ProfileSettings =
            serde_json::from_str(&older).expect("older profile must deserialize");
        assert_eq!(
            s.sustain_stability_threshold, 7.5,
            "`{old_name}` profiles lost their calibration to the default"
        );
        assert_eq!(s.nhwrsf_threshold, 0.9);
    }
}

/// The current name round-trips, and an absent field still falls back.
#[test]
fn current_name_round_trips_and_absence_defaults() {
    let current = r#"{ "nhwrsf_threshold": 0.9, "sustain_stability_threshold": 7.5 }"#;
    let s: ProfileSettings = serde_json::from_str(current).expect("current profile");
    assert_eq!(s.sustain_stability_threshold, 7.5);

    let absent: ProfileSettings = serde_json::from_str("{}").expect("empty settings");
    assert!(absent.sustain_stability_threshold > 0.0);
}
