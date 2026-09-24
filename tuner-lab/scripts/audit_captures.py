"""Audit the regenerated (seed-gated) repeat-capture set."""
import json
import math
import sys
from collections import defaultdict

entries = json.load(open(sys.argv[1]))
keys = defaultdict(list)
for e in entries:
    keys[e["key_index"]].append(e)

def median(v):
    s = sorted(v)
    n = len(s)
    return s[n // 2] if n % 2 else 0.5 * (s[n // 2 - 1] + s[n // 2])

ET = lambda k: 27.5 * 2 ** (k / 12)

print(f"{'key':>4} {'n':>3} {'med mat_f0':>10} {'vs ET ¢':>8} {'med B':>10} "
      f"{'B spr%':>7} {'f0 spr¢':>8}  flags")
suspects = []
for k in sorted(keys):
    caps = keys[k]
    f0s = [c["mat_f0"] for c in caps]
    bs = [c["calculated_b"] for c in caps if c["calculated_b"] > 0]
    for c in caps:
        if c["calculated_b"] <= 0:
            suspects.append((c["source_dir"], f"B={c['calculated_b']} (non-physical)"))
    if not bs:
        print(f"{k:>4} {len(caps):>3}  ALL captures have non-physical B")
        continue
    mf, mb = median(f0s), median(bs)
    cents_et = 1200 * math.log2(mf / ET(k))
    bspread = 100 * (max(bs) / min(bs) - 1)
    fspread = 1200 * math.log2(max(f0s) / min(f0s))
    flags = []
    for c in caps:
        why = []
        dc = abs(1200 * math.log2(c["mat_f0"] / mf))
        if dc > 20:
            why.append(f"f0 {dc:.0f}c off key median")
        if c["calculated_b"] > 0:
            r = abs(math.log(c["calculated_b"] / mb))
            if r > 0.7:
                why.append(f"B {c['calculated_b']:.2e} vs med {mb:.2e}")
        if why:
            suspects.append((c["source_dir"], "; ".join(why)))
            flags.append("suspect:" + c["source_dir"].split("_")[-1])
    print(f"{k:>4} {len(caps):>3} {mf:>10.2f} {cents_et:>+8.1f} {mb:>10.3e} "
          f"{bspread:>7.1f} {fspread:>8.2f}  {' '.join(flags)}")

print(f"\ntotal: {sum(len(v) for v in keys.values())} captures / {len(keys)} keys")
print("\nSuspect captures after seed gate:")
for d, w in suspects:
    print(f"  {d}: {w}")
if not suspects:
    print("  none")
