use super::{AnnotatedOutput, GroundMode, RasterOutput, balance_sources, valid_positive_value};
use crate::linalg::{multigrid::{MgOptions, MgPreconditioner}, operator::{FineOperator, GroundSpec, StencilOperator}, pcg::Preconditioner, timing, profile};

fn dot(a: &[f32], b: &[f32]) -> f64 {
    a.iter().zip(b).map(|(&a, &b)| a as f64 * b as f64).sum()
}

fn replace_residual(op: &StencilOperator<'_, f32>, b: &[f32], x: &[f64], r: &mut [f32], norm: f64) -> f64 {
    let mut sum = 0.0;
    for i in 0..b.len() {
        let value = b[i] as f64 - op.row_f64(i, x).0;
        r[i] = value as f32;
        sum += value * value;
    }
    if norm == 0.0 { sum.sqrt() } else { sum.sqrt() / norm }
}

/// f32 hierarchy and CG work vectors, f64 master voltage and reliable residual updates.
pub fn solve_raster_sources_mg_mixed_with_options(
    resistance: &[f32], nrows: usize, ncols: usize, nodata: f32,
    source: &mut [f32], ground: &[f32], max_iter: usize, tol: f64,
    remove_average: bool, ground_mode: GroundMode, options: MgOptions,
) -> AnnotatedOutput<RasterOutput<f64>> {
    solve_mixed(resistance, nrows, ncols, nodata, source, ground, max_iter, tol, remove_average, ground_mode, options, false)
}

pub fn solve_circuitscape_mixed(
    resistance: &[f32], nrows: usize, ncols: usize, nodata: f32,
    source: &mut [f32], ground: &[f32], max_iter: usize, tol: f64,
    ground_mode: GroundMode, options: MgOptions,
) -> AnnotatedOutput<RasterOutput<f64>> {
    solve_mixed(resistance, nrows, ncols, nodata, source, ground, max_iter, tol, true, ground_mode, options, true)
}

fn solve_mixed(
    resistance: &[f32], nrows: usize, ncols: usize, nodata: f32,
    source: &mut [f32], ground: &[f32], max_iter: usize, tol: f64,
    remove_average: bool, ground_mode: GroundMode, options: MgOptions, circuitscape: bool,
) -> AnnotatedOutput<RasterOutput<f64>> {
    let _profile = profile::Scope::new(options.profile);
    let timing_scope = timing::Scope::new(options.verbose);
    let preparation = timing::timer(None, "System preparation");
    let n = nrows * ncols;
    assert_eq!(source.len(), n);
    assert_eq!(ground.len(), n);
    for (i, value) in source.iter_mut().enumerate() {
        if !valid_positive_value(*value, nodata) || (circuitscape && !valid_positive_value(resistance[i], nodata)) { *value = 0.0; }
    }
    let grounded = ground.iter().any(|&g| valid_positive_value(g, nodata));
    let spec = match ground_mode {
        GroundMode::Shunt => GroundSpec::ShuntRaster(ground),
        GroundMode::Dirichlet => {
            let mask = super::build_dirichlet_mask_identity(ground, nodata);
            for (s, &pinned) in source.iter_mut().zip(&mask) {
                if pinned { *s = 0.0; }
            }
            GroundSpec::Dirichlet(mask)
        }
    };
    balance_sources(source, remove_average && !grounded);
    crate::memory::reset();
    crate::memory::record_inputs_typed::<f32>(n, true);
    crate::memory::record_mixed_cg_vectors(n);
    let fine = if circuitscape {
        StencilOperator::circuitscape(nrows, ncols, nodata, resistance, spec)
    } else {
        StencilOperator::new(nrows, ncols, nodata, resistance, spec)
    };
    drop(preparation);
    let hierarchy = timing::timer(None, "Hierarchy construction");
    let mg = MgPreconditioner::build_with_options(FineOperator::Stencil(fine), nrows, ncols, options);
    drop(hierarchy);
    let solve_timer = timing::timer(None, "CG solve (inclusive)");
    let cg_scope = timing::CgScope::new();
    let mut x = vec![0.0_f64; n];
    let mut r = source.to_vec();
    let mut z = vec![0.0_f32; n];
    let mut p = vec![0.0_f32; n];
    let mut product = vec![0.0_f32; n];
    let norm = dot(source, source).sqrt();
    let mut iterations = 0;
    if norm > 0.0 {
        if options.profile { profile::iteration(0); }
        mg.apply(&r, &mut z);
        p.copy_from_slice(&z);
        let mut rz = dot(&r, &z);
        for iter in 0..max_iter {
            iterations = iter + 1;
            if options.profile { profile::iteration(iterations); }
            let timer = timing::timer(None, "CG matrix-vector product");
            mg.fine_operator().matvec(&p, &mut product);
            drop(timer);
            let pap = dot(&p, &product);
            if !pap.is_finite() || pap <= 0.0 { break; }
            let alpha = rz / pap;
            for i in 0..n {
                x[i] += alpha * p[i] as f64;
                r[i] = (r[i] as f64 - alpha * product[i] as f64) as f32;
            }
            if options.profile { profile::mixed_sample(iterations, "CG update", None, mg.fine_stencil(), source, &x); }
            if dot(&r, &r).sqrt() / norm < tol {
                let timer = timing::timer(None, "Reliable residual update");
                let relative = replace_residual(mg.fine_stencil(), source, &x, &mut r, norm);
                drop(timer);
                if relative <= tol { break; }
                mg.apply(&r, &mut z);
                p.copy_from_slice(&z);
                rz = dot(&r, &z);
                continue;
            }
            mg.apply(&r, &mut z);
            let next = dot(&r, &z);
            if !next.is_finite() || rz <= 0.0 { break; }
            let beta = next / rz;
            for i in 0..n { p[i] = (z[i] as f64 + beta * p[i] as f64) as f32; }
            rz = next;
        }
    }
    cg_scope.iterations(iterations);
    drop(cg_scope);
    drop(solve_timer);
    let smoothing = timing::timer(Some(0), "Final post-smoothing");
    let omega = options.l0omega.unwrap_or(0.95);
    for sweep in 0..options.l0postnu {
        for i in 0..n {
            let (ax, diagonal) = mg.fine_stencil().row_f64(i, &x);
            product[i] = (omega * (source[i] as f64 - ax) / diagonal) as f32;
        }
        for i in 0..n { x[i] += product[i] as f64; }
        if options.profile { profile::mixed_sample(iterations, "final post-sweep", Some((sweep + 1, options.l0postnu)), mg.fine_stencil(), source, &x); }
    }
    drop(smoothing);
    let relative = replace_residual(mg.fine_stencil(), source, &x, &mut r, norm);
    let error = super::residual_error(relative, tol);
    drop((mg, r, z, p, product));
    let output_timer = timing::timer(None, "Current map/output construction");
    let current_map = super::current::compute_current_map_with_neighbours(
        resistance, ground, &x, nrows, ncols, nodata, ground_mode == GroundMode::Shunt, circuitscape,
    );
    let output = RasterOutput { voltages: x, current_map, nrows, ncols };
    drop(output_timer);
    timing_scope.print();
    AnnotatedOutput { output, total_iters: iterations, error }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circuitscape_connects_diagonal_cells_across_nodata() {
        let resistance = [10.0, f32::NAN, f32::NAN, 20.0];
        let mut source = [1.0, 5.0, 5.0, 0.0];
        let ground = [0.0, 0.0, 0.0, 1.0];
        let result = solve_circuitscape_mixed(
            &resistance, 2, 2, -9999.0, &mut source, &ground, 1000, 1e-7,
            GroundMode::Shunt, MgOptions::default(),
        );
        assert!(result.error.is_none(), "{:?}", result.error);
        assert!((result.output.voltages[0] - (1.0 + 15.0 * std::f64::consts::SQRT_2)).abs() < 1e-5);
        assert!((result.output.voltages[3] - 1.0).abs() < 1e-6);
        assert!((result.output.current_map[0] - 1.0).abs() < 1e-6);
        assert!((result.output.current_map[3] - 1.0).abs() < 1e-6);
        assert_eq!(result.output.current_map[1], 0.0);
        assert_eq!(result.output.current_map[2], 0.0);
        assert_eq!(source, [1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn circuitscape_nodata_padding_cannot_create_boundary_shortcuts() {
        let mut results = Vec::new();
        for padding in [0, 3] {
            let n = 6 + 2 * padding;
            let mut resistance = vec![f32::NAN; n * n];
            let mut source = vec![0.0; n * n];
            let mut ground = vec![0.0; n * n];
            for r in 0..6 {
                for c in 0..6 {
                    let i = (r + padding) * n + c + padding;
                    resistance[i] = if c == 2 && r < 5 { 1e8 } else { 10.0 + (r * c) as f32 };
                    if c == 0 { source[i] = 1.0; }
                }
            }
            ground[(2 + padding) * n + 5 + padding] = 1.0;
            let result = solve_circuitscape_mixed(
                &resistance, n, n, -9999.0, &mut source, &ground, 10000, 1e-7,
                GroundMode::Shunt, MgOptions::default(),
            );
            assert!(result.error.is_none(), "{:?}", result.error);
            let mut currents = Vec::new();
            for r in 0..6 { for c in 0..6 {
                currents.push(result.output.current_map[(r + padding) * n + c + padding]);
            }}
            results.push(currents);
        }
        for (a, b) in results[0].iter().zip(&results[1]) {
            assert!((a - b).abs() < 1e-5, "{a} != {b}");
        }
    }
}
