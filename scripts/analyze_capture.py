#!/usr/bin/env python3
"""
analyze_capture.py — Inharmonicity Pipeline Capture Diagnostic

Analyzes audio captures produced by the Worker's diagnostic dump.
Supports both text-only batch analysis and interactive plotting.

Usage (Text/Agent Mode):
    python3 scripts/analyze_capture.py diagnostics/

Usage (GUI/Human Mode):
    python3 scripts/analyze_capture.py diagnostics/ --gui
"""

import struct
import json
import os
import sys
import argparse
import math
import numpy as np

SAMPLE_RATE = 44100
HOP_SIZE = 2048  # One Gatekeeper frame (~46ms)

# --- Analysis Logic ---


def load_payload(capture_dir):
    raw_path = os.path.join(capture_dir, "audio.raw")
    json_path = os.path.join(capture_dir, "analysis.json")

    if not os.path.exists(raw_path):
        return None, None

    # Load Audio
    with open(raw_path, "rb") as f:
        data = f.read()
        samples = np.frombuffer(data, dtype=np.float32)

    # Load Metadata
    meta = {}
    if os.path.exists(json_path):
        with open(json_path) as f:
            doc = json.load(f)
            meta = doc.get("metadata", {})

    return samples, meta


def harmonic_concentration_ratio(frame, f0, sample_rate, n_harmonics=16, bin_radius=2):
    """Fraction of total spectral power that lands on predicted harmonic positions.

    Given a known f0, computes expected positions for n=1..n_harmonics and sums
    the power within ±bin_radius FFT bins of each. Divides by total power.

    Returns a value in [0, 1]:
        ~1.0 — nearly all energy is in harmonic bins  (very clean)
        ~0.0 — energy spread uniformly across spectrum (pure noise)

    This is the right quality metric for piano captures because it uses the
    *known* pitch from analysis.json rather than measuring generic sparsity.
    It correctly distinguishes:
        - Tightly-packed bass harmonics that happen to look sparse (C1, C2) from
        - Genuinely clean captures where harmonics dominate the energy budget (C3+)
    """
    N = len(frame)
    power = np.abs(np.fft.rfft(frame)) ** 2
    total_power = np.sum(power)
    if total_power < 1e-24 or f0 <= 0:
        return 0.0

    hz_per_bin = sample_rate / N
    harmonic_mask = np.zeros_like(power, dtype=bool)

    for n in range(1, n_harmonics + 1):
        center_hz = f0 * n
        if center_hz >= sample_rate / 2:
            break
        center_bin = int(round(center_hz / hz_per_bin))
        lo = max(0, center_bin - bin_radius)
        hi = min(len(power) - 1, center_bin + bin_radius) + 1
        harmonic_mask[lo:hi] = True

    harmonic_power = np.sum(power[harmonic_mask])
    return float(min(harmonic_power / total_power, 1.0))


def calculate_metrics(samples, f0=0.0):
    n = len(samples)
    if n == 0:
        return {}

    full_rms = np.sqrt(np.mean(samples**2))
    peak = np.max(np.abs(samples))

    early_chunk = samples[:HOP_SIZE]
    early_rms = np.sqrt(np.mean(early_chunk**2)) if len(early_chunk) > 0 else 0

    mid_start = n // 2
    late_chunk = samples[mid_start : mid_start + HOP_SIZE]
    late_rms = np.sqrt(np.mean(late_chunk**2)) if len(late_chunk) > 0 else 1e-10

    # Informational only: how fast does the note's amplitude decay?
    # High value = fast-decaying treble note. Not a quality indicator.
    decay_rate = early_rms / max(late_rms, 1e-10)

    # Harmonic Concentration Ratio — the real quality metric.
    # Measures what fraction of total power is at predicted harmonic positions
    # using the known f0 from the Worker's analysis. This correctly rates:
    #   - Bass notes with noise between sparse harmonics: LOW score
    #   - Clean mid/treble captures with dominant harmonics:  HIGH score
    # Computed over three windows to show how quality evolves over the capture.
    hcr_early = (
        harmonic_concentration_ratio(early_chunk, f0, SAMPLE_RATE) if f0 > 0 else 0.0
    )
    hcr_mid = (
        harmonic_concentration_ratio(late_chunk, f0, SAMPLE_RATE) if f0 > 0 else 0.0
    )
    # Late window: last hop
    late2_chunk = samples[max(0, n - HOP_SIZE) :]
    hcr_late = (
        harmonic_concentration_ratio(late2_chunk, f0, SAMPLE_RATE) if f0 > 0 else 0.0
    )

    # Per-hop HCR profile for the GUI plot
    num_hops = n // HOP_SIZE
    hop_rms = []
    hop_hcr = []
    for i in range(num_hops):
        chunk = samples[i * HOP_SIZE : (i + 1) * HOP_SIZE]
        hop_rms.append(np.sqrt(np.mean(chunk**2)) if len(chunk) > 0 else 0)
        hop_hcr.append(
            harmonic_concentration_ratio(chunk, f0, SAMPLE_RATE) if f0 > 0 else 0.0
        )

    return {
        "rms": full_rms,
        "peak": peak,
        "early_rms": early_rms,
        "late_rms": late_rms,
        "decay_rate": decay_rate,
        "hcr_early": hcr_early,
        "hcr_mid": hcr_mid,
        "hcr_late": hcr_late,
        "hop_rms": hop_rms,
        "hop_hcr": hop_hcr,
        "duration": n / SAMPLE_RATE,
    }


# --- Text Output Mode ---


def print_text_analysis(capture_dir):
    samples, meta = load_payload(capture_dir)
    if samples is None:
        print(f"  [SKIP] No data in {capture_dir}")
        return

    f0 = meta.get("measured_f0", 0.0) or 0.0
    m = calculate_metrics(samples, f0=f0)
    label = os.path.basename(capture_dir)
    key_index = meta.get("key_index", "?")

    # HCR thresholds: tuned empirically.
    hcr_val = m["hcr_early"]
    hcr_flag = (
        "✅ Good" if hcr_val >= 0.35 else ("⚠️  Weak" if hcr_val >= 0.20 else "❌ Poor")
    )

    # Detect Issues
    issues = []
    # 1. Identification check (Ground truth vs engine)
    expected_f0 = meta.get("expected_f0", 0.0)
    if expected_f0 > 0 and f0 > 0:
        cents_off = abs(1200 * math.log2(f0 / expected_f0))
        if cents_off > 100:  # Engine picked the wrong note or an octave
            issues.append(
                f"❌ MISIDENTIFIED: {f0:.1f}Hz is {cents_off:+.0f}c away from ideal {expected_f0:.1f}Hz"
            )

    # 2. Spurious duration check
    if m["duration"] < 0.5:
        issues.append(f"❌ SPURIOUS: Capture is only {m['duration']:.3f}s (too short)")

    # 3. Overall quality
    if hcr_val < 0.20:
        issues.append(f"❌ NOISY: Harmonic concentration is too low ({hcr_val:.2f})")

    print(f"{'=' * 60}")
    print(f"  {label}  (key {key_index})")
    print(f"{'=' * 60}")
    print(f"  Measured f0:    {f0:.3f} Hz")
    print(
        f"  Peak:           {m['peak']:.4f}  |  Decay rate: {m['decay_rate']:.1f}x (informational)"
    )
    print(
        f"  HCR:            early={m['hcr_early']:.3f}  mid={m['hcr_mid']:.3f}  late={m['hcr_late']:.3f}  → {hcr_flag}"
    )

    if issues:
        print("  ⚠️  ATTENTION:")
        for issue in issues:
            print(f"    {issue}")

    print()


# --- GUI Mode ---


def run_gui(target_path):
    import matplotlib.pyplot as plt
    from matplotlib.widgets import Button

    class CaptureAnalyzer:
        def __init__(self, directories):
            self.directories = directories
            self.current_idx = 0
            # Increase figsize to accommodate three plots
            self.fig, (self.ax_wave, self.ax_rms, self.ax_spec) = plt.subplots(
                3, 1, figsize=(12, 10)
            )
            plt.subplots_adjust(bottom=0.1, hspace=0.4)

            # Create a PERSISTENT twin axis for HCR
            self.ax_hcr = self.ax_rms.twinx()

            ax_prev = plt.axes([0.7, 0.02, 0.1, 0.04])
            ax_next = plt.axes([0.81, 0.02, 0.1, 0.04])
            self.btn_prev = Button(ax_prev, "Prev")
            self.btn_next = Button(ax_next, "Next")
            self.btn_prev.on_clicked(self.prev)
            self.btn_next.on_clicked(self.next)
            self.fig.canvas.mpl_connect("key_press_event", self.on_key)
            self.update_plot()

        def update_plot(self):
            capture_dir = self.directories[self.current_idx]
            samples, meta = load_payload(capture_dir)
            f0 = meta.get("measured_f0", 0.0) or 0.0
            m = calculate_metrics(samples, f0=f0)

            label = os.path.basename(capture_dir)
            time_axis = np.arange(len(samples)) / SAMPLE_RATE

            # --- Waveform ---
            self.ax_wave.clear()
            self.ax_wave.plot(time_axis, samples, color="#44ccff", linewidth=0.5)
            self.ax_wave.set_title(
                f"{label} (Key {meta.get('key_index', '?')})",
                color="white",
                fontweight="bold",
            )
            self.ax_wave.set_ylabel("Amplitude")
            self.ax_wave.set_facecolor("#1e1e1e")

            # --- RMS + HCR overlay ---
            self.ax_rms.clear()
            self.ax_hcr.clear()

            hop_t = np.arange(len(m["hop_rms"])) * HOP_SIZE / SAMPLE_RATE
            self.ax_rms.step(
                hop_t, m["hop_rms"], color="#ffcc44", where="post", label="RMS"
            )

            # Update HCR twin axis
            self.ax_hcr.step(
                hop_t,
                m["hop_hcr"],
                color="#cc88ff",
                where="post",
                alpha=0.8,
                linewidth=1.5,
                label="HCR",
            )
            self.ax_hcr.set_ylim(0, 1.1)  # Expanded slightly so 1.0 is visible
            self.ax_hcr.set_ylabel("HCR", color="#cc88ff", fontsize=8)
            self.ax_hcr.tick_params(axis="y", colors="#cc88ff", labelsize=7)
            self.ax_hcr.axhline(y=0.20, color="#cc88ff", linestyle=":", alpha=0.5)

            self.ax_rms.set_ylabel("RMS")
            self.ax_rms.set_facecolor("#1e1e1e")
            # Summary annotation
            hcr_flag = "Good" if m["hcr_early"] >= 0.20 else "Weak harmonics"
            hcr_color = "limegreen" if m["hcr_early"] >= 0.20 else "orange"
            self.ax_rms.text(
                0.01,
                0.88,
                f"Decay rate: {m['decay_rate']:.1f}x  |  HCR: early={m['hcr_early']:.2f} mid={m['hcr_mid']:.2f} late={m['hcr_late']:.2f}  ({hcr_flag})",
                transform=self.ax_rms.transAxes,
                color=hcr_color,
                fontsize=8,
                fontweight="bold",
                bbox=dict(facecolor="black", alpha=0.6),
            )

            # --- Spectrogram ---
            self.ax_spec.clear()
            # Parameters tuned for 1.5s piano strikes
            # NFFT 4096 gives good freq resolution for harmonics
            Pxx, freqs, bins, im = self.ax_spec.specgram(
                samples, NFFT=2048, Fs=SAMPLE_RATE, noverlap=1024, cmap="magma"
            )
            self.ax_spec.set_ylabel("Freq (Hz)")
            self.ax_spec.set_xlabel("Time (s)")
            self.ax_spec.set_ylim(0, 5000)  # Limit to relevant range
            self.ax_spec.set_facecolor("#000000")

            # Show partials if available
            partials = meta.get("partials", [])
            if partials:
                # Add horizontal markers for detected partials
                for p in partials[:8]:  # Show first 8 to avoid clutter
                    self.ax_spec.axhline(
                        y=p["frequency"],
                        color="cyan",
                        alpha=0.3,
                        linestyle=":",
                        linewidth=1,
                    )

            self.fig.patch.set_facecolor("#121212")
            for ax in [self.ax_wave, self.ax_rms, self.ax_spec]:
                ax.tick_params(colors="white")
                ax.xaxis.label.set_color("white")
                ax.yaxis.label.set_color("white")
                if ax != self.ax_spec:
                    ax.grid(True, color="#333333", linestyle="--")

            print(
                f"Viewing: {label} | decay={m['decay_rate']:.1f}x | HCR early={m['hcr_early']:.2f} mid={m['hcr_mid']:.2f} late={m['hcr_late']:.2f}"
            )
            plt.draw()

        def next(self, _):
            self.current_idx = (self.current_idx + 1) % len(self.directories)
            self.update_plot()

        def prev(self, _):
            self.current_idx = (self.current_idx - 1) % len(self.directories)
            self.update_plot()

        def on_key(self, event):
            if event.key == "right":
                self.next(None)
            elif event.key == "left":
                self.prev(None)

    analyzer = CaptureAnalyzer(target_path)
    plt.show()


# --- Entry Point ---


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("path", help="Path to diagnostic directory or folder")
    parser.add_argument("--gui", action="store_true", help="Launch interactive plotter")
    args = parser.parse_args()

    target = args.path.rstrip("/")

    # Collect directories
    if os.path.isdir(target) and not os.path.exists(os.path.join(target, "audio.raw")):
        directories = sorted(
            [
                os.path.join(target, d)
                for d in os.listdir(target)
                if os.path.isdir(os.path.join(target, d))
                and os.path.exists(os.path.join(target, d, "audio.raw"))
            ]
        )
    else:
        directories = (
            [target] if os.path.exists(os.path.join(target, "audio.raw")) else []
        )

    if not directories:
        print("No captures found.")
        return

    if args.gui:
        run_gui(directories)
    else:
        for d in directories:
            print_text_analysis(d)


if __name__ == "__main__":
    main()
