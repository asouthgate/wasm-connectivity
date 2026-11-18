use super::{invalid, Result};
use crate::pipeline::{NodeResult, OutputLayer, PipelineData};
use crate::raster::io::{write_raster as write_tiff, RasterGrid as Grid};
use std::{fs, path::Path};

fn layer_name(name: &str, output: &str) -> String {
    if output == "data" {
        name.into()
    } else {
        format!("{name}.{output}")
    }
}

fn write_maps<'a>(directory: &Path, grid: &Grid, maps: Vec<(String, &'a [f64])>) -> Result<()> {
    let mut names = std::collections::HashSet::new();
    for (name, _) in &maps {
        if !names.insert(name) {
            return Err(invalid(format!("duplicate output filename: {name}")).into());
        }
    }
    fs::create_dir_all(directory)?;
    for (name, values) in maps {
        write_tiff(&directory.join(name), grid, values)?;
    }
    Ok(())
}

pub(super) fn write_node_result(
    directory: &Path,
    grid: &Grid,
    samples: usize,
    result: &NodeResult,
) -> Result<()> {
    let name = match &result.layer {
        OutputLayer::Resistance => "total_resistance".into(),
        OutputLayer::Current => "current".into(),
        OutputLayer::Intermediate {
            node_name,
            port_name,
        } => layer_name(node_name, port_name),
    };
    let output_directory = match result.sample {
        Some(sample) if samples > 1 => directory.join("samples").join(format!("{sample:06}")),
        _ => directory.to_path_buf(),
    };
    let mut maps = Vec::new();
    for (port, field) in &result.outputs {
        if let PipelineData::Raster(values) = field {
            let filename = if port == "variance" {
                format!("{name}_variance.tif")
            } else {
                format!("{name}.tif")
            };
            maps.push((filename, values.as_slice()));
        }
    }
    write_maps(&output_directory, grid, maps)
}
