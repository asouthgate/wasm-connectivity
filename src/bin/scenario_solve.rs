use std::{env, fs, path::Path};

use wasm_connect::raster::io::{parse_asc, write_asc};
use wasm_connect::solve::{solve_raster_sources_mg_stencil, GroundMode};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() != 1 || !matches!(args[0].as_str(), "dsm" | "linear_res" | "point_lights") {
        eprintln!("Usage: scenario-solve dsm|linear_res|point_lights");
        std::process::exit(2);
    }
    let directory = Path::new("scripts/scenarios").join(&args[0]);
    let resistance = parse_asc(directory.join("resistance.asc"));
    let source = parse_asc(directory.join("source.asc"));
    let ground = parse_asc(directory.join("ground.asc"));
    for grid in [&resistance, &source, &ground] {
        assert!(grid.nrows > 0 && grid.ncols > 0, "empty raster");
        assert_eq!(
            grid.data.len(),
            grid.nrows * grid.ncols,
            "incomplete raster"
        );
        assert_eq!(
            (
                grid.nrows,
                grid.ncols,
                grid.xllcorner,
                grid.yllcorner,
                grid.cellsize,
                grid.nodata
            ),
            (
                resistance.nrows,
                resistance.ncols,
                resistance.xllcorner,
                resistance.yllcorner,
                resistance.cellsize,
                resistance.nodata
            ),
            "rasters must align and use the same nodata value"
        );
    }
    assert!(
        source
            .data
            .iter()
            .any(|&v| v.is_finite() && v > 0.0 && v != source.nodata),
        "no sources"
    );
    assert!(
        ground
            .data
            .iter()
            .any(|&v| v.is_finite() && v > 0.0 && v != ground.nodata),
        "no grounds"
    );

    let mut result = solve_raster_sources_mg_stencil(
        &resistance.data,
        resistance.nrows,
        resistance.ncols,
        resistance.nodata,
        &source.data,
        &ground.data,
        100_000,
        1e-6,
        false,
        GroundMode::Shunt,
    );
    assert!(
        result.total_iters < 100_000,
        "CG reached the iteration limit"
    );
    assert!(
        result.output.voltages.iter().all(|v| v.is_finite()),
        "non-finite voltages"
    );
    for (value, &res) in result.output.current_map.iter_mut().zip(&resistance.data) {
        if !res.is_finite() || res <= 0.0 || res == resistance.nodata {
            *value = resistance.nodata;
        } else {
            assert!(value.is_finite(), "non-finite current");
        }
    }
    let output = directory.join("output");
    fs::create_dir_all(&output).expect("cannot create output directory");
    write_asc(
        output.join("rust_current.asc"),
        &result.output.current_map,
        resistance.nrows,
        resistance.ncols,
        resistance.xllcorner,
        resistance.yllcorner,
        resistance.cellsize,
        resistance.nodata,
    );
    eprintln!("{}: {} iterations", args[0], result.total_iters);
}
