use crate::current::linalg::{operator::StencilOperator, real::Real};

pub(crate) fn residual_error(relative: f64, tol: f64) -> Option<String> {
    if relative.is_finite() && relative <= tol { return None; }
    Some(format!("solve did not meet tolerance: true relative residual {relative:.6e}, requested {tol:.6e}. Use f64 or mixed precision instead of pure f32; for f64/mixed, increase --max-iter or relax --tol."))
}

pub(super) fn final_residual_error<F: Real>(op: &dyn crate::current::linalg::operator::Operator<F>, b: &[F], x: &[F], tol: F) -> Option<String> {
    let mut residual = 0.0_f64;
    let mut norm = 0.0_f64;
    for i in 0..b.len() {
        let mut ax = 0.0_f64;
        op.for_each_entry(i, &mut |j, a| { ax += a.to_f64().unwrap() * x[j].to_f64().unwrap(); });
        let b = b[i].to_f64().unwrap();
        residual += (b - ax).powi(2);
        norm += b * b;
    }
    let relative = if norm == 0.0 { residual.sqrt() } else { (residual / norm).sqrt() };
    residual_error(relative, tol.to_f64().unwrap())
}

pub(super) fn final_stencil_residual_error<F: Real>(op: &StencilOperator<'_, F>, b: &[F], x: &[F], tol: F) -> Option<String> {
    let mut residual = 0.0_f64;
    let mut norm = 0.0_f64;
    for i in 0..b.len() {
        let b = b[i].to_f64().unwrap();
        residual += (b - op.row_f64(i, x).0).powi(2);
        norm += b * b;
    }
    let relative = if norm == 0.0 { residual.sqrt() } else { (residual / norm).sqrt() };
    residual_error(relative, tol.to_f64().unwrap())
}
