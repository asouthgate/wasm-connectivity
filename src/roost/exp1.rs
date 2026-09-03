//! Exponential integral E1(x) = integral_x^inf exp(-t)/t dt  (x > 0).
//!
//! Algorithm: Cephes `exp1` (`expn.c`, S. L. Moshier) — a Lentz continued
//! fraction for `x > 1` and the power-series expansion in `-gamma - ln(x)`
//! for `x <= 1`. `scipy.special.exp1` wraps the same algorithm. See:
//!
//!   * Moshier, S. L., *Methods and Programs for Mathematical Functions*,
//!     Prentice-Hall, 1989 (Cephes `expn.c`).
//!   * SciPy, `scipy/special/cephes/expn.c` (`scipy.special.exp1`).
//!
//! The test reference values are from `scipy.special.exp1`. Used for the
//! analytic time integral of the heat-diffusion kernel in the roost-location
//! model.

const EULER: f64 = 0.577_215_664_901_532_9;
const MAXIT: usize = 100;
const EPS: f64 = f64::EPSILON;
/// Smallest positive number representable as f64 (subnormal).
const FPMIN: f64 = f64::MIN_POSITIVE;

/// Evaluate E1(x) for `x > 0`.
///
/// For `x` so large that `exp(-x)` underflows, returns 0.0. For `x == 0`
/// the value is +inf (callers must handle the singular case themselves).
pub fn exp1(x: f64) -> f64 {
    debug_assert!(x >= 0.0, "exp1 defined for x > 0");
    if x == 0.0 {
        return f64::INFINITY;
    }
    if x > 1.0 {
        // Continued fraction: E1(x) = exp(-x) * cf(x).
        let mut b = x + 1.0;
        let mut c = 1.0 / FPMIN;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..=MAXIT {
            let a = -(i as f64) * (i as f64); // nm1 == 0
            b += 2.0;
            d = 1.0 / (a * d + b);
            c = b + a / c;
            let del = c * d;
            h *= del;
            if (del - 1.0).abs() < EPS {
                return h * (-x).exp();
            }
        }
        // Not converged; return best estimate.
        h * (-x).exp()
    } else {
        // Series: E1(x) = -gamma - ln(x) + sum_{k>=1} (-1)^(k-1) x^k / (k * k!)
        let mut ans = -x.ln() - EULER;
        let mut fact = 1.0;
        for i in 1..=MAXIT {
            fact *= -x / (i as f64);
            let del = -fact / (i as f64);
            ans += del;
            if del.abs() < ans.abs() * EPS {
                return ans;
            }
        }
        ans
    }
}

#[cfg(test)]
mod tests {
    use super::exp1;

    fn rel_err(a: f64, b: f64) -> f64 {
        ((a - b) / b).abs()
    }

    #[test]
    fn matches_known_values() {
        // Reference values from scipy.special.exp1.
        let cases = [
            (0.01, 4.037_929_576_538_113),
            (0.1, 1.822_923_958_419_390_6),
            (0.5, 0.559_773_594_776_160_8),
            (1.0, 0.219_383_934_395_520_5),
            (1.5, 0.100_019_582_406_632_65),
            (2.0, 0.048_900_510_708_061_125),
            (4.0, 0.003_779_352_409_848_906_3),
            (5.0, 0.001_148_295_591_275_325_7),
            (10.0, 4.156_968_929_685_325e-6),
        ];
        for (x, expected) in cases {
            let got = exp1(x);
            assert!(
                rel_err(got, expected) < 1e-10,
                "exp1({x}) = {got}, expected {expected}"
            );
        }
    }

    #[test]
    fn large_argument_underflows_to_zero() {
        assert_eq!(exp1(1000.0), 0.0);
        assert_eq!(exp1(1e7), 0.0);
    }
}
