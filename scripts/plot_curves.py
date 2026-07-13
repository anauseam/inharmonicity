"""Render the tuning-curve comparison image for a capture set.

One PNG per capture set (the curve is a whole-instrument object), analogous to
the per-sample gatekeeper/goertzel images:

    cargo run --release --example regenerate_partials -- <diagnostics_dir> > partials.json
    cargo run --release --example curve_compare -- partials.json --json curve_report.json
    python scripts/plot_curves.py curve_report.json [out.png]

Default output: curve_analysis.png next to the report JSON. Top panel: all
engine curves d(m) in cents vs ET across the compass, with negative-stretch
flags and the measured-key rug. Bottom: per-register octave stretch, and the
full diagnostics table (roughness, cross-score, leave-key-out error). All
numbers are diagnostics, not selection evidence (design note S11; n = 1).
"""

import json
import os
import sys

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

# Categorical palette, fixed slot order (validated; dataviz reference set).
SERIES = ["#2a78d6", "#1baf7a", "#eda100", "#008300", "#4a3aa7"]
TEXT_PRIMARY = "#0b0b0b"
TEXT_SECONDARY = "#52514e"
GRID = "#e6e5e1"
SURFACE = "#fcfcfb"
FLAG_RED = "#e34948"  # status: negative-stretch detector

A_KEYS = list(range(0, 88, 12)) + [87]
A_LABELS = [f"A{k // 12}" if k != 87 else "C8" for k in A_KEYS]


def main():
    if len(sys.argv) < 2:
        print("Usage: python scripts/plot_curves.py <curve_report.json> [out.png]")
        sys.exit(1)
    report_path = sys.argv[1]
    out_path = (
        sys.argv[2]
        if len(sys.argv) > 2
        else os.path.join(os.path.dirname(os.path.abspath(report_path)), "curve_analysis.png")
    )
    with open(report_path) as f:
        rep = json.load(f)

    engines = rep["engines"]
    keys = np.arange(88)

    fig = plt.figure(figsize=(14, 9.5), facecolor=SURFACE)
    gs = fig.add_gridspec(
        2, 2, height_ratios=[2.1, 1.0], width_ratios=[1.0, 1.35],
        hspace=0.28, wspace=0.18, left=0.06, right=0.97, top=0.90, bottom=0.07,
    )

    # ── Top: the curves ──
    ax = fig.add_subplot(gs[0, :])
    ax.set_facecolor(SURFACE)
    for spine in ("top", "right"):
        ax.spines[spine].set_visible(False)
    for spine in ("left", "bottom"):
        ax.spines[spine].set_color(GRID)
    ax.grid(True, color=GRID, linewidth=0.8, zorder=0)
    ax.axhline(0.0, color=TEXT_SECONDARY, linewidth=0.8, zorder=1)

    for i, eng in enumerate(engines):
        color = SERIES[i % len(SERIES)]
        cents = np.asarray(eng["cents"], dtype=float)
        ax.plot(keys, cents, color=color, linewidth=2, zorder=3, label=eng["name"])
        # Direct label at the LEFT end — the curves converge at C8 but are
        # maximally separated at A0 (relief rule for low-contrast hues).
        ax.annotate(
            eng["name"].split(":")[0],
            xy=(0, cents[0]),
            xytext=(-6, 0),
            textcoords="offset points",
            color=color,
            fontsize=9,
            fontweight="bold",
            va="center",
            ha="right",
        )
        # Negative-stretch flags: status marker, not color-alone (x shape).
        flags = eng["flags"]["negative_stretch"]
        if flags:
            ax.plot(
                flags, cents[flags], "x", color=FLAG_RED, markersize=8,
                markeredgewidth=2, zorder=4,
            )

    # Measured-key rug (from the first engine; identical across engines).
    measured = engines[0]["flags"]["measured"]
    y0, y1 = ax.get_ylim()
    rug_y = y0 + 0.015 * (y1 - y0)
    ax.plot(
        measured, [rug_y] * len(measured), "|", color=TEXT_SECONDARY,
        markersize=6, zorder=2,
    )

    ax.set_xticks(A_KEYS)
    ax.set_xticklabels(A_LABELS, color=TEXT_PRIMARY)
    ax.set_xlim(-4, 90)
    ax.tick_params(colors=TEXT_SECONDARY)
    ax.set_ylabel("deviation from ET (cents, audible f₁)", color=TEXT_PRIMARY)
    handles, labels = ax.get_legend_handles_labels()
    if any(e["flags"]["negative_stretch"] for e in engines):
        from matplotlib.lines import Line2D

        handles.append(
            Line2D([], [], marker="x", color=FLAG_RED, linestyle="none",
                   markersize=8, markeredgewidth=2)
        )
        labels.append("negative-stretch flag")
    ax.legend(
        handles, labels, loc="upper left", frameon=False, fontsize=9,
        labelcolor=TEXT_PRIMARY, ncol=2,
    )

    # ── Bottom left: per-register median octave stretch ──
    ax2 = fig.add_subplot(gs[1, 0])
    ax2.set_facecolor(SURFACE)
    for spine in ("top", "right"):
        ax2.spines[spine].set_visible(False)
    for spine in ("left", "bottom"):
        ax2.spines[spine].set_color(GRID)
    ax2.grid(True, axis="y", color=GRID, linewidth=0.8, zorder=0)
    regs = ["bass", "mid", "treble"]
    n_eng = len(engines)
    width = 0.8 / n_eng
    for i, eng in enumerate(engines):
        vals = [eng["stretch_median"][r] for r in regs]
        x = np.arange(3) + (i - (n_eng - 1) / 2) * width
        ax2.bar(
            x, vals, width * 0.92, color=SERIES[i % len(SERIES)], zorder=3,
            edgecolor=SURFACE, linewidth=1,
        )
    ax2.set_xticks(np.arange(3))
    ax2.set_xticklabels(regs, color=TEXT_PRIMARY)
    ax2.tick_params(colors=TEXT_SECONDARY)
    ax2.set_ylabel("median octave stretch (¢/oct)", color=TEXT_PRIMARY)

    # ── Bottom right: diagnostics table ──
    ax3 = fig.add_subplot(gs[1, 1])
    ax3.axis("off")
    cols = ["engine", "stretch b/m/t", "rough med/max", "cross", "LKO b/m"]
    rows = []
    for eng in engines:
        s = eng["stretch_median"]
        r = eng["roughness"]
        lko = eng.get("lko_median") or {}
        rows.append([
            eng["name"],
            f"{s['bass']:.1f} / {s['mid']:.1f} / {s['treble']:.1f}",
            f"{r['median']:.3f} / {r['max']:.2f}",
            f"{eng['cross_score']:.4f}" if eng.get("cross_score") is not None else "-",
            f"{lko.get('bass', float('nan')):.2f} / {lko.get('mid', float('nan')):.2f}"
            if lko else "-",
        ])
    table = ax3.table(
        cellText=rows, colLabels=cols, loc="center", cellLoc="left", colLoc="left",
        colWidths=[0.30, 0.22, 0.20, 0.12, 0.16],
    )
    table.auto_set_font_size(False)
    table.set_fontsize(8.5)
    table.scale(1.0, 1.5)
    for (row, _col), cell in table.get_celld().items():
        cell.set_edgecolor(GRID)
        cell.get_text().set_color(TEXT_PRIMARY if row > 0 else TEXT_SECONDARY)
        cell.set_facecolor(SURFACE)

    # ── Title + provenance footer ──
    cal = rep["calibration"]
    phi = cal.get("phi")
    phi_txt = (
        f"φ = (κ {phi['kappa']:.2f}, m0 {phi['m0']:.0f}, α {phi['alpha']:.0f})"
        if phi
        else "φ = none (degraded to (b))"
    )
    fig.suptitle(
        f"Tuning-curve engines — {rep['measured_keys']}/88 keys measured",
        color=TEXT_PRIMARY, fontsize=14, fontweight="bold", x=0.06, ha="left",
    )
    fig.text(
        0.06, 0.925,
        f"B_ξ fit: s_B {rep['b_xi']['s_b']:+.4f}, y_B {rep['b_xi']['y_b']:+.3f}   ·   "
        f"Giordano: {len(cal['rho_points'])} ρ points, "
        f"gate {cal['gate_pass']}/{cal['pairs_both_measured']}, "
        f"reg {cal['reg_weight']:g}, {phi_txt}",
        color=TEXT_SECONDARY, fontsize=9,
    )
    fig.text(
        0.06, 0.015,
        "Diagnostics, not selection evidence (n = 1; design note §11). "
        f"Source: {os.path.basename(rep['source'])}",
        color=TEXT_SECONDARY, fontsize=8,
    )

    fig.savefig(out_path, dpi=130, facecolor=SURFACE)
    print(f"wrote {out_path}")


if __name__ == "__main__":
    main()
