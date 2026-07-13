//! # Whittaker — the Whittaker smoother and the shared banded LS solver
//!
//! [`BandedSystem`], the banded SPD solver, is the smoother's numerical kernel and is
//! exported: engine (d)'s penalized least squares reuses it directly.
//!
//! The Whittaker smoother (Whittaker 1923 [1]; Eilers 2003 [2]): penalized
//! least squares on an equally spaced grid,
//!
//! ẑ = argmin_z ∑_i w_i (y_i - z_i)² + λ ∑_i (Δ² z_i)²,
//!
//! solved as the banded SPD system (W + λ Dᵀ D) z = W y with D
//! the second-difference matrix. The second-difference (curvature) penalty is
//! the design note's definition of smoothness (§5): any straight trend passes
//! free, only bending is charged. λ is selected by Eilers' fast
//! leave-one-out cross-validation (his Eq. 10: the LOO residual is
//! (y_i - ẑ_i)/(1 - h_{ii}) with h_{ii} the smoother-matrix
//! diagonal), which is statistical model selection — categorically distinct
//! from tuning on the validation captures (design note §5, defaults #4).
//!
//! [`BandedSystem`] is a general symmetric positive-definite banded normal-equation
//! accumulator + Cholesky solver, shared with the multi-interval engine (d)
//! in [`super::curves`] (bandwidth ≤ 24 there; 2 here). Cold-path only:
//! these functions allocate and are not for the DSP hot loop.
//!
//! # References
//! 1. E. T. Whittaker, "On a new method of graduation", Proc. Edinburgh
//!    Math. Soc. 41, 63–75 (1923).
//! 2. P. H. C. Eilers, "A perfect smoother", Analytical Chemistry 75(14),
//!    3631–3636 (2003).

/// Symmetric positive-definite banded linear system accumulated in
/// normal-equation form: A x = b with A_{ij} = 0 for |i - j| >
/// half-bandwidth. Lower-band storage: entry `(i, k)` holds A_{i, i-k}.
#[derive(Debug, Clone)]
pub struct BandedSystem {
    n: usize,
    hbw: usize,
    band: Vec<f64>,
    /// Right-hand side b.
    pub rhs: Vec<f64>,
}

impl BandedSystem {
    /// Creates an all-zero `n × n` system with the given half-bandwidth.
    pub fn new(n: usize, hbw: usize) -> Self {
        Self {
            n,
            hbw,
            band: vec![0.0; n * (hbw + 1)],
            rhs: vec![0.0; n],
        }
    }

    /// Adds `v` to A_{rc} (and by symmetry A_{cr}; only the lower
    /// triangle is stored). Panics if `|r - c|` exceeds the half-bandwidth —
    /// a caller bug per the assert-vs-Result rule.
    pub fn add(&mut self, r: usize, c: usize, v: f64) {
        let (i, j) = if r >= c { (r, c) } else { (c, r) };
        let k = i - j;
        assert!(
            k <= self.hbw,
            "entry ({r},{c}) outside half-bandwidth {}",
            self.hbw
        );
        self.band[i * (self.hbw + 1) + k] += v;
    }

    /// Accumulates one weighted least-squares residual row
    /// w·(aᵀx − t)² into the normal equations: A += w·a·aᵀ,
    /// b += w·t·a. `cols`/`coefs` are the row's sparse entries.
    pub fn add_row(&mut self, cols: &[usize], coefs: &[f64], target: f64, w: f64) {
        debug_assert_eq!(cols.len(), coefs.len());
        for (p, (&cp, &ap)) in cols.iter().zip(coefs).enumerate() {
            for (q, (&cq, &aq)) in cols.iter().zip(coefs).enumerate().skip(p) {
                // Off-diagonal pairs are stored once (symmetric lower band);
                // a repeated column index folds both cross terms onto the
                // diagonal, so it needs the factor 2.
                let factor = if p != q && cp == cq { 2.0 } else { 1.0 };
                self.add(cp, cq, factor * w * ap * aq);
            }
            self.rhs[cp] += w * target * ap;
        }
    }

    /// System dimension `n`.
    pub fn n(&self) -> usize {
        self.n
    }

    /// BandedSystem Cholesky factorization A = L Lᵀ. Returns `None` if the
    /// matrix is not positive definite (rank-deficient system — e.g. an
    /// unanchored difference system).
    pub fn cholesky(&self) -> Option<BandedCholesky> {
        let (n, hbw, w) = (self.n, self.hbw, self.hbw + 1);
        let mut l = self.band.clone();
        for i in 0..n {
            // Off-diagonals L[i, i-k], farthest column first.
            for k in (1..=hbw.min(i)).rev() {
                let j = i - k;
                let mut sum = l[i * w + k];
                // Σ_p L[i,p] · L[j,p] for p < j within both bands.
                let tmax = (hbw - k).min(j);
                for t in 1..=tmax {
                    sum -= l[i * w + k + t] * l[j * w + t];
                }
                l[i * w + k] = sum / l[j * w];
            }
            // Diagonal.
            let mut sum = l[i * w];
            for t in 1..=hbw.min(i) {
                sum -= l[i * w + t] * l[i * w + t];
            }
            if sum <= 0.0 || !sum.is_finite() {
                return None;
            }
            l[i * w] = sum.sqrt();
        }
        Some(BandedCholesky { n, hbw, l })
    }

    /// Factorizes and solves for the stored right-hand side.
    pub fn solve(&self) -> Option<Vec<f64>> {
        let chol = self.cholesky()?;
        let mut x = self.rhs.clone();
        chol.solve_in_place(&mut x);
        Some(x)
    }
}

/// BandedSystem Cholesky factor L (see [`BandedSystem::cholesky`]). Reusable across
/// multiple right-hand sides — the LOO-CV hat diagonal and the GCV effective
/// DOF both solve many systems against one factorization.
#[derive(Debug, Clone)]
pub struct BandedCholesky {
    n: usize,
    hbw: usize,
    l: Vec<f64>,
}

impl BandedCholesky {
    /// Solves L Lᵀ x = b in place.
    pub fn solve_in_place(&self, b: &mut [f64]) {
        assert_eq!(b.len(), self.n);
        let (n, hbw, w) = (self.n, self.hbw, self.hbw + 1);
        // Forward: L y = b.
        for i in 0..n {
            let mut sum = b[i];
            for k in 1..=hbw.min(i) {
                sum -= self.l[i * w + k] * b[i - k];
            }
            b[i] = sum / self.l[i * w];
        }
        // Backward: Lᵀ x = y.
        for i in (0..n).rev() {
            let mut sum = b[i];
            for k in 1..=hbw.min(n - 1 - i) {
                sum -= self.l[(i + k) * w + k] * b[i + k];
            }
            b[i] = sum / self.l[i * w];
        }
    }

    /// Diagonal of A⁻¹, by solving against each unit vector. O(n² p)
    /// — trivial at the 88-key scale; used for the CV hat diagonal.
    pub fn inverse_diag(&self) -> Vec<f64> {
        let mut diag = vec![0.0; self.n];
        let mut e = vec![0.0; self.n];
        for i in 0..self.n {
            e.fill(0.0);
            e[i] = 1.0;
            self.solve_in_place(&mut e);
            diag[i] = e[i];
        }
        diag
    }
}

/// Builds the Whittaker normal equations (W + λ Dᵀ D) with the
/// data vector on the RHS.
fn system(y: &[f64], w: &[f64], lambda: f64) -> BandedSystem {
    let n = y.len();
    let mut sys = BandedSystem::new(n, 2);
    for i in 0..n {
        sys.add(i, i, w[i]);
        sys.rhs[i] += w[i] * y[i];
    }
    // λ Σ (z_{i} − 2 z_{i+1} + z_{i+2})²: accumulate λ·ccᵀ per difference row.
    for r in 0..n.saturating_sub(2) {
        sys.add_row(&[r, r + 1, r + 2], &[1.0, -2.0, 1.0], 0.0, lambda);
    }
    sys
}

/// The Whittaker smoother (module doc): returns ẑ minimizing
/// ∑ w_i (y_i - z_i)² + λ ∑ (Δ² z_i)².
///
/// `w` are non-negative observation weights (0 = missing: the smoother
/// interpolates there). Returns `None` when the system is singular — fewer
/// than 2 strictly-positive weights leaves the affine null space of Dᵀ D
/// unconstrained. Limits: λ → 0 reproduces the weighted data;
/// λ → ∞ tends to the weighted least-squares *straight line*
/// (the penalty's null space), which is why the curve engines smooth the
/// *residual from the prior mean* — the prior carries the curve shape, the
/// residual's line component is the data's to keep (design note §5).
pub fn smooth(y: &[f64], w: &[f64], lambda: f64) -> Option<Vec<f64>> {
    assert_eq!(y.len(), w.len());
    system(y, w, lambda).solve()
}

/// Eilers' fast leave-one-out cross-validation score for one λ:
/// CV = ∑_{w_i > 0} w_i · ((y_i − ẑ_i)/(1 − h_ii))²,
/// where h_{ii} = [(W + λ Dᵀ D)⁻¹]_{ii} w_i is the smoother
/// hat diagonal (Eilers 2003, Eq. 10). Returns `None` on a singular system
/// or when some h_{ii} = 1 (a point the smoother reproduces exactly cannot
/// be cross-validated).
pub fn cv(y: &[f64], w: &[f64], lambda: f64) -> Option<f64> {
    let all: Vec<bool> = w.iter().map(|&x| x > 0.0).collect();
    cv_masked(y, w, lambda, &all)
}

/// [`cv`] restricted to a validation subset: the CV sum runs only
/// over indices where `cv_mask` is true (and w_i > 0).
///
/// Needed when the weight vector carries **prior pseudo-observations** —
/// the tuning-curve engines' boundary-reversion term (ADR 0007) observes
/// the prior mean at unmeasured keys with a small weight. Pseudo-points
/// encode the prior, not data, so cross-validating them would reward
/// λ-choices for predicting the prior back to itself; they are masked out
/// of the score while still shaping the smoother (they enter W and hence
/// h_{ii} and ẑ).
pub fn cv_masked(y: &[f64], w: &[f64], lambda: f64, cv_mask: &[bool]) -> Option<f64> {
    assert_eq!(y.len(), cv_mask.len());
    let sys = system(y, w, lambda);
    let chol = sys.cholesky()?;
    let mut z = sys.rhs.clone();
    chol.solve_in_place(&mut z);
    let inv_diag = chol.inverse_diag();
    let mut cv = 0.0;
    for i in 0..y.len() {
        if w[i] <= 0.0 || !cv_mask[i] {
            continue;
        }
        let h = inv_diag[i] * w[i];
        let denom = 1.0 - h;
        if denom <= 1e-12 {
            return None;
        }
        let r = (y[i] - z[i]) / denom;
        cv += w[i] * r * r;
    }
    Some(cv)
}

/// The λ grid for automatic selection: half-decade steps over
/// 10^(-2) … 10^(8). In cents²-per-curvature² units on an 88-key grid this
/// spans "follow every point" to "affine residual"; the endpoints are
/// deliberately beyond both useful extremes so the CV minimum is interior in
/// practice.
pub const LAMBDA_GRID_DECADES: (f64, f64, usize) = (-2.0, 8.0, 21);

/// Whittaker smoothing with λ selected by LOO-CV over
/// [`LAMBDA_GRID_DECADES`]. Returns the smoothed vector and the chosen
/// λ, or `None` when no grid point yields a valid CV score (e.g.
/// fewer than 3 observed points — too few to cross-validate a curve).
pub fn smooth_auto(y: &[f64], w: &[f64]) -> Option<(Vec<f64>, f64)> {
    let observed = w.iter().filter(|&&x| x > 0.0).count();
    if observed < 3 {
        return None;
    }
    let (lo, hi, steps) = LAMBDA_GRID_DECADES;
    let mut best: Option<(f64, f64)> = None; // (cv, lambda)
    for s in 0..steps {
        let exp = lo + (hi - lo) * s as f64 / (steps - 1) as f64;
        let lambda = 10f64.powf(exp);
        if let Some(cv) = cv(y, w, lambda)
            && best.is_none_or(|(bcv, _)| cv < bcv)
        {
            best = Some((cv, lambda));
        }
    }
    let (_, lambda) = best?;
    Some((smooth(y, w, lambda)?, lambda))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random stream for test fixtures (no rand dep).
    fn lcg(seed: &mut u64) -> f64 {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*seed >> 33) as f64 / (1u64 << 31) as f64) - 1.0
    }

    /// §11 test: banded Cholesky against a dense reference solve.
    #[test]
    #[allow(clippy::needless_range_loop)] // the dense reference reads clearest in index form
    fn test_banded_vs_dense() {
        let (n, hbw) = (30usize, 5usize);
        let mut seed = 42u64;
        // Random SPD system via normal equations of random sparse rows.
        let mut sys = BandedSystem::new(n, hbw);
        let mut dense = vec![vec![0.0f64; n + 1]; n]; // last col = rhs
        for _ in 0..200 {
            let i = ((lcg(&mut seed).abs() * n as f64) as usize).min(n - 1);
            let j = ((lcg(&mut seed).abs() * (hbw + 1) as f64) as usize).min(hbw);
            let j = if i >= j { i - j } else { i };
            let (a, b_) = (lcg(&mut seed) + 2.0, lcg(&mut seed));
            let t = lcg(&mut seed);
            sys.add_row(&[j, i], &[a, b_], t, 1.0);
            // Dense normal-equation accumulation.
            let (cols, coefs) = ([j, i], [a, b_]);
            for p in 0..2 {
                for q in 0..2 {
                    dense[cols[p]][cols[q]] += coefs[p] * coefs[q];
                }
                dense[cols[p]][n] += t * coefs[p];
            }
        }
        // Ridge to guarantee SPD in both.
        for i in 0..n {
            sys.add(i, i, 1e-3);
            dense[i][i] += 1e-3;
        }
        let x = sys.solve().expect("banded solve");
        // Dense Gaussian elimination with partial pivoting.
        let mut m = dense;
        for c in 0..n {
            let piv = (c..n)
                .max_by(|&a, &b| m[a][c].abs().total_cmp(&m[b][c].abs()))
                .unwrap();
            m.swap(c, piv);
            for r in c + 1..n {
                let f = m[r][c] / m[c][c];
                for k in c..=n {
                    m[r][k] -= f * m[c][k];
                }
            }
        }
        let mut xd = vec![0.0; n];
        for r in (0..n).rev() {
            let mut s = m[r][n];
            for k in r + 1..n {
                s -= m[r][k] * xd[k];
            }
            xd[r] = s / m[r][r];
        }
        for i in 0..n {
            assert!(
                (x[i] - xd[i]).abs() < 1e-8 * (1.0 + xd[i].abs()),
                "x[{i}] banded {} dense {}",
                x[i],
                xd[i]
            );
        }
    }

    /// §11 test: λ → 0 reproduces the input at observed points.
    #[test]
    fn test_whittaker_lambda_zero_limit() {
        let y: Vec<f64> = (0..20).map(|i| (i as f64 * 0.7).sin() * 5.0).collect();
        let w = vec![1.0; 20];
        let z = smooth(&y, &w, 1e-10).expect("solve");
        for i in 0..20 {
            assert!(
                (z[i] - y[i]).abs() < 1e-6,
                "z[{i}]={} y[{i}]={}",
                z[i],
                y[i]
            );
        }
    }

    /// §11 test: λ → ∞ tends to the weighted LS straight line (the penalty
    /// null space) — second differences vanish, and a pure line is
    /// reproduced exactly.
    #[test]
    fn test_whittaker_lambda_infinity_limit() {
        let n = 24;
        let line: Vec<f64> = (0..n).map(|i| 3.0 + 0.5 * i as f64).collect();
        let mut seed = 7u64;
        let noisy: Vec<f64> = line.iter().map(|v| v + lcg(&mut seed)).collect();
        let w = vec![1.0; n];
        // λ = 1e8: far past the CV-useful range but still within what the
        // f64 Cholesky resolves (condition ~λ; 1e12 would drown the data
        // term in solver noise).
        let z = smooth(&noisy, &w, 1e8).expect("solve");
        // Affine output: all second differences ~0.
        for i in 0..n - 2 {
            let d2 = z[i] - 2.0 * z[i + 1] + z[i + 2];
            assert!(d2.abs() < 1e-5, "curvature survives at λ→∞: {d2}");
        }
        // A pure line passes through untouched (zero penalty cost).
        let zl = smooth(&line, &w, 1e8).expect("solve");
        for i in 0..n {
            assert!((zl[i] - line[i]).abs() < 1e-4);
        }
    }

    /// Zero-weight gaps are interpolated, not zeroed.
    #[test]
    fn test_whittaker_interpolates_gaps() {
        let y = vec![0.0, 1.0, 0.0, 0.0, 4.0, 5.0]; // entries 2,3 unobserved
        let w = vec![1.0, 1.0, 0.0, 0.0, 1.0, 1.0];
        let z = smooth(&y, &w, 1.0).expect("solve");
        assert!(z[2] > 1.0 && z[2] < 4.0, "gap not interpolated: {}", z[2]);
        assert!(z[3] > z[2], "gap not monotone between neighbors");
    }

    /// §11 test: Eilers' fast LOO-CV identity against brute force — the fast
    /// residual (y_i − z_i)/(1 − h_ii) equals the true leave-one-out
    /// prediction error y_i − z^{(−i)}_i.
    #[test]
    fn test_loocv_vs_brute_force() {
        let n = 15;
        let mut seed = 99u64;
        let y: Vec<f64> = (0..n)
            .map(|i| (i as f64 * 0.4).cos() * 3.0 + lcg(&mut seed) * 0.3)
            .collect();
        let w = vec![1.0; n];
        let lambda = 5.0;

        // Fast score.
        let fast = cv(&y, &w, lambda).expect("cv");

        // Brute force: refit with each point removed.
        let mut brute = 0.0;
        for i in 0..n {
            let mut wi = w.clone();
            wi[i] = 0.0;
            let z = smooth(&y, &wi, lambda).expect("solve");
            let r = y[i] - z[i];
            brute += r * r;
        }
        assert!(
            (fast - brute).abs() < 1e-6 * (1.0 + brute),
            "fast {fast} brute {brute}"
        );
    }

    /// λ selection: smooth data + noise picks an interior λ and the smoothed
    /// curve beats the raw data against the noiseless truth.
    #[test]
    fn test_whittaker_auto_denoises() {
        let n = 60;
        let truth: Vec<f64> = (0..n).map(|i| (i as f64 * 0.15).sin() * 10.0).collect();
        let mut seed = 3u64;
        let noisy: Vec<f64> = truth.iter().map(|v| v + lcg(&mut seed) * 0.8).collect();
        let w = vec![1.0; n];
        let (z, lambda) = smooth_auto(&noisy, &w).expect("auto");
        let err_raw: f64 = truth.iter().zip(&noisy).map(|(t, v)| (t - v).powi(2)).sum();
        let err_smooth: f64 = truth.iter().zip(&z).map(|(t, v)| (t - v).powi(2)).sum();
        assert!(
            err_smooth < err_raw,
            "smoothing did not denoise: {err_smooth} vs {err_raw} (λ={lambda})"
        );
    }
}
