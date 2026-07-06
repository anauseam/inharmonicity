#!/usr/bin/env python3
"""Score a TWM constant set against the real diagnostic captures — pass/fail per
key, dependency-free (stdlib csv only; no pandas/matplotlib).

Replicates test_engine_all.py's lock logic exactly: merge gatekeeper + engine
per frame, keep only 'Stable' frames, then declare a lock when the per-frame
winning key (`key_idx`) repeats for 3 consecutive stable frames, and compare to
the folder-name ground truth. This is the Phase 5 real-data gate.

Usage:
  python3 scripts/validate_config.py [--refine] [--config "p q r rho lambda"]
"""
import argparse
import csv
import os
import subprocess

BASE = "./diagnostics"
ENGINE = "./target/release/examples/diagnose_engine"
GATE = "./target/release/examples/diagnose_gatekeeper"


def lock_for_key(key_dir, refine, config, sum_forward, stretch, b_deadzone, nonpeak, smoothness):
    raw = os.path.join(key_dir, "audio_full_event.raw")
    if not os.path.exists(raw):
        raw = os.path.join(key_dir, "audio.raw")
    if not os.path.exists(raw):
        return None

    subprocess.run([GATE, raw], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    cmd = [ENGINE, raw]
    if refine:
        cmd.append("--refine")
    if config:
        cmd += ["--config", config]
    if sum_forward:
        cmd.append("--sum-forward")
    if stretch:
        cmd.append("--stretch")
    if b_deadzone:
        cmd += ["--b-deadzone", str(b_deadzone)]
    if nonpeak:
        cmd += ["--nonpeak", str(nonpeak)]
    if smoothness:
        cmd += ["--smoothness", str(smoothness)]
    subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)

    gate_csv = os.path.join(key_dir, "gatekeeper.csv")
    peaks_csv = os.path.join(key_dir, "peaks.csv")
    if not (os.path.exists(gate_csv) and os.path.exists(peaks_csv)):
        return ("FAIL_NO_CSV", -1)

    with open(gate_csv) as f:
        state = {int(r["frame_idx"]): r["state_name"] for r in csv.DictReader(f)}
    with open(peaks_csv) as f:
        winner = {int(r["frame"]): int(r["key_idx"]) for r in csv.DictReader(f)}

    # Stable frames, in frame order (the merge + state_name=='Stable' filter).
    stable = [(fr, winner[fr]) for fr in sorted(winner)
              if state.get(fr) == "Stable"]
    if not stable:
        return ("FAIL_NEVER_STABLE", -1)

    # 3-frame consistency on the winning key.
    cand, count = -1, 0
    for _, w in stable:
        if w == cand:
            count += 1
        else:
            cand, count = w, 1
        if count >= 3:
            return ("LOCK", cand)
    return ("FAIL_NO_3_FRAME_LOCK", -1)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--refine", action="store_true")
    ap.add_argument("--config", default=None, help='"p q r rho lambda" (lambda may be inf)')
    ap.add_argument("--sum-forward", action="store_true",
                    help="EXPERIMENT: sum the forward error instead of /N averaging")
    ap.add_argument("--stretch", action="store_true",
                    help="EXPERIMENT: center templates on the Railsback-stretched pitch")
    ap.add_argument("--b-deadzone", type=float, default=0.0,
                    help="EXPERIMENT: n-kernel forward-error deadzone scale c (0=off)")
    ap.add_argument("--nonpeak", type=float, default=0.0,
                    help="EXPERIMENT: Duan non-peak per-hallucinated-partial penalty (0=off)")
    ap.add_argument("--smoothness", type=float, default=0.0,
                    help="EXPERIMENT: Emiya matched-partial amplitude-incoherence penalty (0=off)")
    args = ap.parse_args()

    for ex, feats in [(ENGINE, ["--features", "telemetry"]), (GATE, [])]:
        tgt = ex.split("/")[-1]
        subprocess.run(["cargo", "build", "--release", "--example", tgt] + feats,
                       check=True, stdout=subprocess.DEVNULL)

    keys = sorted(d for d in os.listdir(BASE) if d.startswith("key_"))
    mode = "REFINED" if args.refine else "DISCRETE"
    sf = " | SUM-FORWARD" if args.sum_forward else ""
    st = " | STRETCH" if args.stretch else ""
    print(f"{mode} | config={args.config or 'DEFAULT'}{sf}{st} | {len(keys)} keys")

    npass = 0
    fails = []
    # register tally: (n, pass) for bass 0-26, mid 27-59, treble 60-87
    reg = [[0, 0], [0, 0], [0, 0]]
    for kd in keys:
        expected = int(kd.split("_")[1])
        res = lock_for_key(os.path.join(BASE, kd), args.refine, args.config, args.sum_forward, args.stretch, args.b_deadzone, args.nonpeak, args.smoothness)
        if res is None:
            continue
        status, locked = res
        ok = status == "LOCK" and locked == expected
        ri = 0 if expected <= 26 else (1 if expected <= 59 else 2)
        reg[ri][0] += 1
        reg[ri][1] += int(ok)
        if ok:
            npass += 1
        else:
            detail = f"locked {locked}" if status == "LOCK" else status
            fails.append(f"{kd} -> {detail}")

    total = sum(r[0] for r in reg)
    print(f"PASS {npass}/{total}   "
          f"bass {reg[0][1]}/{reg[0][0]}  mid {reg[1][1]}/{reg[1][0]}  treble {reg[2][1]}/{reg[2][0]}")
    if fails:
        print("FAILURES:")
        for fl in fails:
            print("  " + fl)


if __name__ == "__main__":
    main()
