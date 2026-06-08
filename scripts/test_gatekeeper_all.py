import os
import subprocess
import pandas as pd

def main():
    base_dir = "./diagnostics"
    results = []

    # Compile the example first
    print("Compiling diagnose_gatekeeper...")
    subprocess.run(["cargo", "build", "--example", "diagnose_gatekeeper"], check=True, stdout=subprocess.DEVNULL)

    executable = "./target/debug/examples/diagnose_gatekeeper"

    keys = []
    for d in os.listdir(base_dir):
        if d.startswith("key_"):
            keys.append(d)
    
    keys.sort() # Sort alphabetically/numerically
    
    print(f"Found {len(keys)} keys. Running diagnostics...")

    for key in keys:
        key_dir = os.path.join(base_dir, key)
        raw_file = os.path.join(key_dir, "audio_full_event.raw")
        if not os.path.exists(raw_file):
            raw_file = os.path.join(key_dir, "audio.raw")
        
        if not os.path.exists(raw_file):
            continue

        # Run the executable
        try:
            subprocess.run([executable, raw_file], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
        except subprocess.CalledProcessError:
            print(f"Error running diagnostic for {key}")
            continue

        # Read the resulting CSV
        csv_file = os.path.join(key_dir, "gatekeeper.csv")
        if not os.path.exists(csv_file):
            print(f"CSV not generated for {key}")
            continue

        try:
            df = pd.read_csv(csv_file)
        except Exception as e:
            print(f"Error reading CSV for {key}: {e}")
            continue

        # Calculate how many frames we spent in Unstable (after onset) before Stable
        # Onset is when nhwrsf spikes, or when state transitions from 0 to 1/2
        
        onset_idx = df.index[df['is_new_onset']].tolist()
        if not onset_idx:
            # Maybe it never detected an onset
            results.append({"key": key, "transient_frames": -1, "status": "No Onset"})
            continue
        
        first_onset = onset_idx[0]
        
        # Find first Stable frame AFTER onset
        stable_idx = df.index[(df.index > first_onset) & (df['state_name'] == 'Stable')].tolist()
        
        if not stable_idx:
            results.append({"key": key, "transient_frames": -1, "status": "Never Stable"})
            continue
            
        first_stable = stable_idx[0]
        
        # Frames spent in transient = first_stable - first_onset
        frames_waiting = first_stable - first_onset
        
        status = "OK"
        if frames_waiting >= 20:
            status = "TIMEOUT"
            
        results.append({
            "key": key,
            "transient_frames": frames_waiting,
            "status": status
        })

    # Summarize
    df_results = pd.DataFrame(results)
    
    print("\n" + "="*50)
    print("GATEKEEPER 88-KEY DIAGNOSTIC SUMMARY")
    print("="*50)
    
    valid = df_results[df_results['transient_frames'] >= 0]
    timeouts = df_results[df_results['status'] == 'TIMEOUT']
    no_onsets = df_results[df_results['status'] == 'No Onset']
    never_stable = df_results[df_results['status'] == 'Never Stable']
    
    print(f"Total keys processed: {len(df_results)}")
    print(f"Successfully stabilized: {len(valid)}")
    
    if len(valid) > 0:
        print("\n--- Transient Delay Stats ---")
        print(f"Min Delay: {valid['transient_frames'].min()} frames ({valid['transient_frames'].min() * 46.4:.1f} ms)")
        print(f"Max Delay: {valid['transient_frames'].max()} frames ({valid['transient_frames'].max() * 46.4:.1f} ms)")
        print(f"Mean Delay: {valid['transient_frames'].mean():.1f} frames ({valid['transient_frames'].mean() * 46.4:.1f} ms)")
        print(f"Median Delay: {valid['transient_frames'].median():.1f} frames ({valid['transient_frames'].median() * 46.4:.1f} ms)")
    
    if len(timeouts) > 0:
        print("\n--- WARNING: TIMEOUTS (>= 20 frames) ---")
        for _, row in timeouts.iterrows():
            print(f"  {row['key']}: {row['transient_frames']} frames")
            
    if len(no_onsets) > 0:
        print("\n--- WARNING: NO ONSET DETECTED ---")
        for _, row in no_onsets.iterrows():
            print(f"  {row['key']}")

    if len(never_stable) > 0:
        print("\n--- WARNING: NEVER STABLE ---")
        for _, row in never_stable.iterrows():
            print(f"  {row['key']}")
            
    print("="*50)

if __name__ == "__main__":
    main()
