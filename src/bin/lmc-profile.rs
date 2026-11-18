//! Profile one connectivity solve and emit one JSON object on stdout.
//!
//! Usage:
//!   cargo run --profile release-prof --features developer \
//!       --bin lmc-profile -- 500 --solver mg-lowmem --ground shunt
//! Diagnostics go to stderr.

use geojson::{FeatureCollection, GeoJson};
use lm_connect::geospatial::{rasterize_values, GeoTransform};
use lm_connect::linalg::real::{scalar, Real};
use std::fs;

use serde::Serialize;

use lm_connect::linalg::multigrid::MgOptions;
use lm_connect::pipeline::{self, ExecutionOptions, Grid, OutputLayer, PipelineData};
use lm_connect::raster::io::{parse_asc_typed, AscGrid};
use lm_connect::memory::{take_profile, InstrumentationProfile};
use lm_connect::solve::{self, GroundMode};

const DATA_DIR: &str = "example/public/geodata";
const MODEL_PATH: &str = "example/chudleigh-model.json";

#[derive(Serialize)]
struct ProfileRecord {
    accounting_scope: &'static str,
    resolution: usize,
    source_resolution: usize,
    solver: String,
    ground: String,
    precision: String,
    total_iters: usize,
    profile: Option<InstrumentationProfile>,
}

fn emit(resolution: usize, solver: &str, ground: &str, precision: &str, total_iters: usize) {
    let rec = ProfileRecord {
        accounting_scope: "Native solver payloads; low-memory solver reuses the source input as RHS. Excludes rasterization, caller extras and WASM/JSON wrapper storage.",
        resolution,
        source_resolution: resolution.min(1000),
        solver: solver.to_string(),
        ground: ground.to_string(),
        precision: precision.into(),
        total_iters,
        profile: take_profile(),
    };
    println!("{}", serde_json::to_string_pretty(&rec).unwrap());
}

const USAGE: &str = "Usage: lmc-profile RESOLUTION --solver mg|mg-lowmem --ground shunt|dirichlet [--precision f32|f64|mixed] [--timings] [--residuals]\n\nMemory accounting is written as JSON to stdout. Timing tables and residual traces go to stderr.\nMixed precision requires --solver mg-lowmem.";

struct Args {
    resolution: usize,
    solver: String,
    ground: String,
    precision: String,
    timings: bool,
    residuals: bool,
}

fn upsample_grid<F: Real>(grid: &mut AscGrid<F>, resolution: usize) {
    let width = grid.ncols as f64 * grid.cellsize;
    grid.data = lm_connect::raster::upsample_raster(
        &grid.data, grid.nrows, grid.ncols, resolution, resolution,
    );
    grid.cellsize = width / resolution as f64;
    grid.nrows = resolution;
    grid.ncols = resolution;
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let resolution = args
        .first()
        .ok_or("resolution is required")?
        .parse::<usize>()
        .ok()
        .filter(|&n| n > 0)
        .ok_or("resolution must be a positive integer")?;
    let mut solver = None;
    let mut ground = None;
    let mut precision = None;
    let mut timings = false;
    let mut residuals = false;
    let mut i = 1;
    while i < args.len() {
        let flag = args[i].as_str();
        if matches!(flag, "--timings" | "--residuals") {
            let target = if flag == "--timings" { &mut timings } else { &mut residuals };
            if *target { return Err(format!("duplicate argument: {flag}")); }
            *target = true;
            i += 1;
            continue;
        }
        if !matches!(flag, "--solver" | "--ground" | "--precision") {
            return Err(format!("unknown argument: {flag}"));
        }
        let value = args
            .get(i + 1)
            .ok_or_else(|| format!("missing value for {flag}"))?;
        let target = match flag {
            "--solver" if matches!(value.as_str(), "mg" | "mg-lowmem") => &mut solver,
            "--ground" if matches!(value.as_str(), "shunt" | "dirichlet") => &mut ground,
            "--precision" if matches!(value.as_str(), "f32" | "f64" | "mixed") => &mut precision,
            _ => return Err(format!("unsupported value for {flag}: {value}")),
        };
        if target.replace(value.clone()).is_some() {
            return Err(format!("duplicate argument: {flag}"));
        }
        i += 2;
    }
    let solver = solver.ok_or("--solver is required")?;
    let precision = precision.unwrap_or_else(|| "f64".into());
    if precision == "mixed" && solver != "mg-lowmem" {
        return Err("mixed precision requires --solver mg-lowmem".into());
    }
    Ok(Args {
        resolution,
        solver,
        ground: ground.ok_or("--ground is required")?,
        precision,
        timings,
        residuals,
    })
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.len() == 1 && matches!(arguments[0].as_str(), "--help" | "-h") {
        println!("{USAGE}");
        return;
    }
    let Args {
        resolution,
        solver,
        ground,
        precision,
        timings,
        residuals,
    } = parse_args(&arguments).unwrap_or_else(|error| {
        eprintln!("{error}\n{USAGE}");
        std::process::exit(2);
    });
    let options = MgOptions { verbose: timings, profile: residuals, ..MgOptions::default() };
    match precision.as_str() {
        "mixed" => run_mixed(resolution, solver, ground, options),
        "f32" => run::<f32>(resolution, solver, ground, options),
        _ => run::<f64>(resolution, solver, ground, options),
    }
}

fn profile_resistance<F: Real>(
    base: &AscGrid<F>,
    features: FeatureCollection,
) -> Result<Vec<F>, String> {
    let mut model: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(MODEL_PATH).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    // The original profiler uses the base resistance at its stored scale.
    let terrain_scale = model
        .pointer_mut("/nodes/terrain/parameters/scale")
        .ok_or("Chudleigh model has no terrain scale")?;
    *terrain_scale = 1.into();

    let grid = Grid {
        nrows: base.nrows,
        ncols: base.ncols,
        pixel_width: base.cellsize,
    };
    let transform = GeoTransform {
        xmin: base.xllcorner,
        ymax: base.ymax,
        cellsize: base.cellsize,
    };
    let base_values = base.data.iter().map(|value| value.to_f64().unwrap()).collect();
    let mut inputs = std::collections::HashMap::from([
        ("base".to_owned(), PipelineData::raster(base_values)),
    ]);
    for (layer, buffer) in [("roads", 3.0), ("rivers", 4.0), ("buildings", 0.0)] {
        let layer_features = features.features.iter().filter_map(|feature| {
            let properties = feature.properties.as_ref()?;
            if properties.get("layer")?.as_str()? != layer {
                return None;
            }
            let mut feature = feature.clone();
            feature.properties = Some(serde_json::Map::from_iter([("value".into(), 1.into())]));
            Some(feature)
        });
        let collection = GeoJson::from(layer_features.collect::<Vec<_>>());
        let mask = rasterize_values::<f64>(
            &collection.to_string(), grid.nrows, grid.ncols, &transform, buffer,
        )?;
        inputs.insert(layer.into(), PipelineData::raster(mask));
    }

    let options = ExecutionOptions {
        samples: 1,
        threads: 1,
        seed: 0,
        all_layers: false,
        statistics: false,
    };
    let mut resistance = None;
    pipeline::execute(&model.to_string(), inputs, grid, options, None, |result| {
        if result.layer == OutputLayer::Resistance {
            let Some(PipelineData::Raster(values)) = result.outputs.get("data") else {
                return Err("Chudleigh model did not return a resistance raster".into());
            };
            resistance = Some(values.iter().map(|&value| scalar(value)).collect());
        }
        Ok(())
    })?;
    resistance.ok_or_else(|| "Chudleigh model did not return a resistance raster".into())
}

fn prepare_inputs<F: Real>(resolution: usize) -> (AscGrid<F>, AscGrid<F>, AscGrid<F>, Vec<F>) {
    let source_resolution = resolution.min(1000);
    let mut base = parse_asc_typed::<F, _>(&format!("{DATA_DIR}/base_resistance_{source_resolution}.asc"));
    let mut src = parse_asc_typed::<F, _>(&format!("{DATA_DIR}/source_{source_resolution}.asc"));
    let mut gnd = parse_asc_typed::<F, _>(&format!("{DATA_DIR}/ground_{source_resolution}.asc"));
    for grid in [&base, &src, &gnd] {
        assert!(grid.nrows > 0 && grid.ncols > 0, "ASC dimensions must be positive");
        assert_eq!(grid.data.len(), grid.nrows * grid.ncols,
            "ASC cell count must match dimensions");
    }
    assert_eq!(
        (src.nrows, src.ncols),
        (base.nrows, base.ncols),
        "source grid dimensions differ"
    );
    assert_eq!(
        (gnd.nrows, gnd.ncols),
        (base.nrows, base.ncols),
        "ground grid dimensions differ"
    );
    // Bundled inputs only go up to 1000×1000; upsample them for larger profiling runs.
    if resolution > source_resolution {
        for grid in [&mut base, &mut src, &mut gnd] {
            upsample_grid(grid, resolution);
        }
        eprintln!("Upsampled {source_resolution}x{source_resolution} inputs to {resolution}x{resolution} using nearest neighbours; geographic extent is unchanged.");
    }
    let geojson =
        fs::read_to_string(format!("{DATA_DIR}/all_features_{source_resolution}.geojson")).unwrap();

    let GeoJson::FeatureCollection(features) = geojson.parse().expect("invalid Chudleigh GeoJSON") else {
        panic!("expected a Chudleigh GeoJSON feature collection");
    };
    let resistance = profile_resistance(&base, features)
        .unwrap_or_else(|error| panic!("could not prepare Chudleigh resistance: {error}"));

    (base, src, gnd, resistance)
}

fn run<F: Real>(resolution: usize, solver: String, ground: String, options: MgOptions) {
    let (base, mut src, gnd, resistance) = prepare_inputs::<F>(resolution);
    let ground_mode = match ground.as_str() {
        "shunt" => GroundMode::Shunt,
        "dirichlet" => GroundMode::Dirichlet,
        _ => unreachable!(),
    };
    let max_iter = 100_000;
    let tol = F::DEFAULT_TOLERANCE;
    let remove_average = true;
    let result = match solver.as_str() {
        "mg" => solve::solve_raster_sources_mg_with_options(
            &resistance,
            base.nrows,
            base.ncols,
            base.nodata,
            &src.data,
            &gnd.data,
            max_iter,
            tol,
            remove_average,
            ground_mode,
            options,
        ),
        "mg-lowmem" => solve::solve_raster_sources_mg_stencil_in_place_with_options(
            &resistance,
            base.nrows,
            base.ncols,
            base.nodata,
            &mut src.data,
            &gnd.data,
            max_iter,
            tol,
            remove_average,
            ground_mode,
            options,
        ),
        _ => unreachable!(),
    };
    if let Some(error) = result.error {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
    let precision = if std::mem::size_of::<F>() == 4 { "f32" } else { "f64" };
    emit(resolution, &solver, &ground, precision, result.total_iters);
}

fn run_mixed(resolution: usize, solver: String, ground: String, options: MgOptions) {
    let (base, mut src, gnd, resistance) = prepare_inputs::<f32>(resolution);
    let ground_mode = match ground.as_str() {
        "shunt" => GroundMode::Shunt,
        "dirichlet" => GroundMode::Dirichlet,
        _ => unreachable!(),
    };
    let result = solve::mixed::solve_raster_sources_mg_mixed_with_options(
        &resistance, base.nrows, base.ncols, base.nodata,
        &mut src.data, &gnd.data, 100_000,
        <f32 as Real>::DEFAULT_TOLERANCE as f64, true, ground_mode, options,
    );
    if let Some(error) = result.error {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
    emit(resolution, &solver, &ground, "mixed", result.total_iters);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_flags_are_explicit_and_validated() {
        let parse = |arguments: &[&str]| {
            parse_args(&arguments.iter().map(|arg| arg.to_string()).collect::<Vec<_>>())
        };
        let plain = parse(&["500", "--solver", "mg-lowmem", "--ground", "shunt"]).unwrap();
        assert!(!plain.timings && !plain.residuals);
        let traced = parse(&["500", "--solver", "mg-lowmem", "--ground", "shunt",
            "--precision", "mixed", "--timings", "--residuals"]).unwrap();
        assert!(traced.timings && traced.residuals);
        assert_eq!(traced.precision, "mixed");
        assert!(parse(&["500", "--solver", "mg", "--ground", "shunt", "--precision", "mixed"]).is_err());
        assert!(parse(&["500", "--solver", "jacobi", "--ground", "shunt"]).is_err());
        assert!(parse(&["500", "--solver", "mg", "--ground", "shunt", "--residuals", "--residuals"]).is_err());
    }

    #[test]
    fn upsampling_preserves_geographic_extent() {
        let mut grid = AscGrid {
            data: vec![1.0, -9999.0, 0.0, 4.0], nrows: 2, ncols: 2,
            xllcorner: 286100.0, yllcorner: 78550.0, ymax: 78552.0,
            cellsize: 1.0, nodata: -9999.0,
        };
        upsample_grid(&mut grid, 4);
        assert_eq!((grid.nrows, grid.ncols), (4, 4));
        assert_eq!(grid.cellsize, 0.5);
        assert_eq!(grid.ncols as f64 * grid.cellsize, 2.0);
        assert_eq!(grid.yllcorner + grid.nrows as f64 * grid.cellsize, grid.ymax);
        assert_eq!((grid.xllcorner, grid.yllcorner), (286100.0, 78550.0));
        assert_eq!(grid.data, lm_connect::raster::upsample_raster(&[1.0, -9999.0, 0.0, 4.0], 2, 2, 4, 4));
    }
}
