//! The exact laws the gate studies compare against: the regularised incomplete
//! beta function, the Clopper–Pearson interval of a binomial proportion, and the
//! thresholds of the Rayleigh, finite-N cell-averaging and F laws at a stated
//! false-alarm probability.

/// Natural log of the gamma function, Lanczos approximation (Numerical Recipes
/// §6.1, g = 5, six terms), accurate to about 2e-10 for `x > 0`.
pub(super) fn ln_gamma(x: f64) -> f64 {
    const COEF: [f64; 6] = [
        76.180_091_729_471_46,
        -86.505_320_329_416_77,
        24.014_098_240_830_91,
        -1.231_739_572_450_155,
        0.120_865_097_386_617_9e-2,
        -0.539_523_938_495_3e-5,
    ];
    let mut y = x;
    let tmp = x + 5.5;
    let tmp = (x + 0.5) * tmp.ln() - tmp;
    let mut ser = 1.000_000_000_190_015;
    for c in COEF {
        y += 1.0;
        ser += c / y;
    }
    tmp + (2.506_628_274_631_000_5 * ser / x).ln()
}

/// Lentz's continued fraction for the incomplete beta function (Numerical
/// Recipes §6.4, `betacf`).
fn betacf(x: f64, a: f64, b: f64) -> f64 {
    const MAX_IT: usize = 500;
    const EPS: f64 = 1e-14;
    const FPMIN: f64 = 1e-300;
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < FPMIN {
        d = FPMIN;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=MAX_IT {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            break;
        }
    }
    h
}

/// Regularised incomplete beta function `I_x(a, b)`, the CDF of `Beta(a, b)` at
/// `x`, for `a, b > 0`.
pub(super) fn beta_cdf(x: f64, a: f64, b: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let ln_beta = ln_gamma(a) + ln_gamma(b) - ln_gamma(a + b);
    let front = (a * x.ln() + b * (1.0 - x).ln() - ln_beta).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        front * betacf(x, a, b) / a
    } else {
        1.0 - front * betacf(1.0 - x, b, a) / b
    }
}

/// Quantile of `Beta(a, b)` at probability `p`, by bisection on [`beta_cdf`].
pub(super) fn beta_quantile(p: f64, a: f64, b: f64) -> f64 {
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if beta_cdf(mid, a, b) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// Clopper–Pearson interval for a proportion with `k` successes in `n` trials at
/// confidence `1 − alpha`: the exact interval from the binomial's beta
/// representation (Clopper & Pearson 1934). `(0, 0)` when `n = 0`.
pub(super) fn clopper_pearson(k: u64, n: u64, alpha: f64) -> (f64, f64) {
    if n == 0 {
        return (0.0, 0.0);
    }
    let (k, n) = (k as f64, n as f64);
    let lo = if k == 0.0 {
        0.0
    } else {
        beta_quantile(alpha / 2.0, k, n - k + 1.0)
    };
    let hi = if k == n {
        1.0
    } else {
        beta_quantile(1.0 - alpha / 2.0, k + 1.0, n - k)
    };
    (lo, hi)
}

/// Median magnitude of a Rayleigh bin of mean power `mean_power`: `√(mean_power · ln 2)`.
pub(super) fn rayleigh_median(mean_power: f64) -> f64 {
    (mean_power * std::f64::consts::LN_2).sqrt()
}

/// Power-ratio threshold at `p_fa` for one exponential periodogram value against
/// the mean of `m` independent ones: the upper quantile of `F(2, 2m)`,
/// `m · (p_fa^(−1/m) − 1)`, which is the cell-averaging CFAR factor and the
/// Student-Rayleigh law of a floor estimated from `m` periodograms (Talbot &
/// Thrane 2020). Tends to `−ln p_fa` as `m → ∞`.
pub(super) fn f_2_2m_threshold(m: f64, p_fa: f64) -> f64 {
    m * (p_fa.powf(-1.0 / m) - 1.0)
}

/// The standard normal CDF `Φ(x)`, by composite Simpson quadrature of the density
/// from 0 to `|x|` over 4000 panels: an error below 10⁻¹¹ for `|x| ≤ 10`, where
/// the tail beyond is under 10⁻²³.
pub(super) fn normal_cdf(x: f64) -> f64 {
    const PANELS: usize = 4000;
    let a = x.abs().min(40.0);
    let h = a / PANELS as f64;
    let density = |t: f64| (-0.5 * t * t).exp();
    let mut sum = density(0.0) + density(a);
    for i in 1..PANELS {
        let weight = if i.is_multiple_of(2) { 2.0 } else { 4.0 };
        sum += weight * density(i as f64 * h);
    }
    let half = sum * h / 3.0 / (2.0 * std::f64::consts::PI).sqrt();
    if x >= 0.0 { 0.5 + half } else { 0.5 - half }
}

/// The standard normal quantile `Φ⁻¹(p)` for `0 < p < 1`, by bisection on
/// [`normal_cdf`].
pub(super) fn normal_quantile(p: f64) -> f64 {
    let (mut lo, mut hi) = (-40.0f64, 40.0f64);
    for _ in 0..100 {
        let mid = 0.5 * (lo + hi);
        if normal_cdf(mid) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// The trigamma function `ψ′(x)` for `x > 0`: the recurrence `ψ′(x) = ψ′(x + 1) +
/// 1/x²` up to `x ≥ 10`, then the asymptotic series `1/x + 1/(2x²) + 1/(6x³) −
/// 1/(30x⁵) + 1/(42x⁷)` (Abramowitz & Stegun 6.4.12). `ψ′(a)` is the variance of
/// `ln G` for `G ~ Gamma(a)`, the log-domain spread of a mean of `a` independent
/// exponential powers.
pub(super) fn trigamma(x: f64) -> f64 {
    let mut x = x;
    let mut sum = 0.0;
    while x < 10.0 {
        sum += 1.0 / (x * x);
        x += 1.0;
    }
    let x2 = x * x;
    sum + 1.0 / x + 1.0 / (2.0 * x2) + 1.0 / (6.0 * x2 * x) - 1.0 / (30.0 * x2 * x2 * x)
        + 1.0 / (42.0 * x2 * x2 * x2 * x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beta_cdf_matches_closed_forms() {
        // Beta(1, 1) is uniform; Beta(1, b) has CDF 1 − (1 − x)^b.
        assert!((beta_cdf(0.3, 1.0, 1.0) - 0.3).abs() < 1e-12);
        assert!((beta_cdf(0.3, 1.0, 4.0) - (1.0 - 0.7f64.powi(4))).abs() < 1e-12);
        assert!((beta_cdf(0.5, 2.0, 2.0) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn clopper_pearson_known_values() {
        // 0 of 100 at 95 %: upper bound 1 − 0.025^(1/100) = 0.0362.
        let (lo, hi) = clopper_pearson(0, 100, 0.05);
        assert_eq!(lo, 0.0);
        assert!((hi - 0.036_217).abs() < 1e-5, "{hi}");
        // 5 of 50: the textbook interval (0.0333, 0.2181).
        let (lo, hi) = clopper_pearson(5, 50, 0.05);
        assert!((lo - 0.033_28).abs() < 2e-4, "{lo}");
        assert!((hi - 0.218_14).abs() < 2e-4, "{hi}");
    }

    #[test]
    fn f_2_2m_limits() {
        // The finite-N factor converges on −ln p, and reproduces the F(2, 8)
        // point the harmonic F-test uses at K = 5 (18.49 at 1e-3).
        assert!((f_2_2m_threshold(1e9, 1e-3) - 6.9078).abs() < 1e-3);
        assert!((f_2_2m_threshold(4.0, 1e-3) - 18.49).abs() < 0.01);
        // The upper tail of F(2, 2m) at t is (1 + t/m)^(−m).
        let t = f_2_2m_threshold(20.8, 1e-3);
        assert!(((1.0 + t / 20.8f64).powf(-20.8) - 1e-3).abs() < 1e-9);
    }
}
