use sprs::CsMat;

pub fn cg_solve(
    a: &CsMat<f64>,
    b: &[f64],
    max_iter: usize,
    tol: f64,
) -> Vec<f64> {
    let n = b.len();
    let mut x = vec![0.0f64; n];
    let mut r: Vec<f64> = b.to_vec();

    let b_norm = dot(b, b).sqrt();
    if b_norm < 1e-15 {
        return x;
    }

    let precond = jacobi_preconditioner(a);

    let mut z = apply_preconditioner(&precond, &r);
    let mut p = z.clone();
    let mut rs_old = dot(&r, &z);

    for _iter in 0..max_iter {
        let ap = mat_vec_mul(a, &p);

        let p_ap = dot(&p, &ap);
        if p_ap.abs() < 1e-30 {
            break;
        }

        let alpha = rs_old / p_ap;

        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }

        let r_norm = dot(&r, &r).sqrt();
        if r_norm / b_norm < tol {
            break;
        }

        z = apply_preconditioner(&precond, &r);
        let rs_new = dot(&r, &z);

        if rs_old.abs() < 1e-30 {
            break;
        }

        let beta = rs_new / rs_old;

        for i in 0..n {
            p[i] = z[i] + beta * p[i];
        }
        rs_old = rs_new;
    }

    x
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

fn mat_vec_mul(a: &CsMat<f64>, v: &[f64]) -> Vec<f64> {
    let n = a.rows();
    let mut result = vec![0.0f64; n];
    for row in 0..n {
        if let Some(rv) = a.outer_view(row) {
            for (col, &val) in rv.iter() {
                result[row] += val * v[col];
            }
        }
    }
    result
}

fn jacobi_preconditioner(a: &CsMat<f64>) -> Vec<f64> {
    let n = a.rows();
    let mut diag = vec![0.0f64; n];
    for row in 0..n {
        if let Some(rv) = a.outer_view(row) {
            for (col, &val) in rv.iter() {
                if col == row {
                    diag[row] = 1.0 / val.max(1e-15);
                    break;
                }
            }
        }
    }
    diag
}

fn apply_preconditioner(precond_diag_inv: &[f64], r: &[f64]) -> Vec<f64> {
    r.iter()
        .zip(precond_diag_inv.iter())
        .map(|(&r_i, &m_inv)| r_i * m_inv)
        .collect()
}
