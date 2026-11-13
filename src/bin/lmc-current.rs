use lm_connect::linalg::real::{Real, scalar};
use std::{collections::HashMap, env, fs, path::Path};

use lm_connect::raster::io::{read_raster_typed, require_aligned, write_raster, Raster};
use gdal::raster::GdalType;
use lm_connect::solve::{solve_raster_sources_mg_stencil_with_options, solve_raster_sources_mg_with_options, GroundMode};
use lm_connect::linalg::multigrid::MgOptions;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const USAGE: &str = "Usage: lmc-current --resistance FILE --source FILE --ground FILE
    [--precision f32|f64|mixed] [--tol FLOAT] [--max-iter INTEGER] [--l0nu INTEGER] [--l1nu INTEGER] [--coarsenu INTEGER] [--l0omega FLOAT] [--coarseomega FLOAT] [--l0postnu INTEGER] [--explicit] [--output FILE] [-v|--verbose]

Solve aligned GeoTIFF or ASC rasters using low-memory CGMG with shunt grounding.
GeoTIFF inputs require a projected CRS in metres; all input grids and CRSs must match.
Output format follows .tif/.tiff/.asc. Legacy ASC inputs without a CRS require an explicit .asc output.
--precision mixed uses f32 hierarchy/work vectors with f64 master voltages and reliable residual updates.
--explicit stores matrices explicitly instead of using the low-memory hierarchy.
Both hierarchies use weighted Jacobi smoothing on nonterminal levels.
--l0postnu adds L0 sweeps once after CG (default 40).
--l0omega directly overrides L0 damping; --coarseomega overrides L1 onward.
Default damping: --l0omega 0.95 --coarseomega 0.4.
--l0nu sets L0 pre/post sweeps (default 2).
--l1nu sets L1 pre/post sweeps (default 5).
--coarsenu sets pre/post sweeps on L2 and deeper nonterminal levels (default 25).
-v, --verbose prints the completion message.
Defaults: --tol 1e-4 (f64), 1e-3 (f32/mixed); --max-iter 100000 --l0nu 2 --l1nu 5 --coarsenu 25 --l0omega 0.95 --coarseomega 0.4 --l0postnu 40 --output rust_current.tif";

fn argument_error(message: &str) -> ! {
    eprintln!("{message}\n\n{USAGE}");
    std::process::exit(2);
}

fn main() {
    let mut args = env::args().skip(1);
    let mut options = HashMap::new();
    let mut verbose = false;
    let mut explicit = false;
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" {
            println!("{USAGE}");
            return;
        }
        if flag == "--verbose" || flag == "-v" {
            if verbose { argument_error("duplicate argument: --verbose"); }
            verbose = true;
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
        "f32" | "mixed" => <f32 as Real>::DEFAULT_TOLERANCE as f64,
        "f64" => <f64 as Real>::DEFAULT_TOLERANCE,
        _ => argument_error("--precision must be f32, f64 or mixed"),
    };
    let tol = options.get("--tol").map(|value| value.parse::<f64>().ok()
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or_else(|| argument_error("--tol must be finite and positive")))
        .unwrap_or(default_tol);
    let max_iter = options.get("--max-iter").map(String::as_str).unwrap_or("100000")
        .parse::<usize>().ok().filter(|value| *value > 0)
        .unwrap_or_else(|| argument_error("--max-iter must be a positive integer"));
    let output = Path::new(options.get("--output").map(String::as_str).unwrap_or("rust_current.tif"));
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
    let mg_options = MgOptions { l0nu, l1nu, coarsenu, l0omega, coarseomega, l0postnu, ..defaults };
    if precision == "mixed" && explicit { argument_error("mixed precision uses the low-memory solver; remove --explicit"); }
    let result = match precision {
        "mixed" => run_mixed(&options, output, max_iter, tol, mg_options, verbose),
        "f32" => run::<f32>(&options, output, max_iter, tol, explicit, mg_options, verbose),
        "f64" => run::<f64>(&options, output, max_iter, tol, explicit, mg_options, verbose),
        _ => argument_error("--precision must be f32, f64 or mixed"),
    };
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn read_inputs<F: Real + GdalType>(
    options: &HashMap<String, String>,
) -> Result<(Raster<F>, Raster<F>, Raster<F>)> {
    let resistance = read_raster_typed::<F>(Path::new(&options["--resistance"]))?;
    let source = read_raster_typed::<F>(Path::new(&options["--source"]))?;
    let ground = read_raster_typed::<F>(Path::new(&options["--ground"]))?;
    require_aligned(
        &resistance.grid,
        &source.grid,
        Path::new(&options["--source"]),
    )?;
    require_aligned(
        &resistance.grid,
        &ground.grid,
        Path::new(&options["--ground"]),
    )?;
    for (name, raster) in [("sources", &source), ("grounds", &ground)] {
        if !raster.data.iter().any(|v| v.is_finite() && *v > F::zero()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("no {name}"),
            )
            .into());
        }
    }
    Ok((resistance, source, ground))
}

fn run<F: Real + GdalType>(
    options: &HashMap<String, String>,
    output: &Path,
    max_iter: usize,
    tol: f64,
    explicit: bool,
    mg_options: MgOptions,
    verbose: bool,
) -> Result<()> {
    let tol = scalar::<F>(tol);
    if !tol.is_finite() || tol <= F::zero() {
        argument_error("--tol must be representable and positive in the selected precision");
    }
    let (resistance, source, ground) = read_inputs::<F>(options)?;
    // The solver's sentinel must be finite; file-specific missing values are NaN.
    let nodata = scalar::<F>(-9999.0);

    let solve = if explicit {
        solve_raster_sources_mg_with_options
    } else {
        solve_raster_sources_mg_stencil_with_options
    };
    let mut result = solve(
        &resistance.data,
        resistance.grid.nrows,
        resistance.grid.ncols,
        nodata,
        &source.data,
        &ground.data,
        max_iter,
        tol,
        false,
        GroundMode::Shunt,
        mg_options,
    );
    if let Some(error) = &result.error {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
    assert!(
        result.output.voltages.iter().all(|v| v.is_finite()),
        "non-finite voltages"
    );
    for (value, &res) in result.output.current_map.iter_mut().zip(&resistance.data) {
        if !res.is_finite() || res <= F::zero() || res == nodata {
            *value = F::nan();
        } else {
            assert!(value.is_finite(), "non-finite current");
        }
    }
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).expect("cannot create output directory");
    }
    write_raster(output, &resistance.grid, &result.output.current_map)?;
    if verbose {
        eprintln!(
            "Computed {} in {} iterations.",
            output.display(),
            result.total_iters
        );
    }
    Ok(())
}

fn run_mixed(
    options: &HashMap<String, String>,
    output: &Path,
    max_iter: usize,
    tol: f64,
    mg_options: MgOptions,
    verbose: bool,
) -> Result<()> {
    let (resistance, mut source, ground) = read_inputs::<f32>(options)?;
    let nodata = -9999.0f32;

    let mut result = lm_connect::solve::mixed::solve_raster_sources_mg_mixed_with_options(
        &resistance.data,
        resistance.grid.nrows,
        resistance.grid.ncols,
        nodata,
        &mut source.data,
        &ground.data,
        max_iter,
        tol,
        false,
        GroundMode::Shunt,
        mg_options,
    );
    if let Some(error) = &result.error {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
    assert!(
        result.output.voltages.iter().all(|v| v.is_finite()),
        "non-finite voltages"
    );
    for (value, &res) in result.output.current_map.iter_mut().zip(&resistance.data) {
        if !res.is_finite() || res <= 0.0 || res == nodata {
            *value = f64::NAN;
        } else {
            assert!(value.is_finite(), "non-finite current");
        }
    }
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).expect("cannot create output directory");
    }
    write_raster(output, &resistance.grid, &result.output.current_map)?;
    if verbose {
        eprintln!(
            "Computed {} in {} iterations.",
            output.display(),
            result.total_iters
        );
    }
    Ok(())
}
