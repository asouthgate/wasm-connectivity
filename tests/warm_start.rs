use std::fs;
use std::time::Instant;

mod common;

use wasm_connect::solve;
use wasm_connect::solve::GroundMode;

const DATA_DIR: &str = "example/public/geodata";
const OUT_DIR: &str = "tests/output";

#[test]
#[ignore]
fn compute_all_current_maps() {
    fs::create_dir_all(OUT_DIR).unwrap();

    let base = common::parse_asc(&format!("{DATA_DIR}/base_resistance_500.asc"));
    let src = common::parse_asc(&format!("{DATA_DIR}/source_500.asc"));
    let gnd = common::parse_asc(&format!("{DATA_DIR}/ground_500.asc"));
    let geojson = fs::read_to_string(format!("{DATA_DIR}/all_features_500.geojson")).unwrap();

    let layer_params_str =
        r#"{"roads":{"resistance":50,"width":3},"rivers":{"resistance":0.5,"width":4},"buildings":{"resistance":500,"width":0}}"#;

    let (resistance, _, warnings) = wasm_connect::geospatial::prepare_geospatial_layers(
        &base.data, base.nrows, base.ncols,
        &geojson, layer_params_str,
        base.xllcorner, base.ymax, base.cellsize,
    );
    for w in &warnings {
        eprintln!("[rasterize warn] {w}");
    }

    let out_nodata = -9999.0;

    common::write_asc(
        &format!("{OUT_DIR}/resistance_with_building.asc"),
        &resistance, base.nrows, base.ncols,
        base.xllcorner, base.yllcorner, base.cellsize, out_nodata,
    );

    let modes = [(GroundMode::Neumann, "neumann"), (GroundMode::Dirichlet, "dirichlet")];

    for &(ground_mode, suffix) in &modes {
        // ---- Jacobi-preconditioned CG (original, component-based) ----
        let t0 = Instant::now();
        wasm_connect::cache::reset();
        let jacobi = solve::solve_raster_cached(
            &resistance, base.nrows, base.ncols, base.nodata,
            &src.data, &gnd.data, 100_000, 1e-6, true, false, ground_mode,
        );
        let t_jacobi = t0.elapsed();
        common::write_asc(
            &format!("{OUT_DIR}/current_map_jacobi_{suffix}.asc"),
            &jacobi.output.current_map, base.nrows, base.ncols,
            base.xllcorner, base.yllcorner, base.cellsize, out_nodata,
        );
        common::asc_to_png(
            &format!("{OUT_DIR}/current_map_jacobi_{suffix}.asc"),
            &format!("{OUT_DIR}/current_map_jacobi_{suffix}.png"),
        );

        // ---- MG-preconditioned CG (bilinear prolongation) ----
        let t1 = Instant::now();
        let mg = solve::solve_raster_sources_mg(
            &resistance, base.nrows, base.ncols, base.nodata,
            &src.data, &gnd.data, 100_000, 1e-6, true, ground_mode,
        );
        let t_mg = t1.elapsed();
        common::write_asc(
            &format!("{OUT_DIR}/current_map_mg_{suffix}.asc"),
            &mg.output.current_map, base.nrows, base.ncols,
            base.xllcorner, base.yllcorner, base.cellsize, out_nodata,
        );
        common::asc_to_png(
            &format!("{OUT_DIR}/current_map_mg_{suffix}.asc"),
            &format!("{OUT_DIR}/current_map_mg_{suffix}.png"),
        );

        // ---- MG-preconditioned CG (Alcouffe matrix-dependent prolongation) ----
        let t2 = Instant::now();
        let mg_alc = solve::solve_raster_sources_mg_alcouffe(
            &resistance, base.nrows, base.ncols, base.nodata,
            &src.data, &gnd.data, 100_000, 1e-6, true, ground_mode,
        );
        let t_mg_alc = t2.elapsed();
        common::write_asc(
            &format!("{OUT_DIR}/current_map_mg_alcouffe_{suffix}.asc"),
            &mg_alc.output.current_map, base.nrows, base.ncols,
            base.xllcorner, base.yllcorner, base.cellsize, out_nodata,
        );
        common::asc_to_png(
            &format!("{OUT_DIR}/current_map_mg_alcouffe_{suffix}.asc"),
            &format!("{OUT_DIR}/current_map_mg_alcouffe_{suffix}.png"),
        );
        common::write_asc(
            &format!("{OUT_DIR}/voltage_map_mg_alcouffe_{suffix}.asc"),
            &mg_alc.output.voltages, base.nrows, base.ncols,
            base.xllcorner, base.yllcorner, base.cellsize, out_nodata,
        );

        // MG bilinear and Alcouffe share the same filled Laplacian — they must agree
        let n = mg.output.current_map.len();
        let mut max_diff = 0.0f64;
        for i in 0..n {
            let diff = (mg.output.current_map[i] - mg_alc.output.current_map[i]).abs();
            if diff > max_diff {
                max_diff = diff;
            }
        }
        assert!(
            max_diff < 1e-3,
            "MG bilinear vs Alcouffe diverge ({suffix}): max_diff={max_diff}"
        );

        // All three solvers must converge
        assert!(jacobi.total_iters < 100_000, "Jacobi CG did not converge ({suffix}, {} iters)", jacobi.total_iters);
        assert!(mg.total_iters < 100_000, "MG-bilinear CG did not converge ({suffix}, {} iters)", mg.total_iters);
        assert!(mg_alc.total_iters < 100_000, "MG-Alcouffe CG did not converge ({suffix}, {} iters)", mg_alc.total_iters);

        println!();
        println!("===== {suffix} =====");
        println!("Jacobi CG:      {:>8} ms  {:>6} iters", t_jacobi.as_millis(), jacobi.total_iters);
        println!("MG bilinear:    {:>8} ms  {:>6} iters", t_mg.as_millis(), mg.total_iters);
        println!("MG Alcouffe:    {:>8} ms  {:>6} iters", t_mg_alc.as_millis(), mg_alc.total_iters);
    }
    println!();
}
