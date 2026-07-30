#!/usr/bin/env python3
"""Offline M-of-N (binary-integration) lock-rule replay over real captures.

The experiment queued by `docs/design/sequential-detection-design.md` (gate 2):
replace the production lock rule — first key to win 3 *consecutive* stable
frames — with the published M-of-N binary-integration rule (Schwartz 1956;
Shnidman 1998): lock on the first key to win >= M of the last N stable frames.
The production rule is exactly the degenerate M = N = 3 case, which this
harness uses as its correctness gate (it must reproduce validate_config.py's
pass/fail sets bit-for-bit from the same CSVs).

Pure post-processing: no engine changes. Three subcommands:

  cache    Run diagnose_gatekeeper + diagnose_engine (DEFAULT TwmConfig,
           telemetry build — mirroring validate_config.py) over every
           `key_*` dir under --base, merge peaks.csv `key_idx` with
           gatekeeper.csv Stable frames (the validate_config.py merge,
           replicated exactly), and write one row per capture:
           dir, expected, n_stable, winners (';'-joined stable-frame
           winner sequence in frame order).

  sweep    Evaluate every (M, N) with M > N/2 (majority => unique winner
           per frame) over a cache file. Prints the baseline row, the
           full-window plurality ceiling (cross-check against the
           Prompt A' gross ceilings), a per-N best-M summary, and writes
           the full surface to --surface-out.

  concord  The pre-registered two-instrument concordance selection
           (sequential-detection-design.md, "Replay protocol"): plateau =
           within --p1-tol keys / --p2-tol dumps of each surface's max at
           N <= --n-budget; candidate region = intersection; recommend
           min N then min M; print the recommendation's neighbor scores
           (isolated-spike check) and its detailed fixed/broken/latency
           report on both instruments.

Dependency-free (stdlib only), like validate_config.py.

Usage:
  python3 scripts/replay_lock_rules.py cache --base diagnostics_piano_1 \
      --out replay_cache/p1_discrete.csv [--refine] [--jobs 8] [--tidy]
  python3 scripts/replay_lock_rules.py sweep replay_cache/p1_discrete.csv \
      [--n-max 43] [--surface-out replay_cache/p1_discrete_surface.csv]
  python3 scripts/replay_lock_rules.py concord \
      --p1 replay_cache/p1_refined.csv --p2 replay_cache/p2_refined.csv \
      [--n-budget 21] [--p1-tol 1] [--p2-tol 7]
"""
import argparse
import csv
import os
import subprocess
from collections import Counter, deque
from concurrent.futures import ThreadPoolExecutor

ENGINE = "./target/release/examples/diagnose_engine"
GATE = "./target/release/examples/diagnose_gatekeeper"

# Register split, identical to validate_config.py.
def register(key):
    return 0 if key <= 26 else (1 if key <= 59 else 2)

REG_NAMES = ["bass", "mid", "treble"]


# ---------------------------------------------------------------- cache phase

def stable_winner_seq(key_dir):
    """validate_config.py's merge, replicated exactly: peaks.csv key_idx per
    frame, filtered to gatekeeper.csv state_name == 'Stable', frame order."""
    gate_csv = os.path.join(key_dir, "gatekeeper.csv")
    peaks_csv = os.path.join(key_dir, "peaks.csv")
    if not (os.path.exists(gate_csv) and os.path.exists(peaks_csv)):
        return None
    with open(gate_csv) as f:
        state = {int(r["frame_idx"]): r["state_name"] for r in csv.DictReader(f)}
    with open(peaks_csv) as f:
        winner = {int(r["frame"]): int(r["key_idx"]) for r in csv.DictReader(f)}
    return [winner[fr] for fr in sorted(winner) if state.get(fr) == "Stable"]


def cache_one(base, kd, refine, tidy):
    key_dir = os.path.join(base, kd)
    raw = os.path.join(key_dir, "audio_full_event.raw")
    if not os.path.exists(raw):
        raw = os.path.join(key_dir, "audio.raw")
    if not os.path.exists(raw):
        return (kd, "NO_AUDIO", None)

    # Track regenerable big files we create, so --tidy only deletes our own.
    tidy_targets = [os.path.join(key_dir, n) for n in ("spectrum.csv", "goertzel.csv")]
    preexisting = {p for p in tidy_targets if os.path.exists(p)}

    try:
        subprocess.run([GATE, raw], stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL, check=True)
        cmd = [ENGINE, raw] + (["--refine"] if refine else [])
        subprocess.run(cmd, stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL, check=True)
    except subprocess.CalledProcessError as e:
        return (kd, f"TOOL_ERROR:{e.returncode}", None)

    seq = stable_winner_seq(key_dir)
    if tidy:
        for p in tidy_targets:
            if p not in preexisting and os.path.exists(p):
                os.remove(p)
    if seq is None:
        return (kd, "NO_CSV", None)
    return (kd, "OK", seq)


def cmd_cache(args):
    for ex, feats in [(ENGINE, ["--features", "telemetry"]), (GATE, [])]:
        tgt = ex.split("/")[-1]
        subprocess.run(["cargo", "build", "--release", "--example", tgt] + feats,
                       check=True, stdout=subprocess.DEVNULL)

    keys = sorted(d for d in os.listdir(args.base) if d.startswith("key_"))
    print(f"cache | base={args.base} refine={args.refine} | {len(keys)} capture dirs")
    os.makedirs(os.path.dirname(args.out) or ".", exist_ok=True)

    rows, errors = [], []
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futs = {pool.submit(cache_one, args.base, kd, args.refine, args.tidy): kd
                for kd in keys}
        # Gather in deterministic (sorted) order regardless of completion order.
        results, done = {}, 0
        for fut, kd in futs.items():
            results[kd] = fut.result()
            done += 1
            if done % 50 == 0:
                print(f"  ... {done}/{len(keys)}")
    for kd in keys:
        name, status, seq = results[kd]
        if status != "OK":
            errors.append(f"{name} -> {status}")
            continue
        expected = int(kd.split("_")[1])
        rows.append({"dir": name, "expected": expected,
                     "n_stable": len(seq),
                     "winners": ";".join(str(w) for w in seq)})

    with open(args.out, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=["dir", "expected", "n_stable", "winners"])
        w.writeheader()
        w.writerows(rows)
    print(f"wrote {len(rows)} rows -> {args.out}")
    if errors:
        print("ERRORS:")
        for e in errors:
            print("  " + e)


# ---------------------------------------------------------------- rule replay

def eval_rule(seq, m, n):
    """First key to win >= m of the last n stable frames (window may be
    partially filled at the start, so clean evidence locks early; m = n = 3
    is exactly the production 3-consecutive rule). Returns (key, stable_idx)
    or (None, -1) if the rule never fires. m > n/2 guarantees at most one
    key can hold >= m votes, so only the just-appended key need be checked."""
    win = deque(maxlen=n)
    counts = Counter()
    for t, w in enumerate(seq):
        if len(win) == n:
            counts[win[0]] -= 1
        win.append(w)
        counts[w] += 1
        if counts[w] >= m:
            return w, t
    return None, -1


def load_cache(path):
    with open(path) as f:
        caps = []
        for r in csv.DictReader(f):
            seq = [int(x) for x in r["winners"].split(";")] if r["winners"] else []
            caps.append((r["dir"], int(r["expected"]), seq))
    return caps


def surface_point(caps, m, n, baseline):
    """Metrics for one (m, n). baseline: dir -> (key, idx) from m=n=3."""
    npass = nolock = 0
    reg = [[0, 0], [0, 0], [0, 0]]
    fixed, broken = [], []
    added_lat = []
    for d, expected, seq in caps:
        key, idx = eval_rule(seq, m, n)
        ok = key == expected
        ri = register(expected)
        reg[ri][0] += 1
        reg[ri][1] += int(ok)
        npass += int(ok)
        if key is None:
            nolock += 1
        bkey, bidx = baseline[d]
        bok = bkey == expected
        if ok and not bok:
            fixed.append(d)
        elif bok and not ok:
            broken.append(d)
        if ok and bok:
            added_lat.append(idx - bidx)
    added_lat.sort()
    med = added_lat[len(added_lat) // 2] if added_lat else 0
    p95 = added_lat[int(len(added_lat) * 0.95)] if added_lat else 0
    mx = added_lat[-1] if added_lat else 0
    return {"m": m, "n": n, "pass": npass,
            "bass": reg[0][1], "mid": reg[1][1], "treble": reg[2][1],
            "nolock": nolock, "fixed": fixed, "broken": broken,
            "lat_med": med, "lat_p95": p95, "lat_max": mx}


def all_pairs(n_max):
    for n in range(3, n_max + 1):
        for m in range(n // 2 + 1, n + 1):
            yield m, n


def compute_surface(caps, n_max):
    baseline = {d: eval_rule(seq, 3, 3) for d, _, seq in caps}
    return {(m, n): surface_point(caps, m, n, baseline)
            for m, n in all_pairs(n_max)}, baseline


def plurality_ceiling(caps):
    """Full-window plurality vote — the Prompt A' 'gross ceiling' cross-check.
    Returns (passes counting ties as fail, passes counting ties as pass)."""
    strict = loose = 0
    for _, expected, seq in caps:
        if not seq:
            continue
        c = Counter(seq)
        top = c.most_common()
        best = top[0][1]
        winners = [k for k, v in top if v == best]
        if winners == [expected]:
            strict += 1
            loose += 1
        elif expected in winners:
            loose += 1
    return strict, loose


def cmd_sweep(args):
    caps = load_cache(args.cache)
    total = len(caps)
    surface, baseline = compute_surface(caps, args.n_max)
    b = surface[(3, 3)]
    print(f"sweep | {args.cache} | {total} captures | N in 3..{args.n_max}, M > N/2")
    print(f"BASELINE (M=N=3): {b['pass']}/{total}  "
          f"bass {b['bass']} mid {b['mid']} treble {b['treble']}")
    bl_fails = sorted(d for d, expected, seq in caps
                      if baseline[d][0] != expected)
    print(f"  baseline failures ({len(bl_fails)}):")
    for d in bl_fails:
        print(f"    {d} -> locked {baseline[d][0]}")
    s, l = plurality_ceiling(caps)
    print(f"PLURALITY CEILING (full window): {s}/{total} (ties=fail), "
          f"{l}/{total} (ties=pass)")

    print(f"{'N':>3} {'bestM':>5} {'pass':>5} {'bass':>4} {'mid':>4} {'tre':>4} "
          f"{'nolock':>6} {'+fix':>4} {'-brk':>4} {'latMed':>6} {'latP95':>6}")
    for n in range(3, args.n_max + 1):
        best = max((surface[(m, n)] for m in range(n // 2 + 1, n + 1)),
                   key=lambda p: (p["pass"], -p["m"]))
        print(f"{n:>3} {best['m']:>5} {best['pass']:>5} {best['bass']:>4} "
              f"{best['mid']:>4} {best['treble']:>4} {best['nolock']:>6} "
              f"{len(best['fixed']):>4} {len(best['broken']):>4} "
              f"{best['lat_med']:>6} {best['lat_p95']:>6}")

    if args.surface_out:
        with open(args.surface_out, "w", newline="") as f:
            w = csv.writer(f)
            w.writerow(["m", "n", "pass", "bass", "mid", "treble", "nolock",
                        "n_fixed", "n_broken", "lat_med", "lat_p95", "lat_max",
                        "fixed", "broken"])
            for (m, n), p in sorted(surface.items(), key=lambda kv: (kv[0][1], kv[0][0])):
                w.writerow([m, n, p["pass"], p["bass"], p["mid"], p["treble"],
                            p["nolock"], len(p["fixed"]), len(p["broken"]),
                            p["lat_med"], p["lat_p95"], p["lat_max"],
                            ";".join(p["fixed"]), ";".join(p["broken"])])
        print(f"full surface -> {args.surface_out}")


# ------------------------------------------------------------- concordance

def per_key_consistency(caps, m, n):
    """P2 repeat aggregation: key -> (n_correct, n_dumps)."""
    agg = {}
    for _, expected, seq in caps:
        key, _ = eval_rule(seq, m, n)
        c, t = agg.get(expected, (0, 0))
        agg[expected] = (c + int(key == expected), t + 1)
    return agg


def cmd_concord(args):
    p1 = load_cache(args.p1)
    p2 = load_cache(args.p2)
    s1, _ = compute_surface(p1, args.n_budget)
    s2, _ = compute_surface(p2, args.n_budget)
    max1 = max(p["pass"] for p in s1.values())
    max2 = max(p["pass"] for p in s2.values())
    plat1 = {k for k, p in s1.items() if p["pass"] >= max1 - args.p1_tol}
    plat2 = {k for k, p in s2.items() if p["pass"] >= max2 - args.p2_tol}
    region = plat1 & plat2
    b1, b2 = s1[(3, 3)]["pass"], s2[(3, 3)]["pass"]
    print(f"concord | N <= {args.n_budget}, M > N/2")
    print(f"P1 {args.p1}: baseline {b1}/{len(p1)}, max {max1}, "
          f"plateau(tol {args.p1_tol}) {len(plat1)} pairs")
    print(f"P2 {args.p2}: baseline {b2}/{len(p2)}, max {max2}, "
          f"plateau(tol {args.p2_tol}) {len(plat2)} pairs")
    print(f"INTERSECTION: {len(region)} pairs")
    if not region:
        print("EMPTY — no concordant (M,N) at this budget (pre-registered: "
              "record and close/re-gate per the design note's outcome gates).")
        return
    rec = min(region, key=lambda k: (k[1], k[0]))
    m, n = rec
    print(f"RECOMMENDATION (min N, then min M): M={m}, N={n}")
    print("neighbor check (isolated-spike guard) [pair: P1pass/P2pass, "
          "*=in region]:")
    for dm, dn in ((0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)):
        mm, nn = m + dm, n + dn
        if (mm, nn) in s1 and mm > nn // 2 and mm <= nn:
            tag = "*" if (mm, nn) in region else " "
            print(f"  ({mm:>2},{nn:>2}){tag}: {s1[(mm, nn)]['pass']}/"
                  f"{s2[(mm, nn)]['pass']}")
    for name, s, caps in (("P1", s1, p1), ("P2", s2, p2)):
        p = s[rec]
        print(f"{name} @ (M={m},N={n}): {p['pass']}/{len(caps)}  "
              f"bass {p['bass']} mid {p['mid']} treble {p['treble']}  "
              f"nolock {p['nolock']}  added-latency med/p95/max "
              f"{p['lat_med']}/{p['lat_p95']}/{p['lat_max']} stable frames")
        print(f"  fixed  ({len(p['fixed'])}): {', '.join(p['fixed']) or '-'}")
        print(f"  broken ({len(p['broken'])}): {', '.join(p['broken']) or '-'}")
    agg = per_key_consistency(p2, m, n)
    base_agg = per_key_consistency(p2, 3, 3)
    incons = {k: v for k, v in agg.items() if 0 < v[0] < v[1]}
    moved = sorted(k for k in agg
                   if agg[k][0] / agg[k][1] != base_agg[k][0] / base_agg[k][1])
    print(f"P2 per-key: {len(incons)} keys with mixed dump outcomes at the "
          f"recommendation; {len(moved)} keys changed vs baseline:")
    for k in moved:
        c, t = agg[k]
        bc, bt = base_agg[k]
        print(f"  key {k:>2}: baseline {bc}/{bt} -> {c}/{t}")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("cache")
    c.add_argument("--base", required=True)
    c.add_argument("--out", required=True)
    c.add_argument("--refine", action="store_true")
    c.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 2) - 1))
    c.add_argument("--tidy", action="store_true",
                   help="delete spectrum.csv/goertzel.csv this run created")
    c.set_defaults(fn=cmd_cache)

    s = sub.add_parser("sweep")
    s.add_argument("cache")
    s.add_argument("--n-max", type=int, default=43)
    s.add_argument("--surface-out", default=None)
    s.set_defaults(fn=cmd_sweep)

    k = sub.add_parser("concord")
    k.add_argument("--p1", required=True)
    k.add_argument("--p2", required=True)
    k.add_argument("--n-budget", type=int, default=21)
    k.add_argument("--p1-tol", type=int, default=1)
    k.add_argument("--p2-tol", type=int, default=7)
    k.set_defaults(fn=cmd_concord)

    args = ap.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
