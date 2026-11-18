use crate::current::linalg::operator::{GroundSpec, StencilOperator};
use crate::current::linalg::multigrid::MgOptions;
use super::*;

#[test]
fn inplace_lowmem_inputs_preserve_filtering_and_current_output() {
    let (nr, nc, nodata) = (16, 16, 5.0);
    let resistance = vec![1.0; nr * nc];
    let mut source = vec![0.0; nr * nc];
    source[..6].copy_from_slice(&[1.0, f64::NAN, f64::INFINITY, -1.0, nodata, nodata + 5e-11]);
    let mut ground = vec![0.0; nr * nc];
    ground[8..12].copy_from_slice(&[f64::NAN, -2.0, nodata, nodata + 5e-11]);
    ground[nr * nc - 1] = 3.0;
    for mode in [GroundMode::Shunt, GroundMode::Dirichlet] {
        let expected = solve_raster_sources_mg(
            &resistance, nr, nc, nodata, &source, &ground, 10000, 1e-11, true, mode,
        );
        let actual = solve_raster_sources_mg_stencil_in_place_with_options(
            &resistance, nr, nc, nodata, &mut source.clone(), &ground, 10000, 1e-11,
            true, mode, MgOptions::default(),
        );
        for (a, b) in [
            (&actual.output.voltages, &expected.output.voltages),
            (&actual.output.current_map, &expected.output.current_map),
        ] {
            for (&a, &b) in a.iter().zip(b) {
                assert!((a - b).abs() < 1e-8, "{mode:?}: {a} != {b}");
            }
        }
    }
}

#[test]
fn grounded_solvers_match_uniform_grid_solution() {
    let (nr, nc) = (8, 10);
    for (r, injection, conductance) in [(0.5, 1.0, 0.25), (2.0, 2.5, 3.0)] {
        let resistance = vec![r; nr * nc];
        let mut source = vec![0.0; nr * nc];
        let mut ground = vec![0.0; nr * nc];
        for row in 0..nr {
            source[row * nc] = injection;
            ground[row * nc + nc - 1] = conductance;
        }
        for mode in [GroundMode::Shunt, GroundMode::Dirichlet] {
            let results = [
                ("mg", solve_raster_sources_mg(
                    &resistance, nr, nc, -9999.0, &source, &ground,
                    10000, 1e-11, true, mode,
                )),
                ("mg-lowmem", solve_raster_sources_mg_stencil(
                    &resistance, nr, nc, -9999.0, &source, &ground,
                    10000, 1e-11, true, mode,
                )),
            ];
            // Each row is the same series circuit: I*R per edge and I/G
            // at the shunt, or zero volts at a Dirichlet ground.
            let ground_voltage = if mode == GroundMode::Shunt {
                injection / conductance
            } else {
                0.0
            };
            for (solver, result) in results {
                for row in 0..nr {
                    for col in 0..nc {
                        let i = row * nc + col;
                        let expected = ground_voltage + injection * r * (nc - 1 - col) as f64;
                        assert!(
                            (result.output.voltages[i] - expected).abs() < 1e-7,
                            "{solver} {mode:?} R={r} I={injection} G={conductance} cell=({row},{col})"
                        );
                        assert!((result.output.current_map[i] - injection).abs() < 1e-7);
                    }
                }
            }
        }
    }
}

#[test]
fn stencil_solve_matches_explicit_outputs() {
    for (nrows, ncols) in [(3, 5), (15, 17), (16, 16)] {
        let n = nrows * ncols;
        let nodata = crate::NODATA_SENTINEL;
        let mut resistance: Vec<f64> = (0..n).map(|i| 0.5 + (i % 7) as f64).collect();
        resistance[ncols + 1] = nodata;
        resistance[ncols + 2] = f64::NAN;
        resistance[ncols + 3] = 0.0;
        let mut source = vec![0.0; n];
        let mut ground = vec![0.0; n];
        for row in 0..nrows {
            source[row * ncols] = 1.0;
            ground[row * ncols + ncols - 1] = 2.0;
        }
        ground[0] = 1.0; // Also exercise a pinned regularization node.
        for mode in [GroundMode::Shunt, GroundMode::Dirichlet] {
            for (grounded, remove_average) in [
                (true, true),
                (true, false),
                (false, true),
                (false, false),
            ] {
                let grounds = if grounded {
                    ground.clone()
                } else {
                    vec![0.0; n]
                };
                let explicit = solve_raster_sources_mg(
                    &resistance,
                    nrows,
                    ncols,
                    nodata,
                    &source,
                    &grounds,
                    5000,
                    1e-9,
                    remove_average,
                    mode,
                );
                let stencil = solve_raster_sources_mg_stencil(
                    &resistance,
                    nrows,
                    ncols,
                    nodata,
                    &source,
                    &grounds,
                    5000,
                    1e-9,
                    remove_average,
                    mode,
                );
                assert!(explicit.total_iters < 5000 && stencil.total_iters < 5000);
                let spec = match mode {
                    GroundMode::Shunt => GroundSpec::Shunt(grounds.clone()),
                    GroundMode::Dirichlet => GroundSpec::Dirichlet(
                        grounds.iter().map(|&value| value > 0.0).collect(),
                    ),
                };
                let operator = StencilOperator::new(nrows, ncols, nodata, &resistance, spec);
                let mut rhs = source.clone();
                if mode == GroundMode::Dirichlet {
                    for (value, &ground) in rhs.iter_mut().zip(&grounds) {
                        if ground > 0.0 {
                            *value = 0.0;
                        }
                    }
                }
                if !grounded && remove_average {
                    let mean = rhs.iter().sum::<f64>() / n as f64;
                    for value in &mut rhs {
                        *value -= mean;
                    }
                }
                let mut product = vec![0.0; n];
                for result in [&explicit, &stencil] {
                    crate::current::linalg::operator::Operator::matvec(
                        &operator,
                        &result.output.voltages,
                        &mut product,
                    );
                    let error = product
                        .iter()
                        .zip(&rhs)
                        .map(|(&value, &source)| (value - source).powi(2))
                        .sum::<f64>()
                        .sqrt();
                    let norm = rhs.iter().map(|value| value * value).sum::<f64>().sqrt();
                    assert!(
                        error / norm < 1e-7,
                        "true relative residual: {}",
                        error / norm
                    );
                }
                for (label, left, right) in [
                    (
                        "voltage",
                        &explicit.output.voltages,
                        &stencil.output.voltages,
                    ),
                    (
                        "current",
                        &explicit.output.current_map,
                        &stencil.output.current_map,
                    ),
                ] {
                    for (&a, &b) in left.iter().zip(right) {
                        assert!(a.is_finite() && b.is_finite());
                        assert!(
                            (a - b).abs() <= 1e-5 * (1.0 + a.abs()),
                            "{nrows}x{ncols} {mode:?} grounded={grounded} {label}: {a} vs {b}"
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn test_raster_edge_to_edge() {
    let size = 10;
    let n = size * size;
    let res_data = vec![1.0f64; n];
    let mut source_data = vec![0.0f64; n];
    let mut ground_data = vec![0.0f64; n];
    for row in 0..size {
        source_data[row * size] = 1.0;
        ground_data[row * size + (size - 1)] = 1.0;
    }
    let output = solve_raster_sources_mg(
        &res_data, size, size, crate::NODATA_SENTINEL, &source_data, &ground_data,
        crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, GroundMode::Shunt,
    ).output;
    assert_eq!(output.voltages.len(), n);
    assert_eq!(output.current_map.len(), n);
    let has_current = output.current_map.iter().any(|&v| v > 0.0);
    assert!(has_current, "Current map should have non-zero values");
    let has_voltage = output.voltages.iter().any(|&v| v.abs() > 0.0);
    assert!(has_voltage, "Voltage map should have non-zero values");
}

#[test]
fn test_raster_center_source() {
    let size = 10;
    let n = size * size;
    let res_data = vec![1.0f64; n];
    let mut source_data = vec![0.0f64; n];
    let mut ground_data = vec![0.0f64; n];
    source_data[5 * size + 5] = 1.0;
    for row in 0..size {
        for col in 0..size {
            if row == 0 || row == size - 1 || col == 0 || col == size - 1 {
                ground_data[row * size + col] = 1.0;
            }
        }
    }
    let output = solve_raster_sources_mg(
        &res_data, size, size, crate::NODATA_SENTINEL, &source_data, &ground_data,
        crate::DEFAULT_MAX_ITER, crate::DEFAULT_TOL, true, GroundMode::Shunt,
    ).output;
    let center_voltage = output.voltages[5 * size + 5];
    assert!(
        center_voltage > 0.0,
        "Center voltage should be positive for current source"
    );
}
