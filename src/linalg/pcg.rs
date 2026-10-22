use crate::linalg::real::{Real, scalar};
#[cfg(test)]
use sprs::CsMat;

use crate::linalg::operator::Operator;
use crate::linalg::{timing, profile};

pub struct CgResult<F: Real = f64> {
    pub v: Vec<F>,
    pub iters: usize,
}

/// The preconditioner must implement apply,
/// which takes the residual vector r and produces the preconditioned vector e0
/// which is carried through the solver computation.
pub trait Preconditioner<F: Real = f64> {
    fn apply(&self, r: &[F], e0: &mut Vec<F>);
}

/// Jacobi (diagonal) preconditioner: `e0[i] = r[i] / |L[i,i]|`.
pub struct JacobiPreconditioner<F: Real = f64> {
    diag_inv: Vec<F>,
}

impl<F: Real> JacobiPreconditioner<F> {
    pub fn new(laplacian: &dyn Operator<F>) -> Self {
        Self { diag_inv: laplacian.diag_inv() }
    }
}

impl<F: Real> Preconditioner<F> for JacobiPreconditioner<F> {
    fn apply(&self, r: &[F], e0: &mut Vec<F>) {
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
/// * `precond` - A reference to a type implementing the `Preconditioner` trait.
pub fn cg_solve_precond<F: Real>(
    laplacian: &dyn Operator<F>,
    s: &[F],
    max_iter: usize,
    tol: F,
    precond: &dyn Preconditioner<F>,
) -> CgResult<F> {
    cg_solve_precond_impl::<F, false>(laplacian, s, max_iter, tol, precond)
}

pub(crate) fn cg_solve_precond_profiled<F: Real>(
    laplacian: &dyn Operator<F>, s: &[F], max_iter: usize, tol: F,
    precond: &dyn Preconditioner<F>,
) -> CgResult<F> {
    cg_solve_precond_impl::<F, true>(laplacian, s, max_iter, tol, precond)
}

fn cg_solve_precond_impl<F: Real, const PROFILE: bool>(
    laplacian: &dyn Operator<F>,
    s: &[F],
    max_iter: usize,
    tol: F,
    precond: &dyn Preconditioner<F>,
) -> CgResult<F> {

    let timing_scope = timing::CgScope::new();
    let mut phase_timer = timing::timer_if(timing_scope.outer, None, "CG vector work/reductions");
    let n = s.len();
    let mut v = vec![scalar::<F>(0.0); n];
    let mut r = s.to_vec();

    // The norm of s is required for convergence checks
    let s_norm = dot(s, s).sqrt();
    if PROFILE {
        phase_timer.exclude(|| profile::global_sample(0, "CG initial", laplacian, s, &v));
    }
    if s_norm < scalar::<F>(1e-15) { return CgResult::<F> { v, iters: 0 }; }

    // e0 = M^-1 r
    let mut e0 = vec![scalar::<F>(0.0); n];
    drop(phase_timer);
    let phase_timer = timing::timer_if(timing_scope.outer, None, "Preconditioner (inclusive)");
    if PROFILE { profile::iteration(1); }
    precond.apply(&r, &mut e0);
    drop(phase_timer);
    let phase_timer = timing::timer_if(timing_scope.outer, None, "CG vector work/reductions");
    // p0 = e0
    let mut p = e0.clone();
    let mut rs_old = dot(&r, &p);
    let mut lp = vec![scalar::<F>(0.0); n];

    drop(phase_timer);
    let mut iters = 0;
    for iter in 0..max_iter {
        iters = iter + 1;
        let phase_timer = timing::timer_if(timing_scope.outer, None, "CG matrix-vector product");
        laplacian.matvec(&p, &mut lp);
        drop(phase_timer);
        let mut phase_timer = timing::timer_if(timing_scope.outer, None, "CG vector work/reductions");
        let p_lp = dot(&p, &lp);
        // avoid dividing by zero
        if p_lp.abs() < scalar::<F>(1e-30) {
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
        if PROFILE {
            phase_timer.exclude(|| profile::global_sample(iter + 1, "CG update", laplacian, s, &v));
        }

        if r_norm / s_norm < tol { break; }

        drop(phase_timer);
        let phase_timer = timing::timer_if(timing_scope.outer, None, "Preconditioner (inclusive)");
        if PROFILE { profile::iteration(iter + 2); }
        precond.apply(&r, &mut e0);
        drop(phase_timer);
        let _phase_timer = timing::timer_if(timing_scope.outer, None, "CG vector work/reductions");

        let rs_new = dot(&r, &e0);
        if rs_old.abs() < scalar::<F>(1e-30) {
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

    timing_scope.iterations(iters);
    CgResult::<F> { v, iters }
}

/// Wrapper for Jacobi preconditioner.
pub fn cg_solve<F: Real>(
    laplacian: &dyn Operator<F>,
    s: &[F],
    max_iter: usize,
    tol: F,
) -> CgResult<F> {
    let j = JacobiPreconditioner::new(laplacian);
    cg_solve_precond(laplacian, s, max_iter, tol, &j)
}

fn dot<F: Real>(a: &[F], b: &[F]) -> F {
    a.iter().zip(b.iter()).map(|(&x, &y)| x * y).sum()
}

#[cfg(test)]
pub(crate) fn mat_vec_mul_into<F: Real>(laplacian: &CsMat<F>, v: &[F], out: &mut Vec<F>) {
    let n = laplacian.rows();
    out.clear();
    out.resize(n, scalar::<F>(0.0));
    for (row, out_slot) in out.iter_mut().enumerate() {
        if let Some(rv) = laplacian.outer_view(row) {
            let mut acc = scalar::<F>(0.0);
            for (col, &val) in rv.iter() {
                acc += val * v[col];
            }
            *out_slot = acc;
        }
    }
}

pub(crate) fn mat_vec_mul_slice<F: Real>(laplacian: &sprs::CsMat<F>, v: &[F], out: &mut [F]) {
    for (row, out_slot) in out.iter_mut().enumerate() {
        if let Some(rv) = laplacian.outer_view(row) {
            let mut acc = scalar::<F>(0.0);
            for (col, &val) in rv.iter() {
                acc += val * v[col];
            }
            *out_slot = acc;
        }
    }
}
