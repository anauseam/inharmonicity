import numpy as np
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
import pandas as pd
import sys
import os

if len(sys.argv) < 2:
    print("Usage: python plot_gatekeeper.py <directory>")
    sys.exit(1)

directory = sys.argv[1]
raw_path = os.path.join(directory, "audio_full_event.raw")
csv_path = os.path.join(directory, "gatekeeper.csv")
out_path = os.path.join(directory, "gatekeeper_analysis.png")

if not os.path.exists(raw_path) or not os.path.exists(csv_path):
    print("Missing raw or csv file")
    sys.exit(1)

# Load audio
audio = np.fromfile(raw_path, dtype=np.float32)
sample_rate = 44100
time_axis = np.arange(len(audio)) / sample_rate * 1000.0 # ms

import json

# Load CSV
df = pd.read_csv(csv_path)

# Load noise floor from analysis.json for RMS threshold
noise_floor = 0.005
json_path = os.path.join(directory, "analysis.json")
if os.path.exists(json_path):
    try:
        with open(json_path, 'r') as f:
            data = json.load(f)
            if 'metadata' in data and 'noise_floor' in data['metadata']:
                noise_floor = float(data['metadata']['noise_floor'])
    except Exception as e:
        pass

# Figure setup
fig, (ax1, ax2, ax3, ax4) = plt.subplots(4, 1, figsize=(12, 14))

# 1. Spectrogram
ax1.specgram(audio, NFFT=2048, Fs=sample_rate, noverlap=1024, cmap='viridis')
ax1.set_ylim(0, 5000)
ax1.set_ylabel('Frequency (Hz)')
ax1.set_title('Ground Truth: Spectrogram (Visualizing the Broadband Transient)')
ax1.set_xlim(0, time_axis[-1] / 1000.0) # specgram uses seconds
# overlay vertical lines for state changes on specgram
for i in range(len(df)-1):
    state = df['state_enum'].iloc[i]
    if state == 1: # Unstable starts
        if i == 0 or df['state_enum'].iloc[i-1] == 0:
            ax1.axvline(x=df['time_ms'].iloc[i]/1000.0, color='red', linestyle='--', alpha=0.8)
    elif state == 2: # Stable starts
        if i == 0 or df['state_enum'].iloc[i-1] == 1:
            ax1.axvline(x=df['time_ms'].iloc[i]/1000.0, color='green', linestyle='--', alpha=0.8)

# 2. Waveform & State
ax2.plot(time_axis, audio, color='gray', alpha=0.6, label='Raw Audio')
ax2.set_ylabel('Amplitude')

# Overlay state as a colored background
state_colors = {0: 'white', 1: 'red', 2: 'green'}
for i in range(len(df)-1):
    t_start = df['time_ms'].iloc[i]
    t_end = df['time_ms'].iloc[i+1]
    state = df['state_enum'].iloc[i]
    if state in state_colors:
        ax2.axvspan(t_start, t_end, color=state_colors[state], alpha=0.3)

ax2.set_title('Waveform with Gatekeeper State (Red=Unstable, Green=Stable)')
ax2.set_xlim(0, time_axis[-1])

# 3. Transient & Volume Metrics
ax3.plot(df['time_ms'], df['nhwrsf'], label='NHWRSF (Onset)', color='blue')
ax3.axhline(y=0.5, color='blue', linestyle=':', label='NHWRSF Transient Threshold (0.5)')
ax3.plot(df['time_ms'], df['rms_ema'], label='RMS (Volume)', color='orange')
ax3.axhline(y=noise_floor, color='orange', linestyle=':', label=f'RMS Silence Threshold ({noise_floor:.3f})')
ax3.set_ylabel('Metric Value')
ax3.legend(loc='upper left', fontsize='small')
ax3.set_title('Gatekeeper Volume & Onset Metrics')
ax3.set_xlim(0, time_axis[-1])

# 4. Stability Metric (NINOS2)
if 'ninos2' in df.columns:
    ax4.plot(df['time_ms'], df['ninos2'], label='NINOS2 (Peak Tonal = High)', color='purple')
    ax4.axhline(y=10.0, color='purple', linestyle=':', label='NINOS2 Threshold (10.0)')
    ax4.set_ylabel('NINOS2 Sparsity', color='purple')
    ax4.tick_params(axis='y', labelcolor='purple')

ax4.set_title('Stability Metric: NINOS2')
ax4.set_xlabel('Time (ms)')
ax4.set_xlim(0, time_axis[-1])
ax4.legend(loc='upper left')

plt.tight_layout()
plt.savefig(out_path, dpi=150)
print(f"Saved plot to {out_path}")
