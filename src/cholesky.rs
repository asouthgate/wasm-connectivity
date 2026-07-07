//! Dense Cholesky factorization and solve for small SPD systems.
//!
//! Used at the coarsest level of the multigrid hierarchy where the system
//! is small enough (typically < 1000 unknowns) that a direct solve is
//! faster and more robust than an iterative method.

/// Compute the lower-triangular Cholesky factor L such that A = L·Lᵀ.
///
/// `a` is a symmetric positive-definite matrix in row-major order (n×n).
/// Returns `None` if the matrix is not SPD (non-positive pivot encountered).
pub fn cholesky_decompose(a: &[f64], n: usize) -> Option<Vec<f64>> {
    let mut l = vec![0.0; n * n];

    for i in 0..n {
        for j in 0..=i {
            let mut sum = 0.0;
            for k in 0..j {
                sum += l[i * n + k] * l[j * n + k];
            }
            if i == j {
                let val = a[i * n + i] - sum;
                if val <= 0.0 {
                    return None;
                }
                l[i * n + i] = val.sqrt();
            } else {
                let diag = l[j * n + j];
                if diag.abs() < 1e-15 {
                    return None;
                }
                l[i * n + j] = (a[i * n + j] - sum) / diag;
            }
        }
    }

    Some(l)
}

/// Solve L·y = b via forward substitution.
///
/// `l` is lower-triangular in row-major order (n×n).
fn forward_substitute(l: &[f64], b: &[f64], n: usize) -> Vec<f64> {
    let mut y = vec![0.0; n];
    for i in 0..n {
        let mut sum = 0.0;
        for j in 0..i {
            sum += l[i * n + j] * y[j];
        }
        let diag = l[i * n + i];
        y[i] = if diag.abs() < 1e-15 {
            0.0
        } else {
            (b[i] - sum) / diag
        };
    }
    y
}

/// Solve Lᵀ·x = y via back substitution.
///
/// `l` is lower-triangular in row-major order (n×n).
fn back_substitute(l: &[f64], y: &[f64], n: usize) -> Vec<f64> {
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut sum = 0.0;
        for j in (i + 1)..n {
            sum += l[j * n + i] * x[j];
        }
        let diag = l[i * n + i];
        x[i] = if diag.abs() < 1e-15 {
            0.0
        } else {
            (y[i] - sum) / diag
        };
    }
    x
}

/// Solve A·x = b using precomputed Cholesky factor L.
///
/// `l` is the lower-triangular factor from `cholesky_decompose` (n×n, row-major).
pub fn cholesky_solve(l: &[f64], b: &[f64], n: usize) -> Vec<f64> {
    let y = forward_substitute(l, b, n);
    back_substitute(l, &y, n)
}

/// Extract a dense n×n matrix from a sparse CSR matrix.
pub fn sparse_to_dense(mat: &sprs::CsMat<f64>, n: usize) -> Vec<f64> {
    let mut dense = vec![0.0; n * n];
    for (row_idx, row_view) in mat.outer_iterator().enumerate() {
        for (col, &val) in row_view.iter() {
            dense[row_idx * n + col] = val;
        }
    }
    dense
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cholesky_3x3() {
        // A = [[4, 2, 0], [2, 5, 3], [0, 3, 6]]
        let a = vec![
            4.0, 2.0, 0.0,
            2.0, 5.0, 3.0,
            0.0, 3.0, 6.0,
        ];
        let n = 3;
        let l = cholesky_decompose(&a, n).expect("SPD matrix should decompose");

        // Verify L·Lᵀ = A
        for i in 0..n {
            for j in 0..n {
                let mut sum = 0.0;
                for k in 0..n {
                    sum += l[i * n + k] * l[j * n + k];
                }
                assert!((sum - a[i * n + j]).abs() < 1e-10,
                    "L·Lᵀ[{},{}] = {} != A[{},{}] = {}", i, j, sum, i, j, a[i * n + j]);
            }
        }
    }

    #[test]
    fn test_cholesky_solve() {
        // A = [[4, 2], [2, 5]], b = [6, 7]
        let a = vec![4.0, 2.0, 2.0, 5.0];
        let b = vec![6.0, 7.0];
        let n = 2;

        let l = cholesky_decompose(&a, n).expect("SPD matrix should decompose");
        let x = cholesky_solve(&l, &b, n);

        // Verify A·x = b
        for i in 0..n {
            let mut sum = 0.0;
            for j in 0..n {
                sum += a[i * n + j] * x[j];
            }
            assert!((sum - b[i]).abs() < 1e-10,
                "A·x[{}] = {} != b[{}] = {}", i, sum, i, b[i]);
        }
    }

    #[test]
    fn test_non_spd_returns_none() {
        // Indefinite matrix
        let a = vec![1.0, 2.0, 2.0, 1.0];
        let n = 2;
        assert!(cholesky_decompose(&a, n).is_none());
    }
}
