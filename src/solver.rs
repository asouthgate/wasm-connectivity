use sprs::CsMat;

pub struct CgResult {
    pub x: Vec<f64>,
    pub iters: usize,
}

/// The preconditioner must implement apply,
/// which takes the residual vector r and produces the preconditioned vector z
/// which is carried through the solver computation.
pub trait Preconditioner {
    fn apply(&self, r: &[f64], z: &mut Vec<f64>);
}

/// Jacobi (diagonal) preconditioner: `z[i] = r[i] / |A[i,i]|`.
pub struct JacobiPreconditioner {
    diag_inv: Vec<f64>,
}

impl JacobiPreconditioner {
    pub fn new(a: &CsMat<f64>) -> Self {
        Self { diag_inv: crate::laplacian::extract_diag_inv(a) }
    }
}

impl Preconditioner for JacobiPreconditioner {
    fn apply(&self, r: &[f64], z: &mut Vec<f64>) {
        z.clear();
        z.reserve(r.len());
        for (&r_i, &m_inv) in r.iter().zip(self.diag_inv.iter()) {
            z.push(r_i * m_inv);
        }
    }
}

/// Solve the system Ax = b using the preconditioned conjugate gradient method
///
/// This is an iterative method similar to steepest descent. Instead of a 
/// sequence of orthogonal search directions, it uses conjugate directions.
/// This avoids zig-zagging during the search. In addition, this method uses
/// preconditioning. Preconditioning solves the transformed system M^-1 Ax = M^-1 b, 
/// where M is a matrix that approximates A but is also easy to invert.
///
/// # Arguments
/// * `a` - A reference to a `CsMat<f64>` representing the matrix A.
/// * `b` - A slice of f64 representing the right-hand side vector b.
/// * `max_iter` - The maximum number of iterations to perform.
/// * `tol` - The tolerance for convergence.
/// * `x0` - An optional initial guess for the solution vector x, otherwise taken to be zero.
/// * `precond` - A reference to a type implementing the `Preconditioner` trait.
pub fn cg_solve_precond(
    a: &CsMat<f64>,
    b: &[f64],
    max_iter: usize,
    tol: f64,
    x0: Option<&[f64]>,
    precond: &dyn Preconditioner,
) -> CgResult {

    let n = b.len();
    // set x0 to zero if not specified
    let mut x = match x0 {
        Some(seed) if seed.len() == n => seed.to_vec(),
        _ => vec![0.0f64; n],
    };

    // Compute the initial residual r = b - Ax0. If x0 is zero, then r = b.
    let mut r = vec![0.0; n];
    if x0.is_some() {
        let mut ax = vec![0.0; n];
        mat_vec_mul_into(a, &x, &mut ax);
        for i in 0..n { r[i] = b[i] - ax[i]; }
    } else {
        r.copy_from_slice(b);
    }

    // The norm of b is requried for convergence checks
    let b_norm = dot(b, b).sqrt();
    if b_norm < 1e-15 { return CgResult { x, iters: 0 }; }

    // z0 = M^-1 r0
    let mut z = vec![0.0; n];
    precond.apply(&r, &mut z);
    // p0 = z0
    let mut p = z.clone();
    let mut rs_old = dot(&r, &p);
    let mut ap = vec![0.0; n];

    let mut iters = 0;
    for iter in 0..max_iter {
        iters = iter + 1;
        mat_vec_mul_into(a, &p, &mut ap);
        let p_ap = dot(&p, &ap);
        // avoid dividing by zero
        if p_ap.abs() < 1e-30 { 
            eprintln!(
                "Warning: iter {}:p^T*L*p ({:.2e}) is close to zero. something bad has happened :(",
                iter, p_ap
            );            
            break;

        }

        // alpha = (r^T * z) / (p^T * A * p)
        let alpha = rs_old / p_ap;
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }

        let r_norm = dot(&r, &r).sqrt();

        if r_norm / b_norm < tol { break; } // ding

        precond.apply(&r, &mut z);

        let rs_new = dot(&r, &z);
        if rs_old.abs() < 1e-30 { 
            eprintln!(
                "Warning: iter {}: rs_old ({:.2e}) is close to zero. something bad has happened :(",
                iter, rs_old
            );            
            break;
        }

        // finally, update the search direction p
        let beta = rs_new / rs_old;
        for i in 0..n { 
            p[i] = z[i] + beta * p[i];
        }
        // update the old residual dot product for the next iteration
        rs_old = rs_new;
    }

    CgResult { x, iters }
}

/// Convenience wrapper — PCG with Jacobi preconditioning (original API).
pub fn cg_solve(
    a: &CsMat<f64>,
    b: &[f64],
    max_iter: usize,
    tol: f64,
    x0: Option<&[f64]>,
) -> CgResult {
    let j = JacobiPreconditioner::new(a);
    cg_solve_precond(a, b, max_iter, tol, x0, &j)
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

pub(crate) fn mat_vec_mul_into(a: &CsMat<f64>, v: &[f64], out: &mut Vec<f64>) {
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

pub(crate) fn mat_vec_mul_slice(a: &CsMat<f64>, v: &[f64], out: &mut [f64]) {
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