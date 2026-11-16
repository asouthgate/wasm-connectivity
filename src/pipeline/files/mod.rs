use super::{
    ExecutionOptions, Grid as ModelGrid, Input as ModelInput, NodeOutputs, PipelineData,
    PipelineGraph, Reference, SamplingResult,
};

use crate::{
    geospatial::{rasterize_field, GeoTransform},
    raster::io::{read_raster, write_raster as write_tiff, Raster, RasterGrid as Grid},
};
use gdal::{
    spatial_ref::SpatialRef,
    vector::{FieldValue, LayerAccess},
    Dataset,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    fs, io,
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

pub struct FileOutput {
    pub grid: Grid,
    pub output: Vec<f64>,
    pub layers: BTreeMap<String, NodeOutputs>,
    pub warnings: Vec<String>,
    output_reference: Reference,
}

fn require_aligned(reference: &Grid, raster: &Raster, path: &Path) -> Result<()> {
    crate::raster::io::require_aligned(reference, &raster.grid, path)
}

fn read_features(
    path: &Path,
    grid: &Grid,
    required_field: Option<&str>,
    nonnegative: bool,
) -> Result<Value> {
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase);
    let collection = match extension.as_deref() {
        Some("json" | "geojson") => {
            let collection: Value = serde_json::from_str(&fs::read_to_string(path)?)?;
            if let Some(crs) = collection.get("crs") {
                let name = crs
                    .pointer("/properties/name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("GeoJSON crs must declare properties.name"))?;
                if Some(SpatialRef::from_definition(name)?) != grid.crs {
                    return Err(invalid("GeoJSON CRS differs from raster CRS").into());
                }
            }
            collection
        }
        Some("gpkg") => {
            let dataset = Dataset::open(path)
                .map_err(|e| invalid(format!("cannot open vector {}: {e}", path.display())))?;
            if dataset.layer_count() != 1 {
                return Err(invalid(format!(
                    "{} must contain exactly one vector layer",
                    path.display()
                ))
                .into());
            }
            let mut layer = dataset.layer(0)?;
            let crs = layer
                .spatial_ref()
                .ok_or_else(|| invalid("GeoPackage layer has no CRS"))?;
            if Some(crs) != grid.crs {
                return Err(invalid("GeoPackage CRS differs from raster CRS").into());
            }
            if let Some(field) = required_field {
                if !layer
                    .defn()
                    .fields()
                    .any(|definition| definition.name() == field)
                {
                    return Err(invalid(format!(
                        "{} requires a numeric field named {field}",
                        path.display()
                    ))
                    .into());
                }
            }
            let mut features = Vec::new();
            for feature in layer.features() {
                let mut properties = serde_json::Map::new();
                if let Some(field) = required_field {
                    let value = match feature.field(feature.field_index(field)?)? {
                        Some(FieldValue::IntegerValue(v)) => v as f64,
                        Some(FieldValue::Integer64Value(v)) => v as f64,
                        Some(FieldValue::RealValue(v)) => v,
                        _ => {
                            return Err(invalid(format!(
                                "{} requires a finite numeric field named {field}",
                                path.display()
                            ))
                            .into())
                        }
                    };
                    properties.insert(field.to_string(), json!(value));
                }
                let geometry = feature
                    .geometry()
                    .ok_or_else(|| invalid("feature has no geometry"))?;
                features.push(json!({
                  "type": "Feature",
                  "properties": properties,
                  "geometry": serde_json::from_str::<Value>(&geometry.json()?)?
                }));
            }
            json!({
              "type": "FeatureCollection",
              "features": features
            })
        }
        _ => return Err(invalid("vector input must be .json, .geojson, or .gpkg").into()),
    };
    if collection.get("type").and_then(Value::as_str) != Some("FeatureCollection") {
        return Err(invalid("expected a GeoJSON FeatureCollection").into());
    }
    let features = collection
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("expected GeoJSON features array"))?;
    for (i, feature) in features.iter().enumerate() {
        if let Some(field) = required_field {
            feature
                .get("properties")
                .and_then(|p| p.get(field))
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite() && (!nonnegative || *v >= 0.0))
                .ok_or_else(|| {
                    invalid(format!(
                        "feature {i} requires a finite {}properties.{field}",
                        if nonnegative { "nonnegative " } else { "" }
                    ))
                })?;
        }
    }
    Ok(collection)
}

fn lamps_from_collection(collection: &Value, grid: &Grid) -> Result<Vec<f64>> {
    let mut lamps = Vec::new();
    for feature in collection["features"]
        .as_array()
        .ok_or_else(|| invalid("expected features array"))?
    {
        if feature.pointer("/geometry/type").and_then(Value::as_str) != Some("Point") {
            return Err(invalid("lamp features must be Points").into());
        }
        let coordinates = feature
            .pointer("/geometry/coordinates")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("lamp coordinates are missing"))?;
        let x = coordinates
            .first()
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
            .ok_or_else(|| invalid("invalid lamp x coordinate"))?;
        let y = coordinates
            .get(1)
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
            .ok_or_else(|| invalid("invalid lamp y coordinate"))?;
        let col = (x - grid.affine[0]) / grid.affine[1];
        let row = (grid.affine[3] - y) / grid.affine[1];
        lamps.extend([
            col,
            row,
            feature["properties"]["height"]
                .as_f64()
                .ok_or_else(|| invalid("lamp value is missing"))?,
        ]);
    }
    Ok(lamps)
}

fn raster_on_grid(grid: &Grid, raster: Raster, path: &Path) -> Result<Vec<f64>> {
    let cellsize = grid.affine[1];
    if grid.crs != raster.grid.crs || (cellsize - raster.grid.affine[1]).abs() > cellsize * 1e-9 {
        return Err(invalid(format!(
            "{}: raster CRS and cellsize must match the explicit grid",
            path.display()
        ))
        .into());
    }
    let col_offset = (grid.affine[0] - raster.grid.affine[0]) / cellsize;
    let row_offset = (raster.grid.affine[3] - grid.affine[3]) / cellsize;
    if [col_offset, row_offset]
        .iter()
        .any(|v| !v.is_finite() || (v - v.round()).abs() > 1e-7)
    {
        return Err(invalid(format!(
            "{}: raster cells must align with the explicit grid",
            path.display()
        ))
        .into());
    }
    let mut values = vec![f64::NAN; grid.nrows * grid.ncols];
    if col_offset >= raster.grid.ncols as f64
        || row_offset >= raster.grid.nrows as f64
        || col_offset + grid.ncols as f64 <= 0.0
        || row_offset + grid.nrows as f64 <= 0.0
    {
        return Ok(values);
    }
    let (dc, dr) = (col_offset.round() as isize, row_offset.round() as isize);
    let col_start = (-dc).max(0) as usize;
    let col_end = (raster.grid.ncols as isize - dc).min(grid.ncols as isize) as usize;
    let row_start = (-dr).max(0) as usize;
    let row_end = (raster.grid.nrows as isize - dr).min(grid.nrows as isize) as usize;
    let width = col_end - col_start;
    for row in row_start..row_end {
        let source =
            (row as isize + dr) as usize * raster.grid.ncols + (col_start as isize + dc) as usize;
        let destination = row * grid.ncols + col_start;
        values[destination..destination + width]
            .copy_from_slice(&raster.data[source..source + width]);
    }
    Ok(values)
}

struct LoadedInputs {
    grid: Grid,
    fields: HashMap<String, PipelineData>,
    warnings: Vec<String>,
}

/// Return the input names declared by validated pipeline JSON.
pub fn input_names(pipeline_json: &str) -> Result<Vec<String>> {
    Ok(PipelineGraph::from_json(pipeline_json)
        .map_err(invalid)?
        .inputs()
        .keys()
        .cloned()
        .collect())
}

pub fn run_files(pipeline_json: &str, paths: &HashMap<String, PathBuf>) -> Result<FileOutput> {
    let graph = PipelineGraph::from_json(pipeline_json).map_err(invalid)?;
    run_files_graph(&graph, paths)
}

fn run_files_graph(graph: &PipelineGraph, paths: &HashMap<String, PathBuf>) -> Result<FileOutput> {
    let loaded = load_inputs(&graph, paths)?;
    let mut result = None;
    graph
        .execute(
            loaded.fields,
            model_grid(&loaded.grid),
            ExecutionOptions {
                samples: 1,
                seed: 0,
                all_layers: true,
                statistics: false,
            },
            None,
            |sample| {
                result = Some((sample.resistance.as_ref().clone(), sample.layers.clone()));
                Ok(())
            },
        )
        .map_err(invalid)?;
    let (output, layers) = result.expect("one execution produces one result");
    Ok(FileOutput {
        grid: loaded.grid,
        output,
        layers,
        warnings: loaded.warnings,
        output_reference: graph.output.clone(),
    })
}

fn load_inputs(graph: &PipelineGraph, paths: &HashMap<String, PathBuf>) -> Result<LoadedInputs> {
    for name in graph.inputs.keys() {
        if !paths.contains_key(name) {
            return Err(invalid(format!("missing input path: {name}")).into());
        }
    }
    if paths.keys().any(|name| !graph.inputs.contains_key(name)) {
        return Err(invalid("undeclared model input path supplied").into());
    }
    let mut warnings = Vec::new();
    for (name, input) in &graph.inputs {
        if input.is_raster() {
            let path = &paths[name];
            let extension = path
                .extension()
                .and_then(|s| s.to_str())
                .map(str::to_ascii_lowercase);
            if !matches!(extension.as_deref(), Some("tif" | "tiff")) {
                return Err(invalid(format!(
                    "input {name}: resistance raster inputs must be GeoTIFF (.tif or .tiff): {}",
                    path.display()
                ))
                .into());
            }
        }
    }
    let mut fields = HashMap::new();
    let grid = if let Some(spec) = &graph.grid {
        let (nrows, ncols) = spec.dimensions().map_err(invalid)?;
        let crs = SpatialRef::from_definition(&spec.crs)?;
        if !crs.is_projected() || (crs.linear_units() - 1.0).abs() > 1e-9 {
            return Err(invalid("grid CRS must be projected with metre units").into());
        }
        Grid {
            nrows,
            ncols,
            crs: Some(crs),
            affine: [
                spec.bounds[0],
                spec.cellsize,
                0.0,
                spec.bounds[3],
                0.0,
                -spec.cellsize,
            ],
        }
    } else {
        let name = graph
            .inputs
            .iter()
            .find_map(
                |(name, input)| {
                    if input.is_raster() {
                        Some(name)
                    } else {
                        None
                    }
                },
            )
            .ok_or_else(|| {
                invalid("model requires at least one raster input or an explicit grid")
            })?;
        let path = &paths[name];
        let reference = read_raster(path)?;
        fields.insert(name.clone(), PipelineData::raster(reference.data));
        reference.grid
    };
    let grid = &grid;
    for (name, input) in &graph.inputs {
        if fields.contains_key(name) {
            continue;
        }
        let path = &paths[name];
        let field = match input {
            ModelInput::RasterTopography {}
            | ModelInput::RasterSurface {}
            | ModelInput::RasterRanked {} => {
                let raster = read_raster(&path)?;
                let values = if graph.grid.is_some() {
                    raster_on_grid(grid, raster, &path)?
                } else {
                    require_aligned(grid, &raster, &path)?;
                    raster.data
                };
                PipelineData::raster(values)
            }
            ModelInput::VectorMask {} => {
                let collection = read_features(path, grid, None, false)?;
                let transform = GeoTransform {
                    xmin: grid.affine[0],
                    ymax: grid.affine[3],
                    cellsize: grid.affine[1],
                };
                let mask = crate::geospatial::rasterize_mask(
                    &collection.to_string(),
                    grid.nrows,
                    grid.ncols,
                    &transform,
                )?;
                PipelineData::raster(mask)
            }
            ModelInput::VectorOccluder {} | ModelInput::VectorResistance {} => {
                let (field, nonnegative) = if matches!(input, ModelInput::VectorOccluder {}) {
                    ("height", true)
                } else {
                    ("resistance", false)
                };
                let collection = read_features(path, grid, Some(field), nonnegative)?;
                let values = rasterize_field(
                    &collection.to_string(),
                    field,
                    grid.nrows,
                    grid.ncols,
                    &GeoTransform {
                        xmin: grid.affine[0],
                        ymax: grid.affine[3],
                        cellsize: grid.affine[1],
                    },
                    0.0,
                    0.0,
                )
                .map_err(invalid)?;
                PipelineData::raster(values)
            }
            ModelInput::RaycasterPointSource {} => {
                let collection = read_features(path, grid, Some("height"), true)?;
                let points = lamps_from_collection(&collection, grid)?;
                let outside = points
                    .chunks_exact(3)
                    .filter(|point| !model_grid(grid).contains_point(point))
                    .count();
                if outside > 0 {
                    warnings.push(format!(
                        "input {name}: {outside} light source(s) outside the grid were ignored and will not contribute irradiance."
                    ));
                }
                PipelineData::Points(std::sync::Arc::new(points))
            }
        };
        fields.insert(name.clone(), field);
    }
    Ok(LoadedInputs {
        grid: grid.clone(),
        fields,
        warnings,
    })
}

fn model_grid(grid: &Grid) -> ModelGrid {
    ModelGrid {
        nrows: grid.nrows,
        ncols: grid.ncols,
        pixel_width: grid.affine[1],
    }
}

impl FileOutput {
    pub fn write(self, output_dir: &Path, all_layers: bool) -> Result<()> {
        let mut maps = vec![("total_resistance.tif".into(), self.output.as_slice())];
        if all_layers {
            for (name, outputs) in &self.layers {
                for (port, field) in outputs {
                    if self.output_reference.selects_node(name, port) {
                        continue;
                    }
                    if let PipelineData::Raster(values) = field {
                        let filename = layer_name(name, port);
                        if filename == "total_resistance" {
                            return Err(invalid(
                                "node total_resistance conflicts with the final output filename",
                            )
                            .into());
                        }
                        maps.push((format!("{filename}.tif"), values.as_slice()));
                    }
                }
            }
        }
        write_maps(output_dir, &self.grid, maps)
    }
}

#[cfg(test)]
mod tests;

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

pub fn run_samples_to_directory(
    pipeline_json: &str,
    paths: &HashMap<String, PathBuf>,
    output_dir: &Path,
    options: ExecutionOptions,
    save_samples: bool,
    current_paths: Option<(&Path, &Path)>,
) -> Result<Vec<String>> {
    let graph = PipelineGraph::from_json(pipeline_json).map_err(invalid)?;
    if !options.statistics {
        return Err(invalid("statistics must be enabled for sampling summaries").into());
    }
    let loaded = load_inputs(&graph, paths)?;
    let current_inputs = current_paths
        .map(|(source, ground)| -> Result<_> {
            let source_raster = read_raster(source)?;
            let ground_raster = read_raster(ground)?;
            require_aligned(&loaded.grid, &source_raster, source)?;
            require_aligned(&loaded.grid, &ground_raster, ground)?;
            Ok((source_raster.data, ground_raster.data))
        })
        .transpose()?;
    let current_solver = current_inputs
        .as_ref()
        .map(|(source, ground)| crate::pipeline::CurrentSolver::new(source, ground));
    let result = graph
        .execute(
            loaded.fields,
            model_grid(&loaded.grid),
            options,
            current_solver.as_ref(),
            |sample| {
                if !save_samples {
                    return Ok(());
                }
                let mut maps = vec![("total_resistance.tif".into(), sample.resistance.as_slice())];
                if let Some(current) = &sample.current {
                    maps.push(("current.tif".into(), current));
                }
                for (name, outputs) in &sample.layers {
                    for (port, field) in outputs {
                        if graph.output.selects_node(name, port) {
                            continue;
                        }
                        if let PipelineData::Raster(values) = field {
                            maps.push((format!("{}.tif", layer_name(name, port)), values));
                        }
                    }
                }
                write_maps(
                    &output_dir
                        .join("samples")
                        .join(format!("{:06}", sample.index)),
                    &loaded.grid,
                    maps,
                )
                .map_err(|e| e.to_string())
            },
        )
        .map_err(invalid)?;
    let result = result.expect("statistics were requested");
    write_statistics(output_dir, &loaded.grid, &graph.output, &result)?;
    Ok(loaded.warnings)
}

fn write_statistics(
    directory: &Path,
    grid: &Grid,
    output: &Reference,
    result: &SamplingResult,
) -> Result<()> {
    let mut maps = vec![
        (
            "total_resistance.tif".into(),
            result.resistance.mean.as_slice(),
        ),
        (
            "total_resistance_variance.tif".into(),
            result.resistance.variance.as_slice(),
        ),
    ];
    if let Some(current) = &result.current {
        maps.push(("current.tif".into(), current.mean.as_slice()));
        maps.push(("current_variance.tif".into(), current.variance.as_slice()));
    }
    for (name, outputs) in &result.layers {
        for (port, stats) in outputs {
            if output.selects_node(name, port) {
                continue;
            }
            let name = layer_name(name, port);
            maps.push((format!("{name}.tif"), stats.mean.as_slice()));
            maps.push((format!("{name}_variance.tif"), stats.variance.as_slice()));
        }
    }
    write_maps(directory, grid, maps)
}
