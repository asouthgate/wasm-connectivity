use gdal::{
    spatial_ref::SpatialRef,
    vector::{FieldValue, LayerAccess},
    Dataset,
};
use lm_connect::{
    geospatial::{rasterize_values, rasterize_values_with_background, GeoTransform},
    raster::io::{read_raster, write_raster as write_tiff, Raster, RasterGrid as Grid},
    resistance::{
        landscape::{compute_base_rank, get_landscape_resistance_lcm},
        model::{Field as ModelField, Grid as ModelGrid, Input as ModelInput, Model},
        pipeline::{run_resistance_pipeline, ResistanceParams},
    },
};
use serde_json::{json, Map, Value};
use std::{
    collections::HashMap,
    env, fs,
    io,
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const USAGE: &str = "Usage: lmc-resistance --model model.json --output-dir results/ [--all-layers] [-v|--verbose]

Legacy usage: lmc-resistance --config model.json --dtm terrain.tif --dsm surface.tif --lcm landcover.tif --output-dir results/
    [--buildings buildings.json|buildings.gpkg] [--roads roads.json|roads.gpkg]
    [--rivers rivers.json|rivers.gpkg] [--generic-resistance zones.json|zones.gpkg]
    [--lamps lamps.json|lamps.gpkg] [--stage full|landscape] [-v|--verbose]

Input rasters must be single-band, north-up, square-pixel grids aligned in the same projected CRS with metre units.
Vector features use numeric value fields: building height, road/river presence, generic resistance, or lamp height.
GeoJSON coordinates must already use the raster CRS. GeoPackages must contain one vector layer in that CRS.
The --model JSON defines inline inputs and named operation layers; paths are relative to the model file.
The legacy --config JSON contains model coefficients only. Grid metadata comes from the rasters.
With --model, --all-layers also writes each named raster layer and surface output.
The final output is total_resistance.tif. Outputs are georeferenced GeoTIFF layers.";

const MODEL_FIELDS: &[&str] = &[
    "road_buffer",
    "road_resmax",
    "road_xmax",
    "river_buffer",
    "river_resmax",
    "river_xmax",
    "landscape_rankmax",
    "landscape_resmax",
    "landscape_xmax",
    "linear_buffer",
    "linear_rankmax",
    "linear_resmax",
    "linear_xmax",
    "lamp_resmax",
    "lamp_xmax",
    "lamp_ext",
];

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

struct Args {
    paths: HashMap<String, PathBuf>,
    landscape_only: bool,
    all_layers: bool,
    verbose: bool,
}

fn parse_args(arguments: &[String]) -> Result<Args> {
    let mut paths = HashMap::new();
    let mut stage = None;
    let mut verbose = false;
    let mut all_layers = false;
    let mut i = 0;
    while i < arguments.len() {
        let flag = &arguments[i];
        if flag == "--all-layers" {
            if all_layers {
                return Err(invalid("duplicate --all-layers").into());
            }
            all_layers = true;
            i += 1;
            continue;
        }
        if matches!(flag.as_str(), "-v" | "--verbose") {
            verbose = true;
            i += 1;
            continue;
        }
        if !matches!(
            flag.as_str(),
            "--model"
                | "--config"
                | "--dtm"
                | "--dsm"
                | "--lcm"
                | "--output-dir"
                | "--buildings"
                | "--roads"
                | "--rivers"
                | "--generic-resistance"
                | "--lamps"
                | "--stage"
        ) {
            return Err(invalid(format!("unknown argument: {flag}")).into());
        }
        let value = arguments
            .get(i + 1)
            .filter(|s| !s.starts_with('-'))
            .ok_or_else(|| invalid(format!("missing value for {flag}")))?;
        if flag == "--stage" {
            if stage.replace(value.clone()).is_some() {
                return Err(invalid("duplicate --stage").into());
            }
            if !matches!(value.as_str(), "full" | "landscape") {
                return Err(invalid("--stage must be full or landscape").into());
            }
        } else if paths.insert(flag.clone(), PathBuf::from(value)).is_some() {
            return Err(invalid(format!("duplicate argument: {flag}")).into());
        }
        i += 2;
    }
    if paths.contains_key("--model") {
        if stage.is_some()
            || paths
                .keys()
                .any(|name| name != "--model" && name != "--output-dir")
        {
            return Err(invalid("--model accepts inline inputs; do not combine it with legacy input/config/stage flags").into());
        }
        if !paths.contains_key("--output-dir") {
            return Err(invalid("--output-dir is required").into());
        }
        return Ok(Args {
            paths,
            landscape_only: false,
            all_layers,
            verbose,
        });
    }
    for name in ["--config", "--dtm", "--dsm", "--lcm", "--output-dir"] {
        if !paths.contains_key(name) {
            return Err(invalid(format!("{name} is required")).into());
        }
    }
    let landscape_only = stage.as_deref() == Some("landscape");
    if landscape_only
        && ["--roads", "--rivers", "--generic-resistance", "--lamps"]
            .iter()
            .any(|name| paths.contains_key(*name))
    {
        return Err(invalid("--stage landscape accepts buildings but not roads, rivers, generic resistance, or lamps").into());
    }
    Ok(Args {
        paths,
        landscape_only,
        all_layers,
        verbose,
    })
}

fn require_aligned(reference: &Grid, raster: &Raster, path: &Path) -> Result<()> {
    lm_connect::raster::io::require_aligned(reference, &raster.grid, path)
}

fn read_features(path: &Path, grid: &Grid) -> Result<Value> {
    read_features_with_field(path, grid, "value", true)
}

fn read_features_with_field(
    path: &Path,
    grid: &Grid,
    value_field: &str,
    nonnegative: bool,
) -> Result<Value> {
    if value_field.is_empty() {
        return Err(invalid("vector value field must not be empty").into());
    }
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase);
    let mut collection = match extension.as_deref() {
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
            let has_value = layer
                .defn()
                .fields()
                .any(|field| field.name() == value_field);
            if !has_value {
                return Err(invalid(
                    "GeoPackage features require the selected numeric value field",
                )
                .into());
            }
            let mut features = Vec::new();
            for feature in layer.features() {
                let value = match feature.field(feature.field_index(value_field)?)? {
                    Some(FieldValue::IntegerValue(v)) => v as f64,
                    Some(FieldValue::Integer64Value(v)) => v as f64,
                    Some(FieldValue::RealValue(v)) => v,
                    _ => {
                        return Err(invalid(
                            "GeoPackage features require the selected numeric value field",
                        )
                        .into())
                    }
                };
                let properties = json!({(value_field): value});
                let geometry = feature
                    .geometry()
                    .ok_or_else(|| invalid("feature has no geometry"))?;
                features.push(json!({"type":"Feature", "properties":properties, "geometry":serde_json::from_str::<Value>(&geometry.json()?)?}));
            }
            json!({"type":"FeatureCollection", "features":features})
        }
        _ => return Err(invalid("vector input must be .json, .geojson, or .gpkg").into()),
    };
    if collection.get("type").and_then(Value::as_str) != Some("FeatureCollection") {
        return Err(invalid("expected a GeoJSON FeatureCollection").into());
    }
    let features = collection
        .get_mut("features")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| invalid("expected GeoJSON features array"))?;
    for (i, feature) in features.iter_mut().enumerate() {
        let value = feature
            .get("properties")
            .and_then(|p| p.get(value_field))
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && (!nonnegative || *v >= 0.0))
            .ok_or_else(|| {
                invalid(format!(
                    "feature {i} requires a finite {}properties.{value_field}",
                    if nonnegative { "nonnegative " } else { "" }
                ))
            })?;
        feature["properties"]["value"] = json!(value);
    }
    Ok(collection)
}

fn vector_raster(path: Option<&PathBuf>, grid: &Grid) -> Result<Vec<f64>> {
    let Some(path) = path else {
        return Ok(vec![0.0; grid.nrows * grid.ncols]);
    };
    let features = read_features(path, grid)?;
    rasterize_collection(&features, grid)
}

fn rasterize_collection(features: &Value, grid: &Grid) -> Result<Vec<f64>> {
    rasterize_values(
        &features.to_string(),
        grid.nrows,
        grid.ncols,
        &GeoTransform {
            xmin: grid.affine[0],
            ymax: grid.affine[3],
            cellsize: grid.affine[1],
        },
        0.0,
    )
    .map_err(|e| invalid(e).into())
}

fn read_buildings(path: Option<&PathBuf>, grid: &Grid) -> Result<Vec<f64>> {
    let Some(path) = path else {
        return Ok(vec![0.0; grid.nrows * grid.ncols]);
    };
    let collection = read_features(path, grid)?;
    for (index, feature) in collection["features"].as_array().unwrap().iter().enumerate() {
        if feature["properties"]["value"].as_f64().unwrap() <= 0.0 {
            return Err(invalid(format!("building feature {index} requires a positive height in properties.value")).into());
        }
    }
    rasterize_collection(&collection, grid)
}

fn read_lamps(path: Option<&PathBuf>, grid: &Grid) -> Result<Vec<f64>> {
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    let collection = read_features(path, grid)?;
    lamps_from_collection(&collection, grid)
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
        if col < 0.0 || row < 0.0 || col >= grid.ncols as f64 || row >= grid.nrows as f64 {
            return Err(invalid("lamp lies outside the raster grid").into());
        }
        lamps.extend([
            col,
            row,
            feature["properties"]["value"]
                .as_f64()
                .ok_or_else(|| invalid("lamp value is missing"))?,
        ]);
    }
    Ok(lamps)
}

fn read_config(path: &Path) -> Result<Map<String, Value>> {
    let config: Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    let config = config
        .as_object()
        .ok_or_else(|| invalid("config must be a JSON object of model parameters"))?;
    for name in config.keys() {
        if !MODEL_FIELDS.contains(&name.as_str()) {
            return Err(invalid(format!("unknown model parameter: {name}")).into());
        }
    }
    Ok(config.clone())
}

fn parameter(config: &Map<String, Value>, name: &str) -> Result<f64> {
    let value = config
        .get(name)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
        .ok_or_else(|| invalid(format!("config requires a finite numeric {name}")))?;
    let denominator = name.ends_with("xmax") || name.ends_with("rankmax");
    if value < 0.0 || (denominator && value == 0.0) {
        return Err(invalid(format!("invalid model parameter: {name}")).into());
    }
    Ok(value)
}

fn execute(args: Args) -> Result<()> {
    if args.paths.contains_key("--model") {
        return execute_model(&args);
    }
    let log = |message: &str| {
        if args.verbose {
            eprintln!("{message}");
        }
    };
    let config = read_config(&args.paths["--config"])?;
    let dtm = read_raster(&args.paths["--dtm"])?;
    let dsm = read_raster(&args.paths["--dsm"])?;
    let lcm = read_raster(&args.paths["--lcm"])?;
    let grid = &dtm.grid;
    require_aligned(grid, &dsm, &args.paths["--dsm"])?;
    require_aligned(grid, &lcm, &args.paths["--lcm"])?;
    let buildings = read_buildings(args.paths.get("--buildings"), &dtm.grid)?;
    let rankmax = parameter(&config, "landscape_rankmax")?;
    let resmax = parameter(&config, "landscape_resmax")?;
    let xmax = parameter(&config, "landscape_xmax")?;
    let output_dir = &args.paths["--output-dir"];
    if args.landscape_only {
        log("Computing landscape resistance");
        let conductance = compute_base_rank(&lcm.data);
        let mut landscape =
            get_landscape_resistance_lcm(&lcm.data, &buildings, rankmax, resmax, xmax);
        for (i, value) in landscape.iter_mut().enumerate() {
            if !dtm.data[i].is_finite() || (!dsm.data[i].is_finite() && !(buildings[i].is_finite() && buildings[i] > 0.0)) {
                *value = f64::NAN;
            }
        }
        fs::create_dir_all(output_dir)?;
        write_tiff(
            &output_dir.join("landscape_conductance.tif"),
            grid,
            &conductance,
        )?;
        write_tiff(&output_dir.join("landscape_res.tif"), grid, &landscape)?;
    } else {
        let params = ResistanceParams {
            road_buffer: parameter(&config, "road_buffer")?,
            road_resmax: parameter(&config, "road_resmax")?,
            road_xmax: parameter(&config, "road_xmax")?,
            river_buffer: parameter(&config, "river_buffer")?,
            river_resmax: parameter(&config, "river_resmax")?,
            river_xmax: parameter(&config, "river_xmax")?,
            landscape_rankmax: rankmax,
            landscape_resmax: resmax,
            landscape_xmax: xmax,
            linear_buffer: parameter(&config, "linear_buffer")?,
            linear_rankmax: parameter(&config, "linear_rankmax")?,
            linear_resmax: parameter(&config, "linear_resmax")?,
            linear_xmax: parameter(&config, "linear_xmax")?,
            lamp_resmax: parameter(&config, "lamp_resmax")?,
            lamp_xmax: parameter(&config, "lamp_xmax")?,
            lamp_ext: parameter(&config, "lamp_ext")?,
            pixw: grid.affine[1],
            nrows: grid.nrows,
            ncols: grid.ncols,
        };
        let roads = vector_raster(args.paths.get("--roads"), grid)?;
        let rivers = vector_raster(args.paths.get("--rivers"), grid)?;
        let generic = vector_raster(args.paths.get("--generic-resistance"), grid)?;
        let lamps = read_lamps(args.paths.get("--lamps"), grid)?;
        log("Computing resistance rasters");
        let output = run_resistance_pipeline(
            &roads,
            &rivers,
            &buildings,
            Some(&lcm.data),
            &dtm.data,
            &dsm.data,
            &generic,
            &lamps,
            &params,
            None,
            None,
        );
        fs::create_dir_all(output_dir)?;
        for (name, values) in [
            ("road_res", &output.road_res),
            ("river_res", &output.river_res),
            ("landscape_res", &output.landscape_res),
            ("linear_res", &output.linear_res),
            ("lamp_res", &output.lamp_res),
            ("generic_res", &output.generic_res),
            ("total_resistance", &output.total_res),
            ("soft_surf", &output.soft_surf),
            ("hard_surf", &output.hard_surf),
            ("manhedge", &output.manhedge),
            ("unmanhedge", &output.unmanhedge),
            ("tree", &output.tree),
        ] {
            write_tiff(&output_dir.join(format!("{name}.tif")), grid, values)?;
        }
        let log_total: Vec<_> = output
            .total_res
            .iter()
            .map(|v| {
                if v.is_finite() && *v > 0.0 {
                    v.ln()
                } else {
                    f64::NAN
                }
            })
            .collect();
        write_tiff(&output_dir.join("log_total_res.tif"), grid, &log_total)?;
        if output.lamp_res.iter().any(|v| *v > 0.0) {
            let log_lamp: Vec<_> = output
                .lamp_res
                .iter()
                .map(|v| {
                    if v.is_finite() && *v > 0.0 {
                        v.ln()
                    } else {
                        f64::NAN
                    }
                })
                .collect();
            write_tiff(&output_dir.join("log_lamp_res.tif"), grid, &log_lamp)?;
        }
    }
    log("Resistance pipeline complete");
    Ok(())
}

fn execute_model(args: &Args) -> Result<()> {
    let model_path = &args.paths["--model"];
    let model: Model = serde_json::from_str(&fs::read_to_string(model_path)?)
        .map_err(|e| invalid(format!("{}: {e}", model_path.display())))?;
    let directory = model_path
        .parent()
        .ok_or_else(|| invalid("model path has no parent"))?;
    let Some(ModelInput::Raster { path }) = model.inputs.get(&model.grid) else {
        return Err(invalid("model grid must name a raster input").into());
    };
    let reference = read_raster(&directory.join(path))?;
    let grid = &reference.grid;
    let mut fields = HashMap::new();
    for (name, input) in &model.inputs {
        if args.verbose {
            eprintln!("Reading input {name}");
        }
        let field = match input {
            ModelInput::Raster { path } => {
                if name == &model.grid {
                    ModelField::raster(reference.data.clone())
                } else {
                    let path = directory.join(path);
                    let raster = read_raster(&path)?;
                    require_aligned(grid, &raster, &path)?;
                    ModelField::raster(raster.data)
                }
            }
            ModelInput::Vector {
                path,
                value_field,
                background,
            } => {
                if !background.is_finite() {
                    return Err(invalid(format!("input {name}: background must be finite")).into());
                }
                let collection =
                    read_features_with_field(&directory.join(path), grid, value_field, false)?;
                ModelField::raster(
                    rasterize_values_with_background(
                        &collection.to_string(),
                        grid.nrows,
                        grid.ncols,
                        &GeoTransform {
                            xmin: grid.affine[0],
                            ymax: grid.affine[3],
                            cellsize: grid.affine[1],
                        },
                        0.0,
                        *background,
                    )
                    .map_err(invalid)?,
                )
            }
            ModelInput::Points { path, height_field } => {
                let collection =
                    read_features_with_field(&directory.join(path), grid, height_field, true)?;
                ModelField::Points(std::sync::Arc::new(lamps_from_collection(
                    &collection,
                    grid,
                )?))
            }
        };
        fields.insert(name.clone(), field);
    }
    if args.verbose {
        eprintln!("Evaluating model {}", model.name);
    }
    let (output, layers) = model
        .run_with_layers(
            fields,
            ModelGrid {
                nrows: grid.nrows,
                ncols: grid.ncols,
                pixel_width: grid.affine[1],
            },
        )
        .map_err(invalid)?;
    let mut extra_layers = Vec::new();
    if args.all_layers {
        for (name, field) in layers {
            match field {
                ModelField::Raster(values) => extra_layers.push((name, values)),
                ModelField::Surfaces { soft, hard } => {
                    extra_layers.push((format!("{name}.soft"), soft));
                    extra_layers.push((format!("{name}.hard"), hard));
                }
                ModelField::Points(_) => {}
            }
        }
        extra_layers.retain(|(name, _)| name != &model.output);
        if extra_layers
            .iter()
            .any(|(name, _)| name == "total_resistance")
        {
            return Err(
                invalid("layer total_resistance conflicts with the final output filename").into(),
            );
        }
    }
    // All validation and calculations complete before creating any output files.
    let output_dir = &args.paths["--output-dir"];
    fs::create_dir_all(output_dir)?;
    write_tiff(&output_dir.join("total_resistance.tif"), grid, &output)?;
    for (name, values) in extra_layers {
        write_tiff(&output_dir.join(format!("{name}.tif")), grid, &values)?;
    }
    Ok(())
}

fn main() {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments.len() == 1 && matches!(arguments[0].as_str(), "--help" | "-h") {
        println!("{USAGE}");
        return;
    }
    let args = match parse_args(&arguments) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    if let Err(e) = execute(args) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gdal::{raster::Buffer, DriverManager};
    use gdal::vector::{Feature, Geometry, LayerOptions, OGRFieldType, OGRwkbGeometryType};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = env::temp_dir().join(format!(
                "lmc-resistance-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn grid() -> Grid {
        Grid {
            ncols: 8,
            nrows: 8,
            affine: [0.0, 1.0, 0.0, 8.0, 0.0, -1.0],
            crs: Some(SpatialRef::from_epsg(27700).unwrap()),
        }
    }
    fn config() -> Value {
        json!({
            "road_buffer":200,"road_resmax":10,"road_xmax":5,
            "river_buffer":10,"river_resmax":2000,"river_xmax":4,
            "landscape_rankmax":8,"landscape_resmax":100,"landscape_xmax":5,
            "linear_buffer":10,"linear_rankmax":4,"linear_resmax":22000,"linear_xmax":3,
            "lamp_resmax":100000000,"lamp_xmax":1,"lamp_ext":100
        })
    }
    fn collection() -> Value {
        json!({"type":"FeatureCollection","features":[{
            "type":"Feature", "properties":{"value":10},
            "geometry":{"type":"Polygon","coordinates":[[[2,2],[6,2],[6,6],[2,6],[2,2]]]}
        }]})
    }
    fn setup(dir: &Path) -> Args {
        let grid = grid();
        for (name, value) in [("dtm", 100.0), ("dsm", 100.0), ("lcm", 1.0)] {
            write_tiff(&dir.join(format!("{name}.tif")), &grid, &vec![value; 64]).unwrap();
        }
        fs::write(dir.join("model.json"), config().to_string()).unwrap();
        fs::write(dir.join("buildings.json"), collection().to_string()).unwrap();
        let mut paths = HashMap::new();
        for (flag, file) in [
            ("--config", "model.json"),
            ("--dtm", "dtm.tif"),
            ("--dsm", "dsm.tif"),
            ("--lcm", "lcm.tif"),
            ("--buildings", "buildings.json"),
            ("--output-dir", "results"),
        ] {
            paths.insert(flag.into(), dir.join(file));
        }
        Args {
            paths,
            landscape_only: false,
            all_layers: false,
            verbose: false,
        }
    }
    fn gpkg(path: &Path, grid: &Grid) {
        let mut dataset = DriverManager::get_driver_by_name("GPKG")
            .unwrap()
            .create_vector_only(path)
            .unwrap();
        let layer = dataset
            .create_layer(LayerOptions {
                name: "buildings",
                srs: grid.crs.as_ref(),
                ty: OGRwkbGeometryType::wkbPolygon,
                ..Default::default()
            })
            .unwrap();
        layer
            .create_defn_fields(&[("value", OGRFieldType::OFTReal)])
            .unwrap();
        let mut feature = Feature::new(layer.defn()).unwrap();
        feature
            .set_field_double(feature.field_index("value").unwrap(), 10.0)
            .unwrap();
        feature
            .set_geometry(Geometry::from_wkt("POLYGON ((2 2,6 2,6 6,2 6,2 2))").unwrap())
            .unwrap();
        feature.create(&layer).unwrap();
    }

    #[test]
    fn buildings_require_explicit_heights() {
        let dir = TestDir::new();
        let grid = grid();
        let path = dir.0.join("buildings.json");
        let mut features = collection();
        for invalid in [json!({}), json!({"value":null}), json!({"value":"10"}), json!({"value":0}), json!({"value":-1})] {
            features["features"][0]["properties"] = invalid;
            fs::write(&path, features.to_string()).unwrap();
            assert!(read_buildings(Some(&path), &grid)
                .unwrap_err()
                .to_string()
                .contains("properties.value"));
        }
        features["features"][0]["properties"] = json!({"value":10});
        fs::write(&path, features.to_string()).unwrap();
        let heights = read_buildings(Some(&path), &grid).unwrap();
        assert_eq!(heights[0], 0.0);
        assert_eq!(heights[3 * 8 + 3], 10.0);
        assert!(read_buildings(None, &grid)
            .unwrap()
            .iter()
            .all(|v| *v == 0.0));
    }

    #[test]
    fn full_pipeline_uses_direct_paths_and_preserves_georeferencing() {
        let dir = TestDir::new();
        execute(setup(&dir.0)).unwrap();
        let output = read_raster(&dir.0.join("results/total_resistance.tif")).unwrap();
        require_aligned(&grid(), &output, Path::new("output")).unwrap();
        assert!(output.data.iter().all(|v| v.is_finite() && *v > 0.0));
        let hard = read_raster(&dir.0.join("results/hard_surf.tif")).unwrap();
        assert_eq!(hard.data[3 * 8 + 3], 10.0);
        assert_eq!(hard.data[0], 0.0);
        assert!(!dir.0.join("results/resistance.asc").exists());
        assert!(!dir.0.join("inputs.json").exists());
        assert!(!dir.0.join("grid_info.json").exists());
        assert!(!dir.0.join("results/source.asc").exists());
    }

    #[test]
    fn geopackage_and_geojson_rasterize_the_same_feature_values() {
        let dir = TestDir::new();
        let grid = grid();
        let json_path = dir.0.join("building.json");
        fs::write(&json_path, collection().to_string()).unwrap();
        let gpkg_path = dir.0.join("building.gpkg");
        gpkg(&gpkg_path, &grid);
        assert_eq!(
            vector_raster(Some(&json_path), &grid).unwrap(),
            vector_raster(Some(&gpkg_path), &grid).unwrap()
        );
        let mut args = setup(&dir.0);
        args.paths.insert("--buildings".into(), gpkg_path);
        execute(args).unwrap();
        assert_eq!(
            read_raster(&dir.0.join("results/hard_surf.tif"))
                .unwrap()
                .data[3 * 8 + 3],
            10.0
        );
    }

    #[test]
    fn malformed_supplied_vectors_and_crs_mismatches_are_errors() {
        let dir = TestDir::new();
        let grid = grid();
        let path = dir.0.join("building.json");
        let mut invalid_feature = collection();
        invalid_feature["features"][0]["properties"] = json!({});
        fs::write(&path, invalid_feature.to_string()).unwrap();
        assert!(vector_raster(Some(&path), &grid).is_err());
        let mut wrong_crs = collection();
        wrong_crs["crs"] = json!({"type":"name","properties":{"name":"EPSG:4326"}});
        fs::write(&path, wrong_crs.to_string()).unwrap();
        assert!(vector_raster(Some(&path), &grid).is_err());
        assert!(vector_raster(Some(&dir.0.join("missing.json")), &grid).is_err());
        let mut geographic = grid.clone();
        geographic.crs = Some(SpatialRef::from_epsg(4326).unwrap());
        let gpkg_path = dir.0.join("wrong.gpkg");
        gpkg(&gpkg_path, &geographic);
        assert!(vector_raster(Some(&gpkg_path), &grid).is_err());
    }

    #[test]
    fn misaligned_rasters_fail_before_creating_outputs() {
        let dir = TestDir::new();
        let args = setup(&dir.0);
        let mut shifted = grid();
        shifted.affine[0] = 0.5;
        write_tiff(&dir.0.join("dsm.tif"), &shifted, &vec![100.0; 64]).unwrap();
        assert!(execute(args).is_err());
        assert!(!dir.0.join("results").exists());
    }

    #[test]
    fn integer_landcover_and_nodata_are_read_without_resampling() {
        let dir = TestDir::new();
        let grid = grid();
        let path = dir.0.join("lcm.tif");
        {
            let mut ds = DriverManager::get_driver_by_name("GTiff")
                .unwrap()
                .create_with_band_type::<u8, _>(&path, 8, 8, 1)
                .unwrap();
            ds.set_geo_transform(&grid.affine).unwrap();
            ds.set_spatial_ref(grid.crs.as_ref().unwrap()).unwrap();
            let mut band = ds.rasterband(1).unwrap();
            band.set_no_data_value(Some(255.0)).unwrap();
            let mut values = vec![1_u8; 64];
            values[0] = 255;
            band.write((0, 0), (8, 8), &mut Buffer::new((8, 8), values))
                .unwrap();
        }
        let raster = read_raster(&path).unwrap();
        assert!(raster.data[0].is_nan());
        assert_eq!(raster.data[1], 1.0);
    }

    #[test]
    fn config_has_required_coefficients_without_grid_metadata_or_defaults() {
        let dir = TestDir::new();
        let args = setup(&dir.0);
        let mut incomplete = config();
        incomplete.as_object_mut().unwrap().remove("road_resmax");
        fs::write(dir.0.join("model.json"), incomplete.to_string()).unwrap();
        assert!(execute(args).is_err());
        assert!(!dir.0.join("results").exists());
        let mut bad = config();
        bad["nrows"] = json!(8);
        fs::write(dir.0.join("model.json"), bad.to_string()).unwrap();
        assert!(read_config(&dir.0.join("model.json")).is_err());
    }

    #[test]
    fn landscape_stage_requires_only_landscape_coefficients() {
        let dir = TestDir::new();
        let mut args = setup(&dir.0);
        args.landscape_only = true;
        fs::write(
            dir.0.join("model.json"),
            json!({"landscape_rankmax":8,"landscape_resmax":100,"landscape_xmax":5}).to_string(),
        )
        .unwrap();
        execute(args).unwrap();
        assert!(dir.0.join("results/landscape_conductance.tif").exists());
        assert!(dir.0.join("results/landscape_res.tif").exists());
        assert!(!dir.0.join("results/total_resistance.tif").exists());
    }

    #[test]
    fn flags_are_explicit_and_invalid_stage_values_are_rejected() {
        let parse =
            |values: &[&str]| parse_args(&values.iter().map(|v| v.to_string()).collect::<Vec<_>>());
        assert!(parse(&["work-directory"]).is_err());
        assert!(parse(&["--dtm"]).is_err());
        assert!(parse(&["--stage", "bad"]).is_err());
        assert!(parse(&["--config", "x", "--config", "y"]).is_err());
        assert!(parse(&[
            "--config",
            "model.json",
            "--dtm",
            "dtm.tif",
            "--dsm",
            "dsm.tif",
            "--lcm",
            "lcm.tif",
            "--output-dir",
            "results"
        ])
        .is_ok());
    }

    fn model_args(path: &Path, output: &Path) -> Args {
        parse_args(&[
            "--model".into(),
            path.to_str().unwrap().into(),
            "--output-dir".into(),
            output.to_str().unwrap().into(),
        ])
        .unwrap()
    }

    #[test]
    fn generic_cli_requires_only_the_model_grid_and_declared_inputs() {
        let dir = TestDir::new();
        let mut values = vec![2.0; 64];
        values[0] = 1.0;
        values[1] = f64::NAN;
        write_tiff(&dir.0.join("habitat.tif"), &grid(), &values).unwrap();
        let model = json!({
            "name":"another_species", "grid":"habitat", "missing_data":"propagate",
            "inputs":{"habitat":{"type":"raster","path":"habitat.tif"}},
            "layers": {
                "a_total":{"op":"sum","inputs":["z_habitat"]},
                "z_habitat":{"input":"habitat","operations":[
                    {"op":"discrete_map","intervals":{"[1,1]":3,"[2,2]":7},"unmatched":"error"}
                ]}
            },
            "output":"a_total"
        });
        let path = dir.0.join("model.json");
        fs::write(&path, model.to_string()).unwrap();
        let output = dir.0.join("results");
        execute(model_args(&path, &output)).unwrap();
        let raster = read_raster(&output.join("total_resistance.tif")).unwrap();
        require_aligned(&grid(), &raster, Path::new("output")).unwrap();
        assert_eq!(raster.data[0], 3.0);
        assert!(raster.data[1].is_nan());
        assert_eq!(raster.data[2], 7.0);
        assert!(!output.join("resistance.asc").exists());
        assert!(!output.join("grid_info.json").exists());
    }

    #[test]
    fn horseshoe_model_cli_matches_legacy_with_custom_vector_fields() {
        let dir = TestDir::new();
        let mut args = setup(&dir.0);
        let line = json!({"type":"FeatureCollection","features":[{
            "type":"Feature","properties":{"value":1,"presence":1},
            "geometry":{"type":"LineString","coordinates":[[1,1],[7,7]]}
        }]});
        let lamps = json!({"type":"FeatureCollection","features":[{
            "type":"Feature","properties":{"value":8,"height":8},
            "geometry":{"type":"Point","coordinates":[1,7]}
        }]});
        for (name, data) in [
            ("roads", line.clone()),
            ("rivers", line),
            ("lamps", lamps),
            ("zones", collection()),
        ] {
            let path = dir.0.join(format!("{name}.json"));
            fs::write(&path, data.to_string()).unwrap();
            args.paths.insert(
                if name == "zones" {
                    "--generic-resistance".into()
                } else {
                    format!("--{name}")
                },
                path,
            );
        }
        execute(args).unwrap();
        let mut model: Value =
            serde_json::from_str(include_str!("../../example/horseshoe-bat-model.json")).unwrap();
        for (name, path) in [
            ("dtm", "dtm.tif"),
            ("dsm", "dsm.tif"),
            ("landcover", "lcm.tif"),
            ("buildings", "buildings.json"),
            ("roads", "roads.json"),
            ("rivers", "rivers.json"),
            ("lamps", "lamps.json"),
            ("generic", "zones.json"),
        ] {
            model["inputs"][name]["path"] = json!(path);
        }
        model["inputs"]["roads"]["value_field"] = json!("presence");
        model["inputs"]["rivers"]["value_field"] = json!("presence");
        model["inputs"]["lamps"]["height_field"] = json!("height");
        let path = dir.0.join("generic-model.json");
        fs::write(&path, model.to_string()).unwrap();
        let output = dir.0.join("model-results");
        execute(model_args(&path, &output)).unwrap();
        let old = read_raster(&dir.0.join("results/total_resistance.tif")).unwrap();
        let new = read_raster(&output.join("total_resistance.tif")).unwrap();
        for (&old, &new) in old.data.iter().zip(&new.data) {
            assert!((old-new).abs() <= 1e-12 * old.abs().max(1.0));
        }
        assert_eq!(old.grid.affine, new.grid.affine);
        assert_eq!(old.grid.crs, new.grid.crs);
    }

    #[test]
    fn generic_vectors_use_explicit_background_and_selected_fields() {
        let dir = TestDir::new();
        write_tiff(&dir.0.join("grid.tif"), &grid(), &vec![0.0; 64]).unwrap();
        let mut features = collection();
        features["features"][0]["properties"] = json!({"cost":-2});
        fs::write(dir.0.join("zones.json"), features.to_string()).unwrap();
        let mut model = json!({
            "name":"direct", "grid":"grid", "missing_data":"propagate",
            "inputs":{
                "grid":{"type":"raster","path":"grid.tif"},
                "zones":{"type":"vector","path":"zones.json","value_field":"cost","background":5}
            },
            "layers":{}, "output":"zones"
        });
        let path = dir.0.join("model.json");
        fs::write(&path, model.to_string()).unwrap();
        let output = dir.0.join("results");
        execute(model_args(&path, &output)).unwrap();
        let raster = read_raster(&output.join("total_resistance.tif")).unwrap();
        assert_eq!(raster.data[0], 5.0);
        assert_eq!(raster.data[3 * 8 + 3], -2.0);
        // Also exercise field selection through the GeoPackage reader.
        gpkg(&dir.0.join("zones.gpkg"), &grid());
        model["inputs"]["zones"]["path"] = json!("zones.gpkg");
        model["inputs"]["zones"]["value_field"] = json!("value");
        fs::write(&path, model.to_string()).unwrap();
        let output = dir.0.join("gpkg-results");
        execute(model_args(&path, &output)).unwrap();
        assert_eq!(
            read_raster(&output.join("total_resistance.tif")).unwrap().data[3 * 8 + 3],
            10.0
        );
    }

    #[test]
    fn invalid_generic_models_fail_without_creating_outputs() {
        let dir = TestDir::new();
        write_tiff(&dir.0.join("grid.tif"), &grid(), &vec![0.0; 64]).unwrap();
        let mut model = json!({
            "name":"invalid", "grid":"grid", "missing_data":"propagate",
            "inputs":{"grid":{"type":"raster","path":"grid.tif"}},
            "layers":{"result":{"input":"grid","operations":[
                {"op":"discrete_map","intervals":{"[0,1]":1,"[1,2]":2},"unmatched":0}
            ]}}, "output":"result"
        });
        let path = dir.0.join("model.json");
        let output = dir.0.join("results");
        fs::write(&path, model.to_string()).unwrap();
        assert!(execute(model_args(&path, &output))
            .unwrap_err()
            .to_string()
            .contains("overlapping interval"));
        assert!(!output.exists());
        model["layers"]["result"]["input"] = json!("missing");
        fs::write(&path, model.to_string()).unwrap();
        assert!(execute(model_args(&path, &output))
            .unwrap_err()
            .to_string()
            .contains("unknown input/layer"));
        assert!(!output.exists());
        let mut wrong_grid = grid();
        wrong_grid.affine[0] += 1.0;
        write_tiff(&dir.0.join("shifted.tif"), &wrong_grid, &vec![0.0; 64]).unwrap();
        model["inputs"]["other"] = json!({"type":"raster","path":"shifted.tif"});
        fs::write(&path, model.to_string()).unwrap();
        assert!(execute(model_args(&path, &output))
            .unwrap_err()
            .to_string()
            .contains("not aligned"));
        assert!(!output.exists());
        assert!(parse_args(
            &[
                "--model",
                "model.json",
                "--config",
                "legacy.json",
                "--output-dir",
                "results"
            ]
            .map(String::from)
        )
        .is_err());
        assert!(parse_args(&["--model", "model.json"].map(String::from)).is_err());
    }


    #[test]
    fn all_layers_exports_named_rasters_and_surface_outputs_without_duplicating_total() {
        let dir = TestDir::new();
        write_tiff(&dir.0.join("terrain.tif"), &grid(), &vec![100.0; 64]).unwrap();
        let model = json!({
            "name":"layers", "grid":"terrain", "missing_data":"propagate",
            "inputs":{"terrain":{"type":"raster","path":"terrain.tif"}},
            "layers":{
                "habitat":{"input":"terrain","operations":[
                    {"op":"discrete_map","intervals":{"[100,100]":4},"unmatched":"error"}
                ]},
                "surfaces":{"op":"surface_heights","terrain":"terrain","surface":"terrain","buildings":"terrain"},
                "total_res":{"op":"sum","inputs":["habitat"]}
            },"output":"total_res"
        });
        let path = dir.0.join("model.json");
        fs::write(&path, model.to_string()).unwrap();
        let output = dir.0.join("results");
        execute(model_args(&path, &output)).unwrap();
        assert_eq!(fs::read_dir(&output).unwrap().count(), 1);
        let all_output = dir.0.join("all-results");
        let args = parse_args(&[
            "--model".into(),
            path.to_str().unwrap().into(),
            "--output-dir".into(),
            all_output.to_str().unwrap().into(),
            "--all-layers".into(),
        ])
        .unwrap();
        execute(args).unwrap();
        let mut names: Vec<_> = fs::read_dir(&all_output)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "habitat.tif",
                "surfaces.hard.tif",
                "surfaces.soft.tif",
                "total_resistance.tif"
            ]
        );
        for (name, value) in [
            ("habitat", 4.0),
            ("surfaces.soft", 0.0),
            ("surfaces.hard", 100.0),
            ("total_resistance", 4.0),
        ] {
            let raster = read_raster(&all_output.join(format!("{name}.tif"))).unwrap();
            require_aligned(&grid(), &raster, Path::new(name)).unwrap();
            assert_eq!(raster.data, vec![value; 64]);
        }
    }

}
