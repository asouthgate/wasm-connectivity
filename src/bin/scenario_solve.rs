use wasm_connect::linalg::real::{Real, scalar};
use std::{collections::HashMap, env, fs, path::Path};

use wasm_connect::raster::io::{parse_asc_typed, write_asc};
use wasm_connect::solve::{solve_raster_sources_mg_stencil_with_options, solve_raster_sources_mg_with_options, GroundMode};
use wasm_connect::linalg::multigrid::MgOptions;

const USAGE: &str = "Usage: scenario-solve --resistance FILE --source FILE --ground FILE
    [--precision f32|f64] [--tol FLOAT] [--max-iter INTEGER] [--l0nu INTEGER] [--l1nu INTEGER] [--coarsenu INTEGER] [--l0omega FLOAT] [--coarseomega FLOAT] [--l0postnu INTEGER] [--explicit] [--output FILE] [--verbose] [--profile]

Solve ASC rasters using low-memory CGMG with shunt grounding.
--explicit uses explicit matrices with symmetric Gauss–Seidel instead of low-memory Jacobi.
--l0postnu adds L0 sweeps once after CG (default 40).
--l0omega directly overrides L0 damping; --coarseomega overrides L1 onward.
Default damping: --l0omega 0.95 --coarseomega 0.4.
--l0nu sets L0 pre/post sweeps (default 2).
--l1nu sets L1 pre/post sweeps (default 5).
--coarsenu sets pre/post sweeps on L2 and deeper nonterminal levels (default 25).
--verbose prints aggregate solve timings by level.
--profile traces equation residuals at each CG iteration and V-cycle step.
Defaults: --tol 1e-4 (f64), 1e-3 (f32); --max-iter 100000 --l0nu 2 --l1nu 5 --coarsenu 25 --l0omega 0.95 --coarseomega 0.4 --l0postnu 40 --output rust_current.asc";

fn argument_error(message: &str) -> ! {
    eprintln!("{message}\n\n{USAGE}");
    std::process::exit(2);
}

fn main() {
    let mut args = env::args().skip(1);
    let mut options = HashMap::new();
    let mut verbose = false;
    let mut explicit = false;
    let mut profile = false;
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
        if flag == "--profile" {
            if profile { argument_error("duplicate argument: --profile"); }
            profile = true;
            continue;
        }
        if flag == "--explicit" {
            if explicit { argument_error("duplicate argument: --explicit"); }
            explicit = true;
            continue;
        }
        if !matches!(flag.as_str(), "--resistance" | "--source" | "--ground"
                     | "--precision" | "--tol" | "--max-iter" | "--l0nu" | "--l1nu" | "--coarsenu" | "--l0omega" | "--coarseomega" | "--l0postnu" | "--output") {
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
    let precision = options.get("--precision").map(String::as_str).unwrap_or("f64");
    let default_tol = match precision {
        "f32" => <f32 as Real>::DEFAULT_TOLERANCE as f64,
        "f64" => <f64 as Real>::DEFAULT_TOLERANCE,
        _ => argument_error("--precision must be f32 or f64"),
    };
    let tol = options.get("--tol").map(|value| value.parse::<f64>().ok()
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or_else(|| argument_error("--tol must be finite and positive")))
        .unwrap_or(default_tol);
    let max_iter = options.get("--max-iter").map(String::as_str).unwrap_or("100000")
        .parse::<usize>().ok().filter(|value| *value > 0)
        .unwrap_or_else(|| argument_error("--max-iter must be a positive integer"));
    let output = Path::new(options.get("--output").map(String::as_str).unwrap_or("rust_current.asc"));
    let defaults = MgOptions::default();
    let l0nu = options.get("--l0nu").map(|value| value.parse::<usize>().ok()
        .filter(|value| *value > 0)
        .unwrap_or_else(|| argument_error("--l0nu must be a positive integer")))
        .unwrap_or(defaults.l0nu);
    let l1nu = options.get("--l1nu").map(|value| value.parse::<usize>().ok()
        .filter(|value| *value > 0)
        .unwrap_or_else(|| argument_error("--l1nu must be a positive integer"))).or(defaults.l1nu);
    let coarsenu = options.get("--coarsenu").map(|value| value.parse::<usize>().ok()
        .filter(|value| *value > 0)
        .unwrap_or_else(|| argument_error("--coarsenu must be a positive integer")))
        .unwrap_or(defaults.coarsenu);
    let parse_omega = |flag: &str| options.get(flag).map(|value| {
        value.parse::<f64>().ok().filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or_else(|| argument_error(&format!("{flag} must be finite and positive")))
    });
    let l0omega = parse_omega("--l0omega").or(defaults.l0omega);
    let coarseomega = parse_omega("--coarseomega").or(defaults.coarseomega);
    let l0postnu = options.get("--l0postnu").map(|value| value.parse::<usize>()
        .unwrap_or_else(|_| argument_error("--l0postnu must be a nonnegative integer")))
        .unwrap_or(defaults.l0postnu);
    let mg_options = MgOptions { verbose, profile, l0nu, l1nu, coarsenu, l0omega, coarseomega, l0postnu, ..defaults };
    match precision {
        "f32" => run::<f32>(&options, output, max_iter, tol, explicit, mg_options),
        "f64" => run::<f64>(&options, output, max_iter, tol, explicit, mg_options),
        _ => argument_error("--precision must be f32 or f64"),
    }
}

fn run<F: Real>(options: &HashMap<String, String>, output: &Path, max_iter: usize,
    tol: f64, explicit: bool, mg_options: MgOptions) {
    let tol = scalar::<F>(tol);
    if !tol.is_finite() || tol <= F::zero() { argument_error("--tol must be representable and positive in the selected precision"); }
    let resistance = parse_asc_typed::<F, _>(&options["--resistance"]);
    let source = parse_asc_typed::<F, _>(&options["--source"]);
    let ground = parse_asc_typed::<F, _>(&options["--ground"]);
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
            .any(|&v| v.is_finite() && v > F::zero() && v != source.nodata),
        "no sources"
    );
    assert!(
        ground
            .data
            .iter()
            .any(|&v| v.is_finite() && v > F::zero() && v != ground.nodata),
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
        mg_options,
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
        if !res.is_finite() || res <= F::zero() || res == resistance.nodata {
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
