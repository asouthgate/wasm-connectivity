use crate::linalg::real::{Real, scalar};
use crate::linalg::operator::Operator;

/// Coefficients are cached only for the diagonal-based smoother. Explicit MG
/// retains its existing Gauss–Seidel update and damping.
pub(super) enum Smoother<F: Real = f64> {
    SymmetricGaussSeidel,
    WeightedJacobi { diag_inv: Vec<F>, omega: F },
}

impl<F: Real> Smoother<F> {
    pub(super) fn weighted_jacobi(op: &dyn Operator<F>, requested_omega: F) -> Self {
        let mut diag_inv = vec![scalar::<F>(0.0); op.n()];
        let mut upper_bound: F = scalar::<F>(0.0);
        for (row, inverse) in diag_inv.iter_mut().enumerate() {
            let mut diagonal: F = scalar::<F>(0.0);
            let mut abs_sum: F = scalar::<F>(0.0);
            op.for_each_entry(row, &mut |col, value| {
                abs_sum += value.abs();
                if col == row {
                    diagonal = value;
                }
            });
            if diagonal.abs() > scalar::<F>(1e-15) {
                *inverse = scalar::<F>(1.0) / diagonal.abs();
                upper_bound = upper_bound.max(abs_sum * *inverse);
            }
        }
        // Symmetric diagonal scaling has the same eigenvalues as D^-1 L,
        // but can give a much tighter row-sum bound when diagonals vary.
        let mut symmetric_bound: F = scalar::<F>(0.0);
        for row in 0..op.n() {
            let row_scale = diag_inv[row].sqrt();
            let mut abs_sum = scalar::<F>(0.0);
            op.for_each_entry(row, &mut |col, value| {
                abs_sum += value.abs() * row_scale * diag_inv[col].sqrt();
            });
            symmetric_bound = symmetric_bound.max(abs_sum);
        }
        upper_bound = upper_bound.min(symmetric_bound);
        // Both bounds are conservative. Keep omega*lambda_max <= 1.95 < 2,
        // leaving a 2.5% margin below the Jacobi stability boundary.
        let omega = requested_omega.min(scalar::<F>(1.95) / upper_bound.max(scalar::<F>(1.0)));
        Self::WeightedJacobi { diag_inv, omega }
    }

    pub(super) fn smooth(
        &self,
        op: &dyn Operator<F>,
        e: &mut [F],
        d: &[F],
        scratch: &mut [F],
        gs_omega: F,
    ) {
        match self {
            Self::SymmetricGaussSeidel => symmetric_gauss_seidel_smooth(op, e, d, gs_omega),
            Self::WeightedJacobi { diag_inv, omega } => {
                weighted_jacobi_smooth(op, e, d, diag_inv, *omega, scratch)
            }
        }
    }
}

/// One simultaneous weighted Jacobi sweep. The entire product uses the old e;
/// only after that product is complete are any entries of e updated.
pub(super) fn weighted_jacobi_smooth<F: Real>(
    op: &dyn Operator<F>,
    e: &mut [F],
    d: &[F],
    diag_inv: &[F],
    omega: F,
    scratch: &mut [F],
) {
    op.matvec(e, scratch);
    for i in 0..e.len() {
        e[i] += omega * diag_inv[i] * (d[i] - scratch[i]);
    }
}

pub(super) fn symmetric_gauss_seidel_smooth<F: Real>(op: &dyn Operator<F>, e: &mut [F], d: &[F], omega: F) {
    let n = d.len();
    // Forward sweep
    for row in 0..n {
        let mut diag = scalar::<F>(0.0);
        let mut off_diag_sum = scalar::<F>(0.0);
        op.for_each_entry(row, &mut |col, val| {
            if col == row {
                diag = val;
            } else {
                off_diag_sum += val * e[col];
            }
        });
        if diag.abs() > scalar::<F>(1e-15) {
            let new_e = (d[row] - off_diag_sum) / diag;
            e[row] = (scalar::<F>(1.0) - omega) * e[row] + omega * new_e;
        }
    }
    // Backward sweep
    for row in (0..n).rev() {
        let mut diag = scalar::<F>(0.0);
        let mut off_diag_sum = scalar::<F>(0.0);
        op.for_each_entry(row, &mut |col, val| {
            if col == row {
                diag = val;
            } else {
                off_diag_sum += val * e[col];
            }
        });
        if diag.abs() > scalar::<F>(1e-15) {
            let new_e = (d[row] - off_diag_sum) / diag;
            e[row] = (scalar::<F>(1.0) - omega) * e[row] + omega * new_e;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jacobi_matches_classical_out_of_place_dense_iteration() {
        // Mixed-sign off-diagonals also exercise the positive couplings that
        // can occur in coarse Galerkin matrices.
        let a = [
            [7.0, -2.0, 1.0, 0.5],
            [-2.0, 5.0, 0.75, -0.25],
            [1.0, 0.75, 4.0, -1.0],
            [0.5, -0.25, -1.0, 3.0],
        ];
        let mut triplets = sprs::TriMat::new((4, 4));
        for i in 0..4 {
            for j in 0..4 {
                triplets.add_triplet(i, j, a[i][j]);
            }
        }
        let op = triplets.to_csr::<usize>();
        let rhs = [2.0, -3.0, 0.0, 1.5];
        for requested in [0.0, 0.4, 0.975, 1.0] {
            let smoother = Smoother::weighted_jacobi(&op, requested);
            let Smoother::WeightedJacobi { omega, ref diag_inv } = smoother else {
                unreachable!()
            };
            for i in 0..4 {
                assert_eq!(diag_inv[i], 1.0 / a[i][i]);
            }
            let mut actual = [1.0, -0.5, 2.0, 0.25];
            let mut expected = actual;
            let mut scratch = [f64::NAN; 4];
            for sweep in 0..12 {
                let old = expected;
                for i in 0..4 {
                    let off_diagonal: f64 = (0..4).filter(|&j| j != i)
                        .map(|j| a[i][j] * old[j]).sum();
                    expected[i] = (1.0 - omega) * old[i]
                        + omega * (rhs[i] - off_diagonal) / a[i][i];
                }
                scratch.fill(f64::NAN);
                smoother.smooth(&op, &mut actual, &rhs, &mut scratch, 0.67);
                for i in 0..4 {
                    assert!((actual[i] - expected[i]).abs()
                        <= 32.0 * f64::EPSILON * (1.0 + expected[i].abs()),
                        "omega={omega}, sweep={sweep}, row={i}: {} != {}", actual[i], expected[i]);
                }
            }
            // A solution of A e = d must remain fixed under smoothing.
            let solution = [2.0, -1.0, 0.5, 3.0];
            let rhs: Vec<f64> = a.iter().map(|row|
                row.iter().zip(solution).map(|(a, x)| a * x).sum()).collect();
            let mut actual = solution;
            smoother.smooth(&op, &mut actual, &rhs, &mut scratch, 0.67);
            assert_eq!(actual, solution);
        }
    }
}

