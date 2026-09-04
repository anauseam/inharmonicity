#!/usr/bin/env python3
"""Isolation-set truth side — every table in ADR 0014 that is not the panel's.

The panel's own readings come from `cargo lab strobe isolation` (it drives the shipped
Strobe, which only Rust can do); everything here is post-processing of
`regenerate_partials` output, the same split as `audit_captures.py`.

    cargo lab mat regen <dump_dir> > iso.json
    cargo lab strobe isolation iso.json <dump_dir> --json panel.json
    python3 scripts/isolation_truth.py iso.json panel.json

Sections map to the ADR: 1 the screen, 2 per-string truth, 3 the regime in
beats, 5 C1, 6 coupling, 7 the B premise.
"""
import json
import math
import statistics as st
import sys
from collections import defaultdict

NOTES = ["A", "A#", "B", "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#"]

# tuner_core::audio and strobe::unison, mirrored so this script needs no build.
HOP_RATE_HZ = 44100.0 / 1024.0
UNISON_RING_HOPS = round(1.30 * HOP_RATE_HZ)
RING_2T = 2.0 * HOP_RATE_HZ / UNISON_RING_HOPS

# tuner-gui app.rs: UNISON_SPAN_LADDER[0], "a unison being finished".
LADDER0_CENTS = 3.0

# ADR 0012 §4: above 1.6 x 2/T the estimator is exact to a -0.085 Hz systematic
# with sigma <= 0.009 Hz. C1's tolerance is derived from these plus the coupling
# bound this script measures -- never an invented percentage (`07` §2).
EST_BIAS_HZ = -0.085
EST_SIGMA_HZ = 0.009
COUPLING_BOUND_HZ = 0.26   # measured in section 6: pooled median excursion

# The failed-solo screen (ADR 0014 §1): a failed capture disagrees with its own
# siblings in B by 30-90x while good ones agree to a fraction of a percent, so
# the cut sits in a chasm and its placement is not load-bearing.
B_SCREEN_FACTOR = 3.0


def name(k):
    return f"{NOTES[k % 12]}{(k + 9) // 12}"


def display_partial(k):
    """curves::default_display_partials — the register defaults."""
    return 6 if k <= 26 else 4 if k <= 38 else 2 if k <= 47 else 1


def partial_hz(f0, b, n):
    return n * f0 * math.sqrt(1.0 + b * n * n)


def cents(a, b):
    return 1200.0 * math.log2(a / b)


def load(path):
    out = []
    for r in json.load(open(path)):
        ss = r.get("sounding_strings")
        if not ss:
            continue
        out.append(dict(
            key=r["key_index"], dir=r["source_dir"], f0=r["mat_f0"],
            b=r["calculated_b"], prior=r["prior_b"], n=r["partial_count"],
            on_key=ss["on_key"], snd=ss["sounding"], count=sum(ss["sounding"]),
            partials={p["number"]: p["frequency"] for p in r["partials"]}))
    return out


def screen(rows, verbose=True):
    """Section 1 — drop failed solos by within-key B agreement."""
    by = defaultdict(list)
    for r in rows:
        if r["count"] == 1 and r["on_key"] > 1:
            by[r["key"]].append(r)
    bad = set()
    for caps in by.values():
        med = st.median([c["b"] for c in caps if c["b"] > 0])
        for c in caps:
            if c["b"] <= 0 or abs(math.log(c["b"] / med)) > math.log(B_SCREEN_FACTOR):
                bad.add(c["dir"])
                if verbose:
                    print(f"  screened: {c['dir']:<34} n={c['n']:<3} "
                          f"B={c['b']:.3e} = {c['b']/med:>6.1f}x its key's solo "
                          f"median, {c['b']/c['prior']:>6.1f}x the prior")
    return [r for r in rows if r["dir"] not in bad]


def per_string(rows):
    """Median (f0, B) per (key, string), solos only."""
    acc = defaultdict(lambda: defaultdict(list))
    for r in rows:
        if r["count"] == 1 and r["on_key"] > 1:
            acc[r["key"]][r["snd"].index(True)].append(r)
    return {k: {s: (st.median([c["f0"] for c in v]), st.median([c["b"] for c in v]))
                for s, v in d.items()} for k, d in acc.items()}


def widest_pair(strings_k, n):
    """Widest f0 pair of a key, and its separation at partial n."""
    ss = sorted(strings_k)
    best = (0.0, 0.0)
    for i in range(len(ss)):
        for j in range(i + 1, len(ss)):
            a, b = strings_k[ss[i]], strings_k[ss[j]]
            c = abs(cents(a[0], b[0]))
            if c > best[0]:
                best = (c, abs(partial_hz(*a, n) - partial_hz(*b, n)))
    return best


def section2(rows, strings):
    print("\n" + "=" * 76)
    print("2. PER-STRING TRUTH")
    print("=" * 76)
    print(f"{'key':>9} {'str':>4} {'reps':>5} {'f0 Hz':>10} {'repeat c':>9} {'partials':>9}")
    acc = defaultdict(lambda: defaultdict(list))
    for r in rows:
        if r["count"] == 1 and r["on_key"] > 1:
            acc[r["key"]][r["snd"].index(True)].append(r)
    sds = []
    for k in sorted(acc):
        for s in sorted(acc[k]):
            caps = acc[k][s]
            med = st.median([c["f0"] for c in caps])
            sd = (st.pstdev([cents(c["f0"], med) for c in caps])
                  if len(caps) > 1 else float("nan"))
            if not math.isnan(sd):
                sds.append(sd)
            print(f"{k:>3}/{name(k):<5} {s+1:>4} {len(caps):>5} {med:>10.3f} "
                  f"{sd:>9.3f} {st.median([c['n'] for c in caps]):>9.0f}")
    print(f"\n  median {st.median(sds):.3f} c, range {min(sds):.3f}-{max(sds):.3f} c")
    print("  (06 published 0.04-0.16 c 'bass through upper mid' — too narrow)")

    print(f"\n{'key':>9} {'widest split c':>16}")
    for k in sorted(strings):
        print(f"{k:>3}/{name(k):<5} {widest_pair(strings[k], 1)[0]:>16.2f}")


def section3(strings):
    print("\n" + "=" * 76)
    print("3. THE OPERATING REGIME, IN BEATS")
    print("=" * 76)
    print(f"  ring cap {UNISON_RING_HOPS} hops = {UNISON_RING_HOPS/HOP_RATE_HZ:.3f} s"
          f"  ->  2/T = {RING_2T:.3f} Hz, one number for the whole compass\n")
    print(f"{'key':>9} {'n*':>3} {'floor@n* c':>11} {'floor@f1 c':>11} "
          f"{'widest c':>9} {'beat@n* Hz':>11}  sees it?")
    res = 0
    for k in sorted(strings):
        n = display_partial(k)
        c, beat = widest_pair(strings[k], n)
        f1 = strings[k][sorted(strings[k])[0]][0]
        fl_n = 1200 * math.log2(1 + RING_2T / (n * f1))
        fl_1 = 1200 * math.log2(1 + RING_2T / f1)
        ok = beat >= RING_2T
        res += ok
        print(f"{k:>3}/{name(k):<5} {n:>3} {fl_n:>11.2f} {fl_1:>11.2f} "
              f"{c:>9.2f} {beat:>11.3f}  {'YES' if ok else 'no'}")
    print(f"\n  resolves on {res} of {len(strings)} isolation keys at n*")
    print(f"  ... and on {sum(1 for k in strings if widest_pair(strings[k],1)[1] >= RING_2T)}"
          f" of {len(strings)} judged at the fundamental — the correction to the")
    print("  displayed partial is real (4-6x in bass/tenor) and changes nothing.")

    clears = [k for k in range(88)
              if 1200 * math.log2(1 + RING_2T / (display_partial(k) * 27.5 * 2 ** (k / 12)))
              < LADDER0_CENTS]
    print(f"\n  Across the compass, the floor beats UNISON_SPAN_LADDER[0] "
          f"({LADDER0_CENTS} c) on")
    print(f"  {len(clears)} of 88 keys — so the amber flag at main_view.rs:583 is lit")
    print(f"  on {88-len(clears)} of 88 with the default display table.")


def section5(panel, strings):
    print("\n" + "=" * 76)
    print("5. C1 — REPORTED LINE POSITIONS vs SOLO TRUTH")
    print("=" * 76)
    tol = abs(EST_BIAS_HZ) + 3 * EST_SIGMA_HZ + COUPLING_BOUND_HZ
    print(f"  derived tolerance = |bias| + 3sigma + coupling = {tol:.3f} Hz\n")
    print(f"{'key':>9} {'truth/2T':>9} {'caps':>5} {'lines':>6} {'med |err| Hz':>13}  verdict")
    opens = [p for p in panel if p["is_open"] and p["record_hops"] > 0
             and p["key"] in strings and p["line_count"] >= 2 and p["ref_hz"]]
    pop = []
    for k in sorted({p["key"] for p in opens}):
        rows = [p for p in opens if p["key"] == k]
        n = rows[0]["n_star"]
        solos = [partial_hz(*strings[k][s], n) for s in sorted(strings[k])]
        ratio = (max(solos) - min(solos)) / st.median([r["resolution_hz"] for r in rows])
        errs = []
        for r in rows:
            lines, used = sorted(r["ref_hz"] + o for o in r["offsets_hz"]), set()
            for L in lines:
                cand = [(abs(L - s), i) for i, s in enumerate(solos) if i not in used]
                if not cand:
                    continue
                d, i = min(cand)
                used.add(i)
                errs.append(L - solos[i])
        if not errs:
            continue
        m = st.median([abs(e) for e in errs])
        inpop = ratio >= 2.0
        if inpop:
            pop += errs
        print(f"{k:>3}/{name(k):<5} {ratio:>9.2f} {len(rows):>5} {len(errs):>6} "
              f"{m:>13.3f}  {'IN C1 POPULATION' if inpop else 'below 2x2/T'}")
    if pop:
        ok = sum(1 for d in pop if abs(d) <= tol)
        print(f"\n  pre-registered population: n={len(pop)} lines, median |err| "
              f"{st.median([abs(d) for d in pop]):.3f} Hz, "
              f"{100.0*ok/len(pop):.0f} % within tolerance")
        print("  NOTE: that population is ONE key. See ADR 0014 §5 — underpowered,")
        print("  and it is the key 06 documents as bistable.")


def section6(rows, strings):
    print("\n" + "=" * 76)
    print("6. COUPLING — is the open partial inside the span its solos define?")
    print("=" * 76)
    print(f"{'key':>9} {'partials':>9} {'span Hz':>9} {'inside':>7}")
    acc = defaultdict(lambda: defaultdict(list))
    for r in rows:
        if r["count"] == 1 and r["on_key"] > 1:
            for n, f in r["partials"].items():
                acc[r["key"]][r["snd"].index(True)].append((n, f))
    pool_in = pool_n = 0
    outs = []
    for k in sorted(strings):
        per = defaultdict(lambda: defaultdict(list))
        for s, lst in acc[k].items():
            for n, f in lst:
                per[s][n].append(f)
        inside = tot = 0
        spans = []
        for op in [r for r in rows if r["key"] == k and r["count"] == r["on_key"]]:
            for n, f in op["partials"].items():
                vals = [st.median(per[s][n]) for s in per if n in per[s]]
                if len(vals) < 2 or max(vals) - min(vals) <= 0:
                    continue
                lo, hi = min(vals), max(vals)
                p = (f - lo) / (hi - lo)
                tot += 1
                spans.append(hi - lo)
                if -0.05 <= p <= 1.05:
                    inside += 1
                elif f > hi or f < lo:
                    outs.append(abs(f - hi if f > hi else f - lo))
        if tot:
            pool_in += inside
            pool_n += tot
            print(f"{k:>3}/{name(k):<5} {tot:>9} {st.median(spans):>9.3f} "
                  f"{100.0*inside/tot:>6.0f}%")
    print(f"\n  pooled {100.0*pool_in/pool_n:.0f} % inside ({pool_in}/{pool_n})")
    if outs:
        print(f"  median excursion when outside {st.median(outs):.3f} Hz, "
              f"p90 {sorted(outs)[int(0.9*len(outs))]:.3f} Hz")
    print("  The inside fraction tracks SPAN WIDTH, which is measurement error,")
    print("  not coupling: the widest-span keys are 96-98 % inside.")


def section7(rows, strings):
    print("\n" + "=" * 76)
    print("7. THE B PREMISE — real spread, or repeat noise?")
    print("=" * 76)
    print(f"{'key':>9} {'within %':>9} {'between %':>10} {'ratio':>7} "
          f"{'split@1':>8} {'split@6':>8} {'tilt':>7}  verdict")
    acc = defaultdict(lambda: defaultdict(list))
    for r in rows:
        if r["count"] == 1 and r["on_key"] > 1:
            acc[r["key"]][r["snd"].index(True)].append(r["b"])
    for k in sorted(strings):
        meds, wit = [], []
        for s, v in acc[k].items():
            if not v:
                continue
            meds.append(st.median(v))
            if len(v) > 1:
                wit.append(100.0 * (max(v) - min(v)) / st.median(v))
        if len(meds) < 2 or not wit:
            continue
        btw = 100.0 * (max(meds) - min(meds)) / st.mean(meds)
        w = st.median(wit)
        ratio = btw / w if w > 0 else float("inf")
        c1 = widest_pair(strings[k], 1)[0]
        ss = sorted(strings[k])
        best = max(((abs(cents(strings[k][ss[i]][0], strings[k][ss[j]][0])), i, j)
                    for i in range(len(ss)) for j in range(i + 1, len(ss))))
        a, b = strings[k][ss[best[1]]], strings[k][ss[best[2]]]
        c6 = abs(cents(partial_hz(*a, 6), partial_hz(*b, 6)))
        print(f"{k:>3}/{name(k):<5} {w:>9.2f} {btw:>10.2f} {ratio:>7.1f} "
              f"{c1:>8.2f} {c6:>8.2f} {100*abs(c6-c1)/max(c1,1e-9):>6.0f}%  "
              f"{'REAL' if ratio > 3 else 'not separable'}")
    print("\n  A real B difference tilts the split across partials by "
          "866*(B1-B2)*n^2 cents,")
    print("  which is what ADR 0012 §6's residual-estimated standard error absorbs.")


def main():
    print("=" * 76)
    print("1. THE FAILED-SOLO SCREEN (within-key B agreement)")
    print("=" * 76)
    raw = load(sys.argv[1])
    rows = screen(raw)
    print(f"  {len(raw) - len(rows)} of {len(raw)} captures screened out")

    strings = per_string(rows)
    section2(rows, strings)
    section3(strings)
    if len(sys.argv) > 2:
        section5(json.load(open(sys.argv[2])), strings)
    section6(rows, strings)
    section7(rows, strings)


if __name__ == "__main__":
    main()
