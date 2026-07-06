import subprocess
import json
import optuna
import re
import sys
import os

EXPECTED_FINGERPRINT = "e11fea90889dee30"

EVALUATOR_BIN = "./target/release/examples/mobo_evaluator"
DB_PATH = "twm_mobo.db"

# Multi-start seeds (review §8.11): the synthetic optimum LOCATION is seed-noise, so
# each arm is run under several seeds and the Pareto candidates are POOLED. This is
# multi-start optimization for coverage, NOT ensemble averaging (evaluation is
# deterministic — nothing to average). Population is also enlarged (50→128) because
# seed-fragility at pop=50 in a 5-D space is premature convergence.
SEEDS = [42, 1, 7]
POPULATION_SIZE = 128

# Degeneracy gate (review §8.3): reject trials in the error-collapse regime (≥half the
# keys score ≈0 on >FLOOR_GATE of frames) so NSGA-II can't exploit a spuriously low
# objective there.
FLOOR_GATE = 0.05


class Evaluator:
    def __init__(self):
        if not os.path.exists(EVALUATOR_BIN):
            raise RuntimeError(f"Evaluator binary not found at {EVALUATOR_BIN}. Please build it first.")

        self.proc = subprocess.Popen(
            [EVALUATOR_BIN, "--serve"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1
        )

        self.fingerprint = None
        while True:
            line = self.proc.stderr.readline()
            if not line:
                err = self.proc.stderr.read()
                raise RuntimeError(f"Evaluator process died before becoming ready. stderr:\n{err}")
            if "ready:" in line:
                print("Evaluator ready:", line.strip())
                # Parse "…, fingerprint <hex>, …" so the orchestrator can bind the
                # study to the exact dataset (a subtly-drifted dataset would
                # otherwise pass the objA sanity gate silently).
                m = re.search(r"fingerprint ([0-9a-f]{16})", line)
                if m:
                    self.fingerprint = m.group(1)
                break

    def evaluate(self, mode, p, q, r, rho, lambd, nonpeak=0.0, smoothness=0.0):
        """Run one trial; returns the full metrics dict. Always sends the 8-token
        protocol (structural coeffs default 0 = off)."""
        lambd_str = "inf" if lambd == float("inf") else f"{lambd:.6f}"
        input_line = (
            f"{mode} {p:.6f} {q:.6f} {r:.6f} {rho:.6f} {lambd_str} "
            f"{nonpeak:.6f} {smoothness:.6f}\n"
        )
        self.proc.stdin.write(input_line)
        self.proc.stdin.flush()

        out_line = self.proc.stdout.readline()
        if not out_line:
            err = self.proc.stderr.read()
            raise RuntimeError(f"Evaluator process died unexpectedly. stderr:\n{err}")

        try:
            return json.loads(out_line)
        except json.JSONDecodeError as e:
            raise RuntimeError(f"Failed to parse evaluator JSON: {out_line!r}") from e

    def close(self):
        if self.proc.poll() is None:
            self.proc.stdin.close()
            self.proc.wait()


def compute_2d_hypervolume(pts, ref_x=1.0, ref_y=1.0):
    """2D hypervolume for MINIMIZING both objectives (bass FL, treble FL), bounded by
    the worst-corner reference (1.0, 1.0). Larger = better. Used only for the
    plateau-stopper; final selection is on real-capture validation, never HV."""
    valid = [(x, y) for (x, y) in pts if x < ref_x and y < ref_y]
    if not valid:
        return 0.0
    valid.sort(key=lambda p: p[0])  # x ascending → y descending along the front
    hv = 0.0
    prev_y = ref_y
    for x, y in valid:
        if y < prev_y:
            hv += (ref_x - x) * (prev_y - y)
            prev_y = y
    return hv


def suggest_params(trial, fixed, free):
    """Resolve the 7 TWM params for a trial: fixed where pinned, suggested where free."""
    bounds = {
        "p": (0.0, 1.0),
        "q": (0.0, 8.0),          # widened from the old 4.0 ceiling
        "r": (0.0, 3.0),          # widened from the old 1.5 ceiling
        "rho": (0.0, 1.5),
        "lambda": (1.0, 50.0),
        "nonpeak": (0.0, 1.0),    # structural (co-tuned arm): un-normalized count penalty
        "smoothness": (0.0, 3.0), # structural (co-tuned arm): amplitude-incoherence penalty
    }
    out = {}
    for name in ("p", "q", "r", "rho", "lambda", "nonpeak", "smoothness"):
        if name in fixed:
            out[name] = fixed[name]
        elif name in free:
            lo, hi = bounds[name]
            out[name] = trial.suggest_float(name, lo, hi)
        else:
            out[name] = float("inf") if name == "lambda" else 0.0  # off by default
    return out


def optimize_arm(evaluator, arm_num, mode, fixed, free, seed, n_trials=2000):
    """Run one (arm, seed) study. objA = bass false-lock, objB = treble false-lock
    (both MINIMIZE) — an orthogonal, decision-relevant tradeoff (review: the previous
    ordinal objB was near-degenerate). Returns the study's Pareto points."""
    def objective(trial):
        prm = suggest_params(trial, fixed, free)
        d = evaluator.evaluate(
            mode, prm["p"], prm["q"], prm["r"], prm["rho"], prm["lambda"],
            prm["nonpeak"], prm["smoothness"],
        )

        trial.set_user_attr("diagnostics", {
            "n": d.get("n"),
            "prod_fl_overall": d.get("prod_fl"),  # overall production K=3 (selection-relevant)
            "sep_fl": d.get("sep_fl"),
            "fl_bass": d.get("fl_bass"),
            "fl_mid": d.get("fl_mid"),
            "fl_treble": d.get("fl_treble"),
            "fl_hard": d.get("fl_hard"),
            "fidelity_mean": d.get("fidelity_mean"),
            # K-robustness sweep: is the config K=3-overfit or K-robust?
            "prod_fl_k2": d.get("prod_fl_k2"),
            "prod_fl_k3": d.get("prod_fl_k3"),
            "prod_fl_k4": d.get("prod_fl_k4"),
            "prod_fl_k5": d.get("prod_fl_k5"),
            "floor_frac": d.get("floor_frac"),
        })

        if (d.get("floor_frac") or 0.0) > FLOOR_GATE:
            return 1.0, 1.0  # worst corner (both minimized) — exclude collapse regime

        # OPTIMIZED objectives: bass FL vs treble FL (both minimize).
        return d["fl_bass"], d["fl_treble"]

    class PlateauStopper:
        def __init__(self, patience=300):
            self.patience = patience
            self.best_hv = -1.0
            self.stagnant_trials = 0

        def __call__(self, study, trial):
            if len(study.trials) % 10 == 0:
                pts = [(t.values[0], t.values[1]) for t in study.best_trials if t.values is not None]
                hv = compute_2d_hypervolume(pts)
                if hv > self.best_hv + 1e-4:
                    self.best_hv = hv
                    self.stagnant_trials = 0
                else:
                    self.stagnant_trials += 10
                    if self.stagnant_trials >= self.patience:
                        print(f"  [arm {arm_num} seed {seed}] HV plateaued at {self.best_hv:.4f}; stopping.")
                        study.stop()

    study = optuna.create_study(
        study_name=f"twm_arm_{arm_num}_s{seed}",
        storage=f"sqlite:///{DB_PATH}",
        directions=["minimize", "minimize"],  # bass FL, treble FL
        sampler=optuna.samplers.NSGAIISampler(population_size=POPULATION_SIZE, seed=seed),
        load_if_exists=True,
    )
    study.optimize(objective, n_trials=n_trials, callbacks=[PlateauStopper(patience=300)])

    pts = []
    for t in study.best_trials:
        if t.values is None:
            continue
        pts.append({
            "seed": seed,
            "number": t.number,
            "params": t.params,
            "fl_bass": t.values[0],
            "fl_treble": t.values[1],
            "diagnostics": t.user_attrs.get("diagnostics"),
        })
    return pts


def pool_pareto(per_seed_points):
    """Pool candidates across seeds; dedup by rounded param tuple. The union is the
    candidate menu handed to real-capture validation (we do NOT pick by synthetic HV)."""
    seen = set()
    pooled = []
    for pt in per_seed_points:
        key = tuple(sorted((k, round(v, 4)) for k, v in pt["params"].items()))
        if key in seen:
            continue
        seen.add(key)
        pooled.append(pt)
    return pooled


def main():
    # Guard against silent trial accumulation: load_if_exists=True means a second run
    # RESUMES each study. Refuse rather than auto-delete (never destroy a prior run).
    if os.path.exists(DB_PATH) and "--resume" not in sys.argv:
        print(
            f"'{DB_PATH}' already exists. For a fresh run, delete it "
            f"(rm {DB_PATH}); to continue the existing studies, pass --resume.",
            file=sys.stderr,
        )
        sys.exit(1)

    evaluator = Evaluator()
    try:
        # Bind the study to the exact dataset: a subtly-drifted dataset would pass the
        # objA sanity window below but invalidate cross-run comparison. Fail loud.
        assert evaluator.fingerprint == EXPECTED_FINGERPRINT, (
            f"Dataset fingerprint {evaluator.fingerprint} != expected "
            f"{EXPECTED_FINGERPRINT}. The synthetic dataset drifted — refusing to run."
        )

        print("Running sanity check...")
        d = evaluator.evaluate("refine", 0.5, 1.4, 0.5, 0.33, 18.0)
        print(f"Sanity M&B: prod_fl(overall)={d['prod_fl']:.5f} "
              f"fl_bass={d['fl_bass']:.5f} fl_treble={d['fl_treble']:.5f}")
        # Overall production K=3 false-lock for the M&B default must reproduce ~0.308
        # (fingerprint e11fea90889dee30). A mismatch ⇒ flipped objective / stale binary
        # / protocol regression — fail loud now, not after a multi-hour sweep.
        assert abs(d["prod_fl"] - 0.308) < 0.02, (
            f"Sanity check failed: prod_fl={d['prod_fl']:.5f}, expected ~0.308. "
            "Rebuild the evaluator (cargo build --release --example mobo_evaluator) "
            "and confirm the protocol before running the sweep."
        )

        # Arms. objB is now bass-vs-treble FL, so the 1-5 arms keep their param-freedom
        # structure; arms 6-7 co-tune a structural term (the test ADR 0006 Finding #3
        # says the frozen-constant bolt-on rejections were invalid for).
        arms = [
            (1, "refine", {"p": 0.5, "lambda": float("inf")}, ["q", "r", "rho"]),
            (2, "refine", {"p": 0.5, "lambda": 18.0}, ["q", "r", "rho"]),
            (3, "refine", {"p": 0.5}, ["q", "r", "rho", "lambda"]),
            (4, "refine", {}, ["p", "q", "r", "rho", "lambda"]),
            (5, "discrete", {}, ["p", "q", "r", "rho", "lambda"]),
            (6, "refine", {"p": 0.5, "lambda": 18.0}, ["q", "r", "rho", "nonpeak"]),
            (7, "refine", {"p": 0.5, "lambda": 18.0}, ["q", "r", "rho", "smoothness"]),
        ]

        summary = {}
        for arm_num, mode, fixed, free in arms:
            print(f"\n--- Arm {arm_num} ({mode}); free={free} ---")
            per_seed = []
            for seed in SEEDS:
                per_seed += optimize_arm(evaluator, arm_num, mode, fixed, free, seed)
            pooled = pool_pareto(per_seed)
            with open(f"twm_pareto_arm{arm_num}.json", "w") as f:
                json.dump(pooled, f, indent=2)
            summary[arm_num] = (len(pooled), len(per_seed))
            print(f"Arm {arm_num}: {len(pooled)} pooled Pareto candidates "
                  f"({len(per_seed)} pre-dedup across {len(SEEDS)} seeds).")

        print("\n=== Pooled Pareto candidate counts ===")
        for arm_num, mode, fixed, free in arms:
            pooled_n, raw_n = summary[arm_num]
            fixed_str = ",".join(f"{k}={v}" for k, v in fixed.items()) or "-"
            print(f"arm {arm_num:<2} | {mode:<8} | free={','.join(free):<24} | "
                  f"fixed={fixed_str:<16} | {pooled_n} candidates")

    finally:
        evaluator.close()


if __name__ == "__main__":
    main()
