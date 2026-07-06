use sprs::CsMat;

pub fn cg_solve(a: &CsMat<f64>, b: &[f64], max_iter: usize, tol: f64) -> Vec<f64> {
    let n = b.len();
    let mut x = vec![0.0f64; n];
    let mut r: Vec<f64> = b.to_vec();

    let b_norm = dot(b, b).sqrt();
    if b_norm < 1e-15 {
        return x;
    }

    let precond = jacobi_preconditioner(a);

    let mut z = Vec::with_capacity(n);
    apply_preconditioner_into(&precond, &r, &mut z);
    let mut p = z.clone();
    let mut rs_old = dot(&r, &p);
    let mut ap = Vec::with_capacity(n);

    for _iter in 0..max_iter {
        mat_vec_mul_into(a, &p, &mut ap);

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

        apply_preconditioner_into(&precond, &r, &mut z);
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

fn jacobi_preconditioner(a: &CsMat<f64>) -> Vec<f64> {
    let n = a.rows();
    let mut diag = vec![0.0f64; n];
    for (row, d) in diag.iter_mut().enumerate() {
        if let Some(rv) = a.outer_view(row) {
            for (col, &val) in rv.iter() {
                if col == row {
                    let abs_val = val.abs();
                    *d = if abs_val > 1e-15 { 1.0 / abs_val } else { 0.0 };
                    break;
                }
            }
        }
    }
    diag
}

fn apply_preconditioner_into(precond_diag_inv: &[f64], r: &[f64], out: &mut Vec<f64>) {
    out.clear();
    out.reserve(r.len());
    for (&r_i, &m_inv) in r.iter().zip(precond_diag_inv.iter()) {
        out.push(r_i * m_inv);
    }
}

fn mat_vec_mul_into(a: &CsMat<f64>, v: &[f64], out: &mut Vec<f64>) {
    let n = a.rows();
    out.clear();
    out.resize(n, 0.0);
    for (row, out_slot) in out.iter_mut().enumerate() {
        if let Some(rv) = a.outer_view(row) {
            let mut acc = 0.0f64;
            for (col, &val) in rv.iter() {
                acc += val * v[col];
            }
            *out_slot = acc;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::laplacian;
    use crate::graph;

    #[test]
    fn test_cg_solve_simple_circuit() {
        let mut edges = graph::EdgeTriplets::new();
        edges.push(0, 1, 1.0);
        edges.push(1, 0, 1.0);
        let a = laplacian::build_laplacian(&edges, 2);
        let b = vec![1.0, -1.0];
        let x = cg_solve(&a, &b, 1000, 1e-10);
        assert!((x[0] - 0.5).abs() < 1e-4, "got {}", x[0]);
        assert!((x[1] + 0.5).abs() < 1e-4, "got {}", x[1]);
    }

    #[test]
    fn test_cg_solve_zero_rhs() {
        let mut edges = graph::EdgeTriplets::new();
        edges.push(0, 1, 1.0);
        edges.push(1, 0, 1.0);
        let a = laplacian::build_laplacian(&edges, 2);
        let x = cg_solve(&a, &[0.0, 0.0], 1000, 1e-10);
        assert_eq!(x[0], 0.0);
        assert_eq!(x[1], 0.0);
    }
}
