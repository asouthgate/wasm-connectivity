use sprs::CsMat;

/// Result of a Conjugate Gradient solve.
pub struct CgResult {
    pub x: Vec<f64>,
    pub iters: usize,
}

impl CgResult {
    pub fn into_x(self) -> Vec<f64> {
        self.x
    }
}

/// Trait for PCG preconditioners. The implementor must approximate
/// `M⁻¹·r` where `M` is a preconditioning matrix close to `A`.
pub trait Preconditioner {
    fn apply(&self, r: &[f64], z: &mut Vec<f64>);
}

/// Jacobi (diagonal) preconditioner — `z[i] = r[i] / |A[i,i]|`.
pub struct JacobiPreconditioner {
    diag_inv: Vec<f64>,
}

impl JacobiPreconditioner {
    pub fn new(a: &CsMat<f64>) -> Self {
        let n = a.rows();
        let mut diag_inv = vec![0.0f64; n];
        for (row, d) in diag_inv.iter_mut().enumerate() {
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
        Self { diag_inv }
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

pub fn cg_solve_precond(
    a: &CsMat<f64>,
    b: &[f64],
    max_iter: usize,
    tol: f64,
    x0: Option<&[f64]>,
    precond: &dyn Preconditioner,
) -> CgResult {
    let n = b.len();
    let mut x = match x0 {
        Some(seed) if seed.len() == n => seed.to_vec(),
        _ => vec![0.0f64; n],
    };

    let mut r = vec![0.0; n];
    if x0.is_some() {
        let mut ax = vec![0.0; n];
        mat_vec_mul_into(a, &x, &mut ax);
        for i in 0..n { r[i] = b[i] - ax[i]; }
    } else {
        r.copy_from_slice(b);
    }

    let b_norm = dot(b, b).sqrt();
    if b_norm < 1e-15 { return CgResult { x, iters: 0 }; }

    let mut z = vec![0.0; n];
    precond.apply(&r, &mut z);
    let mut p = z.clone();
    let mut rs_old = dot(&r, &p);
    let mut ap = vec![0.0; n];

    let mut iters = 0; // Initialize outside the loop to fix scope error
    for iter in 0..max_iter {
        iters = iter + 1; // Update iteration count cleanly
        mat_vec_mul_into(a, &p, &mut ap);
        let p_ap = dot(&p, &ap);
        if p_ap.abs() < 1e-30 { break; }

        let alpha = rs_old / p_ap;
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }

        let r_norm = dot(&r, &r).sqrt();
        println!("Iteration {:4}: Residual Norm = {:.6e}, Relative = {:.6e}", 
            iter, r_norm, if b_norm > 1e-12 { r_norm / b_norm } else { r_norm }
        );

        if r_norm / b_norm < tol { break; }

        precond.apply(&r, &mut z);

        let rs_new = dot(&r, &z);
        if rs_old.abs() < 1e-30 { break; }

        let beta = rs_new / rs_old;
        for i in 0..n { p[i] = z[i] + beta * p[i]; }
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