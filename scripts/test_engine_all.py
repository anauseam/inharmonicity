import os
import subprocess
import pandas as pd
import argparse

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--refine", action="store_true", help="Enable Stage B refinement")
    args = parser.parse_args()
    base_dir = "./diagnostics"
    results = []

    print("Compiling diagnose_engine (with telemetry) and diagnose_gatekeeper...")
    subprocess.run(["cargo", "build", "--example", "diagnose_engine", "--features", "telemetry"], check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["cargo", "build", "--example", "diagnose_gatekeeper"], check=True, stdout=subprocess.DEVNULL)

    executable_engine = "./target/debug/examples/diagnose_engine"
    executable_gate = "./target/debug/examples/diagnose_gatekeeper"

    keys = sorted([d for d in os.listdir(base_dir) if d.startswith("key_")])
    mode_str = "REFINED" if args.refine else "DISCRETE"
    print(f"Found {len(keys)} keys. Running Engine diagnostics ({mode_str} mode)...")

    for key_dir_name in keys:
        expected_key = int(key_dir_name.split("_")[1])
        key_dir = os.path.join(base_dir, key_dir_name)
        raw_file = os.path.join(key_dir, "audio_full_event.raw")
        if not os.path.exists(raw_file):
            raw_file = os.path.join(key_dir, "audio.raw")
        
        if not os.path.exists(raw_file):
            continue

        try:
            subprocess.run([executable_gate, raw_file], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
            
            engine_cmd = [executable_engine, raw_file]
            if args.refine:
                engine_cmd.append("--refine")
            subprocess.run(engine_cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
            
            # Run the plotting script
            subprocess.run(["python", "./scripts/plot_engine.py", key_dir], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
        except subprocess.CalledProcessError:
            print(f"Error running diagnostic for {key_dir_name}")
            continue

        csv_gate = os.path.join(key_dir, "gatekeeper.csv")
        csv_engine = os.path.join(key_dir, "peaks.csv")
        csv_goertzel = os.path.join(key_dir, "goertzel.csv")
        
        if not os.path.exists(csv_gate) or not os.path.exists(csv_engine) or not os.path.exists(csv_goertzel):
            continue

        try:
            df_gate = pd.read_csv(csv_gate)
            df_engine = pd.read_csv(csv_engine)
            df_goertzel = pd.read_csv(csv_goertzel)
        except Exception as e:
            print(f"Error reading CSVs for {key_dir_name}: {e}")
            continue

        # Merge TWM/Consistency checks on frame
        df = pd.merge(df_gate, df_engine, left_on='frame_idx', right_on='frame')
        stable_df = df[df['state_name'] == 'Stable'].copy()
        
        p1_alive_frames = 0
        p1_dead_frames = 0
        if len(df_goertzel) > 0:
            p1_data = df_goertzel[df_goertzel['partial_n'] == 1]
            p1_alive_frames = len(p1_data[p1_data['is_alive'] == True])
            p1_dead_frames = len(p1_data[p1_data['is_alive'] == False])

        median_s_win = 0.0
        if 's_win_cents' in stable_df.columns and len(stable_df) > 0:
            median_s_win = stable_df['s_win_cents'].median()

        if len(stable_df) == 0:
            results.append({"key": key_dir_name, "locked_key": -1, "status": "FAIL_NEVER_STABLE", "p1_alive": p1_alive_frames, "p1_dead": p1_dead_frames, "median_s_win": 0.0})
            continue
            
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
            results.append({"key": key_dir_name, "locked_key": -1, "status": "FAIL_NO_3_FRAME_LOCK", "p1_alive": p1_alive_frames, "p1_dead": p1_dead_frames, "median_s_win": median_s_win})
        elif locked_key != expected_key:
            results.append({"key": key_dir_name, "locked_key": locked_key, "status": f"FAIL_WRONG_KEY (Expected {expected_key})", "p1_alive": p1_alive_frames, "p1_dead": p1_dead_frames, "median_s_win": median_s_win})
        else:
            results.append({"key": key_dir_name, "locked_key": locked_key, "status": "PASS", "p1_alive": p1_alive_frames, "p1_dead": p1_dead_frames, "median_s_win": median_s_win})

    df_results = pd.DataFrame(results)
    
    print("\n" + "="*80)
    print("ENGINE TWM & GOERTZEL DIAGNOSTIC (88 KEYS)")
    print("="*80)
    
    passes = df_results[df_results['status'] == 'PASS']
    failures = df_results[df_results['status'] != 'PASS']
    
    print(f"Total keys processed: {len(df_results)}")
    print(f"TWM Lock PASS: {len(passes)}")
    print(f"TWM Lock FAIL: {len(failures)}")
    
    if len(failures) > 0:
        print("\n--- TWM LOCK FAILURES ---")
        for _, row in failures.iterrows():
            print(f"  {row['key']}: {row['status']} (Locked on: {row['locked_key']}) s_win: {row['median_s_win']:+.1f}c")
    else:
        print("\nSUCCESS! All 88 keys achieved a perfect 3-frame stability lock on the correct fundamental!")

    print("\n--- REFINED SCALE SUMMARY ---")
    if args.refine:
        s_wins = df_results['median_s_win']
        print(f"s_win_cents distribution: min={s_wins.min():+.1f}c, median={s_wins.median():+.1f}c, max={s_wins.max():+.1f}c")
        for _, row in df_results.iterrows():
            if abs(row['median_s_win']) >= 79.0:
                print(f"  WARNING: {row['key']} s_win pinned at {row['median_s_win']:+.1f}c (possible edge clip)")
    else:
        print("Discrete mode (s_win_cents = 0.0)")

    print("\n--- GOERTZEL PARTIAL 1 TRACKING SUMMARY ---")
    dead_keys = df_results[(df_results['p1_alive'] == 0) & (df_results['p1_dead'] > 0)]
    print(f"Keys where Partial 1 was completely DEAD: {len(dead_keys)}")
    for _, row in dead_keys.iterrows():
        print(f"  {row['key']}: DEAD ({row['p1_dead']} frames)")
        
    struggling_keys = df_results[(df_results['p1_alive'] > 0) & (df_results['p1_dead'] > 0)]
    if len(struggling_keys) > 0:
        print(f"\nKeys where Partial 1 struggled (flickered ALIVE/DEAD): {len(struggling_keys)}")
        for _, row in struggling_keys.iterrows():
            print(f"  {row['key']}: {row['p1_alive']} ALIVE / {row['p1_dead']} DEAD")

if __name__ == "__main__":
    main()
