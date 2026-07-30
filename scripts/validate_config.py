#!/usr/bin/env python3
"""Score a TWM constant set against the real diagnostic captures — pass/fail per
key, dependency-free (stdlib csv only; no pandas/matplotlib).

Replicates the engine's lock logic exactly: merge gatekeeper + engine per frame,
keep only 'Stable' frames, then apply the M-of-N (binary-integration) lock rule
— lock the first key to win >= M of the last N stable-frame winners (`key_idx`),
Schwartz 1956 / Shnidman 1998 — and compare to the folder-name ground truth.
This is the Phase 5 real-data gate. The shipped rule is (M, N) = (7, 8) for the
refined path (ADR 0010); `--lock-m 3 --lock-n 3` reproduces the old
3-consecutive rule. The rule here is identical to `replay_lock_rules.py`'s
`eval_rule` and `Engine::record_stable_winner`.

Usage:
  python3 scripts/validate_config.py [--refine] [--config "p q r rho lambda"]
                                     [--base DIR] [--lock-m 7] [--lock-n 8]
"""
import argparse
import csv
import os
import subprocess
from collections import Counter, deque

ENGINE = "./target/release/examples/diagnose_engine"
GATE = "./target/release/examples/diagnose_gatekeeper"


def mofn_lock(winners, m, n):
    """First key to win >= m of the last n winners (deque(maxlen=n)); identical
    to replay_lock_rules.py eval_rule and the engine's record_stable_winner.
    m > n/2 => at most one key can hold >= m votes. Returns the key or None."""
    win = deque(maxlen=n)
    counts = Counter()
    for w in winners:
        if len(win) == n:
            counts[win[0]] -= 1
        win.append(w)
        counts[w] += 1
        if counts[w] >= m:
            return w
    return None


def lock_for_key(key_dir, refine, config, sum_forward, stretch, b_deadzone, nonpeak, smoothness, m, n):
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

    # Stable-frame winner sequence, in frame order (merge + 'Stable' filter).
    stable = [winner[fr] for fr in sorted(winner)
              if state.get(fr) == "Stable"]
    if not stable:
        return ("FAIL_NEVER_STABLE", -1)

    # M-of-N binary-integration lock on the winning key.
    locked = mofn_lock(stable, m, n)
    if locked is None:
        return ("FAIL_NO_LOCK", -1)
    return ("LOCK", locked)


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
    ap.add_argument("--base", default="./diagnostics",
                    help="capture dir holding key_* subdirs (default ./diagnostics)")
    ap.add_argument("--lock-m", type=int, default=7,
                    help="M-of-N lock: votes required (default 7; ADR 0010 refined)")
    ap.add_argument("--lock-n", type=int, default=8,
                    help="M-of-N lock: window length (default 8; 3/3 = old rule)")
    args = ap.parse_args()
    if not args.lock_m > args.lock_n // 2:
        ap.error("--lock-m must exceed --lock-n/2 (majority => unique winner)")

    for ex, feats in [(ENGINE, ["--features", "telemetry"]), (GATE, [])]:
        tgt = ex.split("/")[-1]
        subprocess.run(["cargo", "build", "--release", "--example", tgt] + feats,
                       check=True, stdout=subprocess.DEVNULL)

    keys = sorted(d for d in os.listdir(args.base) if d.startswith("key_"))
    mode = "REFINED" if args.refine else "DISCRETE"
    sf = " | SUM-FORWARD" if args.sum_forward else ""
    st = " | STRETCH" if args.stretch else ""
    print(f"{mode} | config={args.config or 'DEFAULT'}{sf}{st} | "
          f"lock={args.lock_m}-of-{args.lock_n} | base={args.base} | {len(keys)} keys")

    npass = 0
    fails = []
    # register tally: (n, pass) for bass 0-26, mid 27-59, treble 60-87
    reg = [[0, 0], [0, 0], [0, 0]]
    for kd in keys:
        expected = int(kd.split("_")[1])
        res = lock_for_key(os.path.join(args.base, kd), args.refine, args.config, args.sum_forward, args.stretch, args.b_deadzone, args.nonpeak, args.smoothness, args.lock_m, args.lock_n)
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
