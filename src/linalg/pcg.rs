#[cfg(test)]
use sprs::CsMat;

use crate::linalg::operator::Operator;

pub struct CgResult {
    pub v: Vec<f64>,
    pub iters: usize,
}

/// The preconditioner must implement apply,
/// which takes the residual vector r and produces the preconditioned vector e0
/// which is carried through the solver computation.
pub trait Preconditioner {
    fn apply(&self, r: &[f64], e0: &mut Vec<f64>);
}

/// Jacobi (diagonal) preconditioner: `e0[i] = r[i] / |L[i,i]|`.
pub struct JacobiPreconditioner {
    diag_inv: Vec<f64>,
}

impl JacobiPreconditioner {
    pub fn new(laplacian: &dyn Operator) -> Self {
        Self { diag_inv: laplacian.diag_inv() }
    }
}

impl Preconditioner for JacobiPreconditioner {
    fn apply(&self, r: &[f64], e0: &mut Vec<f64>) {
        e0.clear();
        e0.reserve(r.len());
        for (&r_i, &m_inv) in r.iter().zip(self.diag_inv.iter()) {
            e0.push(r_i * m_inv);
        }
    }
}

/// Solve the system Lv = s using the preconditioned conjugate gradient method
///
/// This is an iterative method similar to steepest descent. Instead of a 
/// sequence of orthogonal search directions, it uses conjugate directions.
/// This avoids zig-zagging during the search. In addition, this method uses
/// preconditioning. Preconditioning solves the transformed system M^-1 Lv = M^-1 s, 
/// where M is a matrix that approximates L but is also easy to invert.
///
/// # Arguments
/// * `laplacian` - A reference to an [`Operator`] representing the matrix L.
/// * `s` - A slice of f64 representing the right-hand side (source) vector s.
/// * `max_iter` - The maximum number of iterations to perform.
/// * `tol` - The tolerance for convergence.
/// * `v0` - An optional initial guess for the voltage v, otherwise taken to be zero.
/// * `precond` - A reference to a type implementing the `Preconditioner` trait.
pub fn cg_solve_precond(
    laplacian: &dyn Operator,
    s: &[f64],
    max_iter: usize,
    tol: f64,
    v0: Option<&[f64]>,
    precond: &dyn Preconditioner,
) -> CgResult {

    let n = s.len();
    // set v0 to zero if not specified
    let mut v = match v0 {
        Some(seed) if seed.len() == n => seed.to_vec(),
        _ => vec![0.0f64; n],
    };

    // Compute the initial residual r = s - Lv0. If v0 is zero, then r = s.
    let mut r = vec![0.0; n];
    if v0.is_some() {
        let mut lv = vec![0.0; n];
        laplacian.matvec(&v, &mut lv);
        for i in 0..n { r[i] = s[i] - lv[i]; }
    } else {
        r.copy_from_slice(s);
    }

    // The norm of s is required for convergence checks
    let s_norm = dot(s, s).sqrt();
    if s_norm < 1e-15 { return CgResult { v, iters: 0 }; }

    // e0 = M^-1 r
    let mut e0 = vec![0.0; n];
    precond.apply(&r, &mut e0);
    // p0 = e0
    let mut p = e0.clone();
    let mut rs_old = dot(&r, &p);
    let mut lp = vec![0.0; n];

    let mut iters = 0;
    for iter in 0..max_iter {
        iters = iter + 1;
        laplacian.matvec(&p, &mut lp);
        let p_lp = dot(&p, &lp);
        // avoid dividing by zero
        if p_lp.abs() < 1e-30 { 
            eprintln!(
                "Warning: iter {}:p^T*L*p ({:.2e}) is close to zero. PCG cannot continue",
                iter, p_lp
            );            
            break;

        }

        // alpha = (r^T * e0) / (p^T * L * p)
        let alpha = rs_old / p_lp;
        for i in 0..n {
            v[i] += alpha * p[i];
            r[i] -= alpha * lp[i];
        }

        let r_norm = dot(&r, &r).sqrt();

        if r_norm / s_norm < tol { break; }

        precond.apply(&r, &mut e0);

        let rs_new = dot(&r, &e0);
        if rs_old.abs() < 1e-30 { 
            eprintln!(
                "Warning: iter {}: rs_old ({:.2e}) is close to zero. PCG cannot continue",
                iter, rs_old
            );            
            break;
        }

        // finally, update the search direction p
        let beta = rs_new / rs_old;
        for i in 0..n { 
            p[i] = e0[i] + beta * p[i];
        }
        // update the old residual dot product for the next iteration
        rs_old = rs_new;
    }

    CgResult { v, iters }
}

/// Wrapper for Jacobi preconditioner.
pub fn cg_solve(
    laplacian: &dyn Operator,
    s: &[f64],
    max_iter: usize,
    tol: f64,
    v0: Option<&[f64]>,
) -> CgResult {
    let j = JacobiPreconditioner::new(laplacian);
    cg_solve_precond(laplacian, s, max_iter, tol, v0, &j)
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
pub(crate) fn mat_vec_mul_into(laplacian: &CsMat<f64>, v: &[f64], out: &mut Vec<f64>) {
    let n = laplacian.rows();
    out.clear();
    out.resize(n, 0.0);
    for (row, out_slot) in out.iter_mut().enumerate() {
        if let Some(rv) = laplacian.outer_view(row) {
            let mut acc = 0.0f64;
            for (col, &val) in rv.iter() {
                acc += val * v[col];
            }
            *out_slot = acc;
        }
    }
}

pub(crate) fn mat_vec_mul_slice(laplacian: &sprs::CsMat<f64>, v: &[f64], out: &mut [f64]) {
    for (row, out_slot) in out.iter_mut().enumerate() {
        if let Some(rv) = laplacian.outer_view(row) {
            let mut acc = 0.0f64;
            for (col, &val) in rv.iter() {
                acc += val * v[col];
            }
            *out_slot = acc;
        }
    }
}
