use lm_connect::linalg::{
    multigrid::{MgOptions, MgPreconditioner},
    operator::{FineOperator, GroundSpec, Operator, StencilOperator},
    real::{scalar, Real},
};
use lm_connect::solve::{self, GroundMode};

fn check_solver<F: Real>() {
    let (nr, nc) = (32, 32);
    let n = nr * nc;
    let nd = scalar::<F>(-9999.0);
    let mut resistance: Vec<F> = (0..n).map(|i| scalar(1.0 + (i % 17) as f64)).collect();
    resistance[3] = nd;
    resistance[5] = F::nan();
    resistance[7] = F::zero();
    let mut source = vec![F::zero(); n];
    source[0] = F::one();
    let mut ground = vec![F::zero(); n];
    for i in n - nc..n {
        ground[i] = scalar(0.25);
    }
    ground[1] = nd;
    ground[2] = F::nan();
    let options = MgOptions {
        l0postnu: 0,
        ..MgOptions::default()
    };
    for mode in [GroundMode::Shunt, GroundMode::Dirichlet] {
        for explicit in [false, true] {
            let solve = if explicit {
                solve::solve_raster_sources_mg_with_options::<F>
            } else {
                solve::solve_raster_sources_mg_stencil_with_options::<F>
            };
            let tol = scalar::<F>(1e-4);
            let result = solve(
                &resistance,
                nr,
                nc,
                nd,
                &source,
                &ground,
                1000,
                tol,
                false,
                mode,
                options,
            );
            assert!(result.total_iters < 1000);
            assert!(result
                .output
                .voltages
                .iter()
                .chain(&result.output.current_map)
                .all(|x| x.is_finite()));
            let spec = match mode {
                GroundMode::Shunt => GroundSpec::ShuntRaster(&ground),
                GroundMode::Dirichlet => GroundSpec::Dirichlet(
                    ground
                        .iter()
                        .map(|&v| v.is_finite() && v > F::zero())
                        .collect(),
                ),
            };
            let op = StencilOperator::new(nr, nc, nd, &resistance, spec);
            // Independently accumulate the equation in f64; solver storage and arithmetic remain F.
            let mut residual2 = 0.0;
            let mut rhs2 = 0.0;
            for row in 0..n {
                let d = if matches!(mode, GroundMode::Dirichlet)
                    && ground[row].is_finite()
                    && ground[row] > F::zero()
                {
                    0.0
                } else {
                    source[row].to_f64().unwrap()
                };
                let mut av = 0.0;
                op.for_each_entry(row, &mut |col, value| {
                    av += value.to_f64().unwrap() * result.output.voltages[col].to_f64().unwrap()
                });
                residual2 += (d - av).powi(2);
                rhs2 += d * d;
            }
            let relative = (residual2 / rhs2).sqrt();
            assert!(
                relative < 1e-4,
                "{} explicit={explicit}, true residual={relative}",
                std::any::type_name::<F>()
            );
        }
    }
    // Both preconditioner application paths retain and reuse caller-owned output storage.
    use lm_connect::linalg::pcg::Preconditioner;
    let fine = FineOperator::Stencil(StencilOperator::new(
        nr,
        nc,
        nd,
        &resistance,
        GroundSpec::ShuntRaster(&ground),
    ));
    let mg = MgPreconditioner::build_from_operator(fine, nr, nc, 8);
    let mut correction = vec![F::zero(); n];
    let ptr = correction.as_ptr();
    mg.apply(&source, &mut correction);
    mg.apply(&source, &mut correction);
    assert_eq!(ptr, correction.as_ptr());
}

#[test]
fn solver_f32() {
    check_solver::<f32>();
}
#[test]
fn solver_f64() {
    check_solver::<f64>();
}

#[test]
fn rasterization_keeps_f64_geometry_and_writes_f32_rasters() {
    let json = r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"layer":"buildings"},"geometry":{"type":"Polygon","coordinates":[[[286100,78550],[286100.002,78550],[286100.002,78550.002],[286100,78550.002],[286100,78550]]]}}]}"#;
    let params = r#"{"buildings":{"resistance":7.25,"width":0}}"#;
    let (a, masks_a, _) = lm_connect::geospatial::prepare_geospatial_layers(
        &[1.0_f32; 16],
        4,
        4,
        json,
        params,
        286100.0,
        78550.004,
        0.001,
    );
    let (b, masks_b, _) = lm_connect::geospatial::prepare_geospatial_layers(
        &[1.0_f64; 16],
        4,
        4,
        json,
        params,
        286100.0,
        78550.004,
        0.001,
    );
    assert_eq!(a.iter().filter(|&&v| v == 7.25).count(), 4);
    assert_eq!(a.iter().map(|&v| v as f64).collect::<Vec<_>>(), b);
    assert_eq!(
        masks_a[0]
            .data
            .iter()
            .map(|&v| v as f64)
            .collect::<Vec<_>>(),
        masks_b[0].data
    );
}

#[cfg(feature = "instrumentation-profile")]
#[test]
fn scalar_memory_accounting_counts_values_and_preserves_index_width() {
    fn profile<F: Real>() -> lm_connect::memory::InstrumentationProfile {
        let n = 32 * 32;
        solve::solve_raster_sources_mg_stencil_in_place_with_options(
            &vec![F::one(); n],
            32,
            32,
            scalar(-9999.0),
            &mut vec![F::one(); n],
            &vec![F::one(); n],
            100,
            scalar(1e-4),
            false,
            GroundMode::Shunt,
            MgOptions::default(),
        );
        lm_connect::memory::take_profile().unwrap()
    }
    let a = profile::<f32>();
    let b = profile::<f64>();
    assert_eq!(a.scalar_bytes, 4);
    assert_eq!(b.scalar_bytes, 8);
    assert_eq!(a.input_resistance_bytes * 2, b.input_resistance_bytes);
    assert_eq!(a.search_direction_p_bytes * 2, b.search_direction_p_bytes);
    for (a, b) in a.hierarchy.iter().zip(&b.hierarchy) {
        assert_eq!(a.matrix_values_bytes * 2, b.matrix_values_bytes);
        assert_eq!(a.matrix_indices_bytes, b.matrix_indices_bytes);
        assert_eq!(a.matrix_indptr_bytes, b.matrix_indptr_bytes);
        assert_eq!(a.workspace_d_prime_bytes * 2, b.workspace_d_prime_bytes);
    }
}

#[cfg(feature = "bin")]
#[test]
#[ignore = "full 2000x2000 scenario validation"]
fn full_scenario_residuals() {
    fn run<F: Real>(scenario: &str) {
        use lm_connect::raster::io::parse_asc_typed;
        let folder = format!("scripts/scenarios/{scenario}");
        let resistance = parse_asc_typed::<F, _>(format!("{folder}/resistance.asc"));
        let mut source = parse_asc_typed::<F, _>(format!("{folder}/source.asc"));
        let ground = parse_asc_typed::<F, _>(format!("{folder}/ground.asc"));
        let rhs = source.data.clone();
        let result = solve::solve_raster_sources_mg_stencil_in_place_with_options(
            &resistance.data,
            resistance.nrows,
            resistance.ncols,
            resistance.nodata,
            &mut source.data,
            &ground.data,
            100000,
            scalar(1e-4),
            false,
            GroundMode::Shunt,
            MgOptions::default(),
        );
        let op = StencilOperator::new(
            resistance.nrows,
            resistance.ncols,
            resistance.nodata,
            &resistance.data,
            GroundSpec::ShuntRaster(&ground.data),
        );
        let mut residual2 = 0.0;
        let mut rhs2 = 0.0;
        for row in 0..rhs.len() {
            let d = if rhs[row].is_finite() && rhs[row] > F::zero() && rhs[row] != source.nodata {
                rhs[row].to_f64().unwrap()
            } else {
                0.0
            };
            let mut av = 0.0;
            op.for_each_entry(row, &mut |col, value| {
                av += value.to_f64().unwrap() * result.output.voltages[col].to_f64().unwrap()
            });
            residual2 += (d - av).powi(2);
            rhs2 += d * d;
        }
        let relative = (residual2 / rhs2).sqrt();
        let mut applied = vec![F::zero(); rhs.len()];
        op.matvec(&result.output.voltages, &mut applied);
        let working_residual = (applied
            .iter()
            .zip(&rhs)
            .map(|(&a, &b)| (a - b).to_f64().unwrap().powi(2))
            .sum::<f64>()
            / rhs2)
            .sqrt();
        println!("working precision residual={working_residual:.8e}");
        println!(
            "{scenario} {}: iterations={}, true residual={relative:.8e}",
            std::any::type_name::<F>(),
            result.total_iters
        );
        assert!(result.total_iters < 100000);
        assert!(result.output.voltages.iter().all(|v| v.is_finite()));
    }
    for scenario in ["linear_res", "dsm", "point_lights"] {
        run::<f64>(scenario);
        run::<f32>(scenario);
    }
}

#[test]
fn exactly_converged_f32_has_no_residual_error() {
    let result = solve::solve_raster_sources_mg_stencil_with_options(
        &[1.0_f32],
        1,
        1,
        -9999.0,
        &[1.0],
        &[4.0],
        100,
        1e-10,
        false,
        GroundMode::Shunt,
        MgOptions::default(),
    );
    assert!(result.total_iters < 100);
    assert!(result.output.voltages.iter().all(|v| v.is_finite()));
    assert!(!serde_json::to_string(&result).unwrap().contains("\"error\""));
}

#[test]
fn rounding_accurate_voltages_to_f32_can_destroy_current_balance() {
    // Three cells in a chain, one unit injected at the left and a unit shunt
    // at the right. Exact voltages are [1 + 2R, 1 + R, 1].
    let resistance = [1e-6_f64; 3];
    let ground = [0.0, 0.0, 1.0];
    let rhs = [1.0, 0.0, 0.0];
    let op = StencilOperator::new(1, 3, -9999.0, &resistance, GroundSpec::ShuntRaster(&ground));
    let accurate = [1.000002_f64, 1.000001, 1.0];
    let rounded = accurate.map(|v| (v as f32) as f64);
    let residual = |v: &[f64]| {
        let mut norm2 = 0.0;
        for row in 0..3 {
            let mut applied = 0.0;
            op.for_each_entry(row, &mut |col, a| applied += a * v[col]);
            norm2 += (rhs[row] - applied).powi(2);
        }
        norm2.sqrt()
    };
    assert!(residual(&accurate) < 1e-8);
    assert!(residual(&rounded) > 0.1);
    // No CG, hierarchy or f32 matrix arithmetic was used in this comparison.
    let result = solve::solve_raster_sources_mg_stencil_with_options(
        &[1e-6_f32; 3], 1, 3, -9999.0, &[1.0, 0.0, 0.0], &[0.0, 0.0, 1.0],
        100, 1e-4, false, GroundMode::Shunt, MgOptions::default(),
    );
    assert!(result.total_iters < 100);
    assert!(result.output.voltages.iter().all(|v| v.is_finite()));
}

#[test]
fn mixed_master_voltage_meets_tolerance_where_pure_f32_cannot() {
    let resistance = [1e-6_f32; 3];
    let ground = [0.0, 0.0, 1.0];
    let options = MgOptions { l0postnu: 0, ..MgOptions::default() };
    let pure = solve::solve_raster_sources_mg_stencil_with_options(
        &resistance, 1, 3, -9999.0, &[1.0, 0.0, 0.0], &ground,
        1000, 1e-3, false, GroundMode::Shunt, options,
    );
    let error = pure.error.expect("f32 cannot represent the voltage differences");
    assert!(error.contains("Use f64 or mixed precision"));
    let mixed = solve::mixed::solve_raster_sources_mg_mixed_with_options(
        &resistance, 1, 3, -9999.0, &mut [1.0, 0.0, 0.0], &ground,
        1000, 1e-3, false, GroundMode::Shunt, options,
    );
    assert!(mixed.error.is_none(), "{:?}", mixed.error);
    for &current in &mixed.output.current_map { assert!((current - 1.0).abs() < 1e-3); }
}

#[test]
fn mixed_supports_both_ground_modes_and_reports_iteration_limit() {
    let (nr, nc) = (24, 25);
    let n = nr * nc;
    let resistance: Vec<f32> = (0..n).map(|i| 0.5 + (i % 13) as f32).collect();
    let mut ground = vec![0.0_f32; n];
    for g in &mut ground[n - nc..] { *g = 2.0; }
    let mut source = vec![0.0_f32; n];
    source[0] = 1.0;
    for mode in [GroundMode::Shunt, GroundMode::Dirichlet] {
        let result = solve::mixed::solve_raster_sources_mg_mixed_with_options(
            &resistance, nr, nc, -9999.0, &mut source.clone(), &ground,
            1000, 1e-3, false, mode, MgOptions::default(),
        );
        assert!(result.error.is_none(), "{mode:?}: {:?}", result.error);
        let result = solve::mixed::solve_raster_sources_mg_mixed_with_options(
            &resistance, nr, nc, -9999.0, &mut source.clone(), &ground,
            1, 1e-12, false, mode, MgOptions { l0postnu: 0, ..MgOptions::default() },
        );
        assert!(result.error.is_some());
    }
}
