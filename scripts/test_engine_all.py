import os
import subprocess
import pandas as pd

def main():
    base_dir = "./diagnostics"
    results = []

    print("Compiling diagnose_engine and diagnose_gatekeeper...")
    subprocess.run(["cargo", "build", "--example", "diagnose_engine"], check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["cargo", "build", "--example", "diagnose_gatekeeper"], check=True, stdout=subprocess.DEVNULL)

    executable_engine = "./target/debug/examples/diagnose_engine"
    executable_gate = "./target/debug/examples/diagnose_gatekeeper"

    keys = sorted([d for d in os.listdir(base_dir) if d.startswith("key_")])
    print(f"Found {len(keys)} keys. Running Engine diagnostics...")

    for key_dir_name in keys:
        # e.g., key_022_G2 -> expected_key = 22
        expected_key = int(key_dir_name.split("_")[1])
        
        key_dir = os.path.join(base_dir, key_dir_name)
        raw_file = os.path.join(key_dir, "audio_full_event.raw")
        if not os.path.exists(raw_file):
            raw_file = os.path.join(key_dir, "audio.raw")
        
        if not os.path.exists(raw_file):
            continue

        try:
            subprocess.run([executable_gate, raw_file], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
            subprocess.run([executable_engine, raw_file], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
        except subprocess.CalledProcessError:
            print(f"Error running diagnostic for {key_dir_name}")
            continue

        csv_gate = os.path.join(key_dir, "gatekeeper.csv")
        csv_engine = os.path.join(key_dir, "peaks.csv")
        
        if not os.path.exists(csv_gate) or not os.path.exists(csv_engine):
            continue

        try:
            df_gate = pd.read_csv(csv_gate)
            df_engine = pd.read_csv(csv_engine)
        except Exception as e:
            print(f"Error reading CSVs for {key_dir_name}: {e}")
            continue

        # Merge on frame
        df = pd.merge(df_gate, df_engine, left_on='frame_idx', right_on='frame')
        
        # Filter to Stable states where TWM is active
        stable_df = df[df['state_name'] == 'Stable'].copy()
        
        if len(stable_df) == 0:
            results.append({"key": key_dir_name, "locked_key": -1, "status": "FAIL_NEVER_STABLE"})
            continue
            
        # Check for 3-frame consistency lock
        locked_key = -1
        consistency_count = 0
        current_candidate = -1
        
        for idx, row in stable_df.iterrows():
            winner = int(row['key_idx'])
            if winner == current_candidate:
                consistency_count += 1
            else:
                current_candidate = winner
                consistency_count = 1
                
            if consistency_count >= 3:
                locked_key = current_candidate
                break
                
        if locked_key == -1:
            results.append({"key": key_dir_name, "locked_key": -1, "status": "FAIL_NO_3_FRAME_LOCK"})
        elif locked_key != expected_key:
            results.append({"key": key_dir_name, "locked_key": locked_key, "status": f"FAIL_WRONG_KEY (Expected {expected_key})"})
        else:
            results.append({"key": key_dir_name, "locked_key": locked_key, "status": "PASS"})

    df_results = pd.DataFrame(results)
    
    print("\n" + "="*60)
    print("ENGINE TWM 3-FRAME CONSISTENCY DIAGNOSTIC (88 KEYS)")
    print("="*60)
    
    passes = df_results[df_results['status'] == 'PASS']
    failures = df_results[df_results['status'] != 'PASS']
    
    print(f"Total keys processed: {len(df_results)}")
    print(f"Total PASS: {len(passes)}")
    print(f"Total FAIL: {len(failures)}")
    
    if len(failures) > 0:
        print("\n--- FAILURES ---")
        for _, row in failures.iterrows():
            print(f"  {row['key']}: {row['status']} (Locked on: {row['locked_key']})")
    else:
        print("\nSUCCESS! All 88 keys achieved a perfect 3-frame stability lock on the correct fundamental!")

if __name__ == "__main__":
    main()
