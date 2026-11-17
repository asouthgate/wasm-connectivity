use super::Result;
use gdal::raster::GdalType;
use lm_connect::{
    linalg::{
        multigrid::MgOptions,
        real::{scalar, Real},
    },
    raster::io::{read_raster_typed, require_aligned, write_raster, Raster},
    solve::{
        solve_raster_sources_mg_stencil_with_options, solve_raster_sources_mg_with_options,
        AnnotatedOutput, GroundMode, RasterOutput,
    },
};
use std::{fs, io, path::PathBuf};

pub(super) struct CurrentArguments {
    pub resistance_path: PathBuf,
    pub source_path: PathBuf,
    pub ground_path: PathBuf,
    pub output_path: PathBuf,
    pub precision: String,
    pub max_iter: usize,
    pub tolerance: f64,
    pub explicit: bool,
    pub multigrid: MgOptions,
    pub verbose: bool,
}


fn read_inputs<F: Real + GdalType>(
    arguments: &CurrentArguments,
) -> Result<(Raster<F>, Raster<F>, Raster<F>)> {
    let resistance = read_raster_typed::<F>(&arguments.resistance_path)?;
    let source = read_raster_typed::<F>(&arguments.source_path)?;
    let ground = read_raster_typed::<F>(&arguments.ground_path)?;
    require_aligned(&resistance.grid, &source.grid, &arguments.source_path)?;
    require_aligned(&resistance.grid, &ground.grid, &arguments.ground_path)?;
    for (name, raster) in [("sources", &source), ("grounds", &ground)] {
        if !raster
            .data
            .iter()
            .any(|value| value.is_finite() && *value > F::zero())
        {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("no {name}")).into());
        }
    }
    Ok((resistance, source, ground))
}

fn write_current_output<F: Real + GdalType, R: Real>(
    arguments: &CurrentArguments,
    resistance: &Raster<R>,
    mut result: AnnotatedOutput<RasterOutput<F>>,
) -> Result<()> {
    if let Some(error) = result.error {
        return Err(io::Error::other(error).into());
    }
    result
        .output
        .mask_invalid_current(&resistance.data, scalar::<R>(-9999.0))?;
    if let Some(parent) = arguments
        .output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    write_raster(
        &arguments.output_path,
        &resistance.grid,
        &result.output.current_map,
    )?;
    if arguments.verbose {
        eprintln!(
            "Computed {} in {} iterations.",
            arguments.output_path.display(),
            result.total_iters
        );
    }
    Ok(())
}

fn execute_with_precision<F: Real + GdalType>(
    arguments: &CurrentArguments
) -> Result<()> {
    let (resistance, source, ground) = read_inputs::<F>(arguments)?;
    let solve = if arguments.explicit {
        solve_raster_sources_mg_with_options
    } else {
        solve_raster_sources_mg_stencil_with_options
    };
    let result = solve(
        &resistance.data,
        resistance.grid.nrows,
        resistance.grid.ncols,
        scalar::<F>(-9999.0),
        &source.data,
        &ground.data,
        arguments.max_iter,
        scalar::<F>(arguments.tolerance),
        false,
        GroundMode::Shunt,
        arguments.multigrid,
    );
    write_current_output(arguments, &resistance, result)
}

fn execute_mixed(
    arguments: &CurrentArguments
) -> Result<()> {
    let (resistance, mut source, ground) = read_inputs::<f32>(arguments)?;
    let result = lm_connect::solve::mixed::solve_raster_sources_mg_mixed_with_options(
        &resistance.data,
        resistance.grid.nrows,
        resistance.grid.ncols,
        -9999.0,
        &mut source.data,
        &ground.data,
        arguments.max_iter,
        arguments.tolerance,
        false,
        GroundMode::Shunt,
        arguments.multigrid,
    );
    write_current_output(arguments, &resistance, result)
}


pub(super) fn execute(
    arguments: CurrentArguments
) -> Result<()> {
    match arguments.precision.as_str() {
        "mixed" => execute_mixed(&arguments),
        "f32" => execute_with_precision::<f32>(&arguments),
        "f64" => execute_with_precision::<f64>(&arguments),
        _ => unreachable!("clap should have validated the precision :("),
    }
}