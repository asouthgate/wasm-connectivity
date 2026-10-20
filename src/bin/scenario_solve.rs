use std::{collections::HashMap, env, fs, path::Path};

use wasm_connect::raster::io::{parse_asc, write_asc};
use wasm_connect::solve::{solve_raster_sources_mg_stencil_with_options, solve_raster_sources_mg_with_options, GroundMode};
use wasm_connect::linalg::multigrid::MgOptions;

const USAGE: &str = "Usage: scenario-solve --resistance FILE --source FILE --ground FILE
    [--tol FLOAT] [--max-iter INTEGER] [--nu INTEGER] [--autonu] [--explicit] [--output FILE] [--verbose]

Solve ASC rasters using low-memory CGMG with shunt grounding.
--explicit uses explicit matrices with symmetric Gauss–Seidel instead of low-memory Jacobi.
--nu sets the number of sweeps in each pre- and post-smoothing phase.
--autonu retains --nu on levels 0/1; levels 2–6 use at least 8,7,6,5,4 sweeps.
--verbose includes timings and aggregate V-cycle equation residuals by level.
Defaults: --tol 1e-4 --max-iter 100000 --nu 2 --output rust_current.asc";

fn argument_error(message: &str) -> ! {
    eprintln!("{message}\n\n{USAGE}");
    std::process::exit(2);
}

fn main() {
    let mut args = env::args().skip(1);
    let mut options = HashMap::new();
    let mut verbose = false;
    let mut autonu = false;
    let mut explicit = false;
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" {
            println!("{USAGE}");
            return;
        }
        if flag == "--verbose" {
            if verbose { argument_error("duplicate argument: --verbose"); }
            verbose = true;
            continue;
        }
        if flag == "--autonu" {
            if autonu { argument_error("duplicate argument: --autonu"); }
            autonu = true;
            continue;
        }
        if flag == "--explicit" {
            if explicit { argument_error("duplicate argument: --explicit"); }
            explicit = true;
            continue;
        }
        if !matches!(flag.as_str(), "--resistance" | "--source" | "--ground"
                     | "--tol" | "--max-iter" | "--nu" | "--output") {
            argument_error(&format!("unknown argument: {flag}"));
        }
        let value = args.next().filter(|value| !value.starts_with("--"))
            .unwrap_or_else(|| argument_error(&format!("missing value for {flag}")));
        if options.insert(flag.clone(), value).is_some() {
            argument_error(&format!("duplicate argument: {flag}"));
        }
    }
    for flag in ["--resistance", "--source", "--ground"] {
        if !options.contains_key(flag) {
            argument_error(&format!("{flag} is required"));
        }
    }
    let tol = options.get("--tol").map(String::as_str).unwrap_or("1e-4")
        .parse::<f64>().ok().filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or_else(|| argument_error("--tol must be finite and positive"));
    let max_iter = options.get("--max-iter").map(String::as_str).unwrap_or("100000")
        .parse::<usize>().ok().filter(|value| *value > 0)
        .unwrap_or_else(|| argument_error("--max-iter must be a positive integer"));
    let output = Path::new(options.get("--output").map(String::as_str).unwrap_or("rust_current.asc"));
    let nu = options.get("--nu").map(String::as_str).unwrap_or("2")
        .parse::<usize>().ok().filter(|value| *value > 0)
        .unwrap_or_else(|| argument_error("--nu must be a positive integer"));
    let resistance = parse_asc(&options["--resistance"]);
    let source = parse_asc(&options["--source"]);
    let ground = parse_asc(&options["--ground"]);
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

    let solve = if explicit {
        solve_raster_sources_mg_with_options
    } else {
        solve_raster_sources_mg_stencil_with_options
    };
    let mut result = solve(
        &resistance.data,
        resistance.nrows,
        resistance.ncols,
        resistance.nodata,
        &source.data,
        &ground.data,
        max_iter,
        tol,
        false,
        GroundMode::Shunt,
        MgOptions { verbose, nu, autonu, ..MgOptions::default() },
    );
    assert!(
        result.total_iters < max_iter,
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
    if let Some(parent) = output.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent).expect("cannot create output directory");
    }
    write_asc(
        output,
        &result.output.current_map,
        resistance.nrows,
        resistance.ncols,
        resistance.xllcorner,
        resistance.yllcorner,
        resistance.cellsize,
        resistance.nodata,
    );
    eprintln!("{}: {} iterations", output.display(), result.total_iters);
}
