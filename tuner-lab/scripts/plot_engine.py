import pandas as pd
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
import sys
import os

if len(sys.argv) < 2:
    print("Usage: python plot_engine.py <directory>")
    sys.exit(1)

directory = sys.argv[1]
csv_path = os.path.join(directory, "goertzel.csv")
out_path = os.path.join(directory, "goertzel_analysis.png")

if not os.path.exists(csv_path):
    print(f"Missing goertzel.csv in {directory}")
    sys.exit(1)

df = pd.read_csv(csv_path)

# Filter for Partial 1
df_p1 = df[df['partial_n'] == 1].copy()

if len(df_p1) == 0:
    print(f"No Partial 1 data found in {directory}")
    sys.exit(0)

# Extract key_idx from the first row
key_idx = df_p1['key_idx'].iloc[0]
f0_et = 440.0 * (2.0 ** ((key_idx - 48) / 12.0))

fig, (ax1, ax2) = plt.subplots(2, 1, figsize=(10, 8), sharex=True)

# Top Panel: Amplitude vs SNR Gate Threshold
ax1.set_title(f"Partial 1 Amplitude vs SNR Gate (Key {key_idx})")
frames = df_p1['frame']
amplitude = df_p1['amplitude']
t_amp_threshold = df_p1['t_amp']

ax1.plot(frames, amplitude, label='Amplitude', color='blue')
ax1.plot(frames, t_amp_threshold, label='Neyman-Pearson Threshold', color='red', linestyle='--')

# Fill red when amplitude is below threshold (DEAD)
ax1.fill_between(frames, amplitude, t_amp_threshold, where=(amplitude < t_amp_threshold), interpolate=True, color='red', alpha=0.3, label='DEAD Zone')
# Fill green when alive
ax1.fill_between(frames, amplitude, t_amp_threshold, where=(amplitude >= t_amp_threshold), interpolate=True, color='green', alpha=0.3, label='ALIVE Zone')

ax1.set_ylabel("Amplitude")
ax1.legend()
ax1.grid(True)

# Bottom Panel: Frequencies relative to f0_et
ax2.set_title("Tracking Seed vs Measured Frequency (Offset from ET)")

# Calculate offset from exact ET frequency
target_offset = df_p1['target_hz'] - f0_et
measured_offset = df_p1['measured_hz'] - f0_et

# Only plot measured offset where it is alive
alive_mask = df_p1['is_alive']

ax2.plot(frames, target_offset, label='TWM Seed (target_hz)', color='orange', linestyle=':')
ax2.plot(frames[alive_mask], measured_offset[alive_mask], label='Goertzel Result (measured_hz)', color='green', marker='o', markersize=4)

ax2.axhline(0, color='black', linewidth=1, linestyle='-', label=f'Expected f0_et ({f0_et:.1f} Hz)')
ax2.set_xlabel("Frame Index")
ax2.set_ylabel("Difference from ET (Hz)")
ax2.legend()
ax2.grid(True)

plt.tight_layout()
plt.savefig(out_path, dpi=150)
plt.close()
print(f"Saved {out_path}")
