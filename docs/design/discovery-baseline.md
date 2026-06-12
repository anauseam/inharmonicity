# Discovery Baseline (Pre-Optimization)

**Date**: 2026-06-12
**Commit**: `4a813643fe51e44954bce3de294c2368236136aa`
**Config**: Default `TwmConfig` (Maher & Beauchamp wind-instrument values)

> **Note**: This baseline is **early signal + the denominators for the post-opt regression gate — NOT the formal verdict**. The formal verdict will come from the MOBO ablation (Phase 4).

## Summary Table

| Mode | Total Keys Processed | Lock PASS | Lock FAIL |
| --- | --- | --- | --- |
| DISCRETE (Off) | 87 | 71 | 16 |
| REFINED (On) | 87 | 70 | 17 |

## Known Real False-Locks Status

The three known real false-locks are passing in both modes:
- `key_017_D2` (D2): PASS (both modes)
- `key_033_F#3` (F#3): PASS (both modes)
- `key_042_D#4` (D#4): PASS (both modes)

## Full Failure List

### DISCRETE Mode (Off)

- `key_000_A0`: FAIL_WRONG_KEY (Expected 0) (Locked on: 5)
- `key_004_C#1`: FAIL_WRONG_KEY (Expected 4) (Locked on: 11)
- `key_005_D1`: FAIL_WRONG_KEY (Expected 5) (Locked on: 11)
- `key_006_D#1`: FAIL_WRONG_KEY (Expected 6) (Locked on: 2)
- `key_010_G1`: FAIL_WRONG_KEY (Expected 10) (Locked on: 24)
- `key_012_A1`: FAIL_WRONG_KEY (Expected 12) (Locked on: 24)
- `key_016_C#2`: FAIL_WRONG_KEY (Expected 16) (Locked on: 22)
- `key_072_A6`: FAIL_WRONG_KEY (Expected 72) (Locked on: 35)
- `key_080_F7`: FAIL_WRONG_KEY (Expected 80) (Locked on: 68)
- `key_081_F#7`: FAIL_WRONG_KEY (Expected 81) (Locked on: 1)
- `key_082_G7`: FAIL_WRONG_KEY (Expected 82) (Locked on: 1)
- `key_083_G#7`: FAIL_WRONG_KEY (Expected 83) (Locked on: 0)
- `key_084_A7`: FAIL_WRONG_KEY (Expected 84) (Locked on: 0)
- `key_085_A#7`: FAIL_WRONG_KEY (Expected 85) (Locked on: 1)
- `key_086_B7`: FAIL_WRONG_KEY (Expected 86) (Locked on: 0)
- `key_087_C8`: FAIL_WRONG_KEY (Expected 87) (Locked on: 1)

### REFINED Mode (On)

- `key_000_A0`: FAIL_WRONG_KEY (Expected 0) (Locked on: 5) s_win: -8.5c
- `key_001_A#0`: FAIL_WRONG_KEY (Expected 1) (Locked on: 5) s_win: -20.0c
- `key_004_C#1`: FAIL_WRONG_KEY (Expected 4) (Locked on: 16) s_win: +38.8c
- `key_005_D1`: FAIL_WRONG_KEY (Expected 5) (Locked on: 17) s_win: -5.6c
- `key_006_D#1`: FAIL_WRONG_KEY (Expected 6) (Locked on: 0) s_win: -68.3c
- `key_010_G1`: FAIL_WRONG_KEY (Expected 10) (Locked on: 24) s_win: +15.8c
- `key_012_A1`: FAIL_WRONG_KEY (Expected 12) (Locked on: 24) s_win: +15.8c
- `key_034_G3`: FAIL_WRONG_KEY (Expected 34) (Locked on: 35) s_win: +14.7c
- `key_072_A6`: FAIL_WRONG_KEY (Expected 72) (Locked on: 35) s_win: +3.3c
- `key_080_F7`: FAIL_WRONG_KEY (Expected 80) (Locked on: 68) s_win: +1.1c
- `key_081_F#7`: FAIL_WRONG_KEY (Expected 81) (Locked on: 2) s_win: +1.1c
- `key_082_G7`: FAIL_WRONG_KEY (Expected 82) (Locked on: 70) s_win: -11.1c
- `key_083_G#7`: FAIL_WRONG_KEY (Expected 83) (Locked on: 0) s_win: -2.8c
- `key_084_A7`: FAIL_WRONG_KEY (Expected 84) (Locked on: 2) s_win: -3.3c
- `key_085_A#7`: FAIL_WRONG_KEY (Expected 85) (Locked on: 73) s_win: -3.8c
- `key_086_B7`: FAIL_WRONG_KEY (Expected 86) (Locked on: 74) s_win: -3.3c
- `key_087_C8`: FAIL_WRONG_KEY (Expected 87) (Locked on: 1) s_win: +1.1c

## Refined Scale Summary

**Distribution**: min=-76.2c, median=+3.0c, max=+38.9c

No keys hit the ±80 cents window edge clipping limit (although -76.2c approaches it).

## Goertzel Partial 1 Tracking Summary

### DISCRETE Mode (Off)

- **Keys where Partial 1 was completely DEAD**: 0
- **Keys where Partial 1 struggled (flickered ALIVE/DEAD)**: 8
  - `key_005_D1`: 25 ALIVE / 1 DEAD
  - `key_031_E3`: 45 ALIVE / 1 DEAD
  - `key_080_F7`: 11 ALIVE / 6 DEAD
  - `key_081_F#7`: 16 ALIVE / 1 DEAD
  - `key_082_G7`: 11 ALIVE / 5 DEAD
  - `key_084_A7`: 19 ALIVE / 2 DEAD
  - `key_085_A#7`: 6 ALIVE / 4 DEAD
  - `key_086_B7`: 6 ALIVE / 1 DEAD

### REFINED Mode (On)

- **Keys where Partial 1 was completely DEAD**: 0
- **Keys where Partial 1 struggled (flickered ALIVE/DEAD)**: 7
  - `key_005_D1`: 20 ALIVE / 1 DEAD
  - `key_031_E3`: 45 ALIVE / 1 DEAD
  - `key_080_F7`: 10 ALIVE / 6 DEAD
  - `key_082_G7`: 9 ALIVE / 5 DEAD
  - `key_084_A7`: 18 ALIVE / 2 DEAD
  - `key_085_A#7`: 7 ALIVE / 4 DEAD
  - `key_086_B7`: 3 ALIVE / 1 DEAD
