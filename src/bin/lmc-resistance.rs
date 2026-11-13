use gdal::{
    spatial_ref::SpatialRef,
    vector::{FieldValue, LayerAccess},
    Dataset,
};
use lm_connect::{
    geospatial::{rasterize_field, GeoTransform},
    raster::io::{read_raster, write_raster as write_tiff, Raster, RasterGrid as Grid},
    resistance::{
        distance::distance_generation,
        model::{Field as ModelField, Grid as ModelGrid, Input as ModelInput, Model},
    },
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    env, fs, io,
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const USAGE: &str = "Usage: lmc-resistance --model model.json --<input-name> FILE ... --output-dir results/ [--all-layers] [-v|--verbose]

Raster inputs must be single-band GeoTIFFs with north-up square pixels and a projected CRS in metres.
An explicit model grid may crop or extend their bounds; CRS, cellsize and cell alignment must match.
Input types define their data requirements: vector_distance uses geometry; occluders and raycasters require height; vector_resistance requires resistance.
GeoJSON coordinates must already use the grid CRS. GeoPackages must contain one vector layer in that CRS.
The model defines input names and types. Each input requires a matching --<name> FILE flag.
File paths are relative to the working directory.
--all-layers also writes each named raster layer and surface output.
The final output is total_resistance.tif. Outputs are georeferenced GeoTIFF layers.";

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

struct Args {
    model: Model,
    paths: HashMap<String, PathBuf>,
    all_layers: bool,
    verbose: bool,
}

fn parse_args(arguments: &[String]) -> Result<Args> {
    let mut paths = HashMap::new();
    let mut verbose = false;
    let mut all_layers = false;
    let mut i = 0;
    while i < arguments.len() {
        let flag = &arguments[i];
        match flag.as_str() {
            "--all-layers" => {
                if all_layers {
                    return Err(invalid("duplicate --all-layers").into());
                }
                all_layers = true;
            }
            "-v" | "--verbose" => {
                verbose = true;
            }
            _ if flag.starts_with("--") && flag.len() > 2 => {
                let value = arguments
                    .get(i + 1)
                    .filter(|s| !s.starts_with('-'))
                    .ok_or_else(|| invalid(format!("missing value for {flag}")))?;
                if paths.insert(flag.clone(), PathBuf::from(value)).is_some() {
                    return Err(invalid(format!("duplicate argument: {flag}")).into());
                }
                i += 1;
            }
            _ => return Err(invalid(format!("unknown argument: {flag}")).into()),
        }
        i += 1;
    }
    for name in ["--model", "--output-dir"] {
        if !paths.contains_key(name) {
            return Err(invalid(format!("{name} is required")).into());
        }
    }
    let model_path = &paths["--model"];
    let model: Model = serde_json::from_str(&fs::read_to_string(model_path)?)
        .map_err(|e| invalid(format!("{}: {e}", model_path.display())))?;
    for name in model.inputs.keys() {
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(invalid(format!("invalid input name: {name}")).into());
        }
        if matches!(
            name.as_str(),
            "model" | "output-dir" | "all-layers" | "verbose" | "help"
        ) {
            return Err(invalid(format!(
                "input name {name} conflicts with a command-line option"
            ))
            .into());
        }
    }
    for flag in paths
        .keys()
        .filter(|flag| flag.as_str() != "--model" && flag.as_str() != "--output-dir")
    {
        if !model.inputs.contains_key(&flag[2..]) {
            return Err(invalid(format!("unknown input argument: {flag}")).into());
        }
    }
    for name in model.inputs.keys() {
        if !paths.contains_key(&format!("--{name}")) {
            return Err(invalid(format!("--{name} is required by model input {name}")).into());
        }
    }
    Ok(Args {
        model,
        paths,
        all_layers,
        verbose,
    })
}

fn require_aligned(reference: &Grid, raster: &Raster, path: &Path) -> Result<()> {
    lm_connect::raster::io::require_aligned(reference, &raster.grid, path)
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
        if col < 0.0 || row < 0.0 || col >= grid.ncols as f64 || row >= grid.nrows as f64 {
            continue;
        }
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

fn execute(args: &Args) -> Result<()> {
    let model = &args.model;
    let graph = model.compile().map_err(invalid)?;
    for (name, input) in &model.inputs {
        if input.is_raster() {
            let path = &args.paths[&format!("--{name}")];
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
    let grid = if let Some(spec) = &model.grid {
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
        let name = model
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
        let path = &args.paths[&format!("--{name}")];
        if args.verbose {
            eprintln!("Reading input {name}");
        }
        let reference = read_raster(path)?;
        fields.insert(name.clone(), ModelField::raster(reference.data));
        reference.grid
    };
    let grid = &grid;
    for (name, input) in &model.inputs {
        if fields.contains_key(name) {
            continue;
        }
        if args.verbose {
            eprintln!("Reading input {name}");
        }
        let path = &args.paths[&format!("--{name}")];
        let field = match input {
            ModelInput::RasterTopography {}
            | ModelInput::RasterSurface {}
            | ModelInput::RasterRanked {} => {
                let raster = read_raster(&path)?;
                let values = if model.grid.is_some() {
                    raster_on_grid(grid, raster, &path)?
                } else {
                    require_aligned(grid, &raster, &path)?;
                    raster.data
                };
                ModelField::raster(values)
            }
            ModelInput::VectorDistance {} => {
                let collection = read_features(path, grid, None, false)?;
                let transform = GeoTransform {
                    xmin: grid.affine[0],
                    ymax: grid.affine[3],
                    cellsize: grid.affine[1],
                };
                let mask = lm_connect::geospatial::rasterize_mask(
                    &collection.to_string(),
                    grid.nrows,
                    grid.ncols,
                    &transform,
                )?;
                let mut distance =
                    distance_generation(&mask, grid.nrows, grid.ncols, f64::INFINITY);
                for value in &mut distance {
                    *value *= grid.affine[1];
                }
                ModelField::raster(distance)
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
                ModelField::raster(values)
            }
            ModelInput::RaycasterPointSource {} => {
                let collection = read_features(path, grid, Some("height"), true)?;
                let points = lamps_from_collection(&collection, grid)?;
                let outside = collection["features"].as_array().unwrap().len() - points.len() / 3;
                if outside > 0 {
                    eprintln!(
                        "Warning: input {name}: {outside} light source(s) outside the grid were ignored and will not contribute irradiance."
                    );
                }
                ModelField::Points(std::sync::Arc::new(points))
            }
        };
        fields.insert(name.clone(), field);
    }
    if args.verbose {
        eprintln!("Evaluating model {}", model.name);
    }
    let (output, layers) = graph
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
        for (name, outputs) in layers {
            for (port, field) in outputs {
                if model.output.selects_node(&name, &port) {
                    continue;
                }
                if let ModelField::Raster(values) = field {
                    let filename = if port == "data" {
                        name.clone()
                    } else {
                        format!("{name}.{port}")
                    };
                    extra_layers.push((filename, values));
                }
            }
        }
        if extra_layers
            .iter()
            .any(|(name, _)| name == "total_resistance")
        {
            return Err(
                invalid("node total_resistance conflicts with the final output filename").into(),
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
    if let Err(e) = execute(&args) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gdal::vector::{Feature, Geometry, LayerOptions, OGRFieldType, OGRwkbGeometryType};
    use gdal::{raster::Buffer, DriverManager};
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

    #[test]
    fn raycaster_ignores_points_outside_the_grid() {
        let features: Vec<_> = [
            [-1.0, 4.0],
            [8.0, 4.0],
            [4.0, 9.0],
            [4.0, 0.0],
            [0.0, 8.0],
            [4.0, 4.0],
        ]
        .into_iter()
        .map(|coordinates| {
            json!({
                "type": "Feature", "properties": {"height": 3},
                "geometry": {"type": "Point", "coordinates": coordinates}
            })
        })
        .collect();
        let collection = json!({"type": "FeatureCollection", "features": features});
        assert_eq!(
            lamps_from_collection(&collection, &grid()).unwrap(),
            vec![0.0, 0.0, 3.0, 4.0, 4.0, 3.0]
        );
    }

    fn grid() -> Grid {
        Grid {
            ncols: 8,
            nrows: 8,
            affine: [0.0, 1.0, 0.0, 8.0, 0.0, -1.0],
            crs: Some(SpatialRef::from_epsg(27700).unwrap()),
        }
    }
    fn model() -> Value {
        serde_json::from_str(include_str!("../../example/horseshoe-bat-model.json")).unwrap()
    }
    fn collection() -> Value {
        json!({
          "type": "FeatureCollection",
          "features": [{
            "type": "Feature",
            "properties": {
              "height": 10
            },
            "geometry": {
              "type": "Polygon",
              "coordinates": [[[2, 2], [6, 2], [6, 6], [2, 6], [2, 2]]]
            }
          }]
        })
    }
    fn setup(dir: &Path) -> Args {
        for (name, value) in [("dtm", 100.0), ("dsm", 100.0), ("lcm", 1.0)] {
            write_tiff(&dir.join(format!("{name}.tif")), &grid(), &vec![value; 64]).unwrap();
        }
        fs::write(dir.join("buildings.json"), collection().to_string()).unwrap();
        fs::write(
            dir.join("empty.json"),
            json!({
              "type": "FeatureCollection",
              "features": []
            })
            .to_string(),
        )
        .unwrap();
        let model = model();
        let input_paths = [
            ("dtm", "dtm.tif"),
            ("dsm", "dsm.tif"),
            ("landcover", "lcm.tif"),
            ("buildings", "buildings.json"),
            ("roads", "empty.json"),
            ("rivers", "empty.json"),
            ("generic", "empty.json"),
            ("lamps", "empty.json"),
        ];
        let path = dir.join("model.json");
        fs::write(&path, model.to_string()).unwrap();
        let mut args = model_args(&path, &dir.join("results"), &input_paths);
        args.all_layers = true;
        args
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
            .create_defn_fields(&[("height", OGRFieldType::OFTReal)])
            .unwrap();
        let mut feature = Feature::new(layer.defn()).unwrap();
        feature
            .set_field_double(feature.field_index("height").unwrap(), 10.0)
            .unwrap();
        feature
            .set_geometry(Geometry::from_wkt("POLYGON ((2 2,6 2,6 6,2 6,2 2))").unwrap())
            .unwrap();
        feature.create(&layer).unwrap();
    }

    #[test]
    fn vector_fields_require_finite_numeric_values() {
        let dir = TestDir::new();
        let path = dir.0.join("buildings.json");
        let mut features = collection();
        for invalid in [
            json!({}),
            json!({
              "height": null
            }),
            json!({
              "height": "10"
            }),
        ] {
            features["features"][0]["properties"] = invalid;
            fs::write(&path, features.to_string()).unwrap();
            assert!(read_features(&path, &grid(), Some("height"), false)
                .unwrap_err()
                .to_string()
                .contains("properties.height"));
        }
        features["features"][0]["properties"] = json!({
          "height": 10
        });
        fs::write(&path, features.to_string()).unwrap();
        assert_eq!(
            read_features(&path, &grid(), Some("height"), false).unwrap()["features"][0]
                ["properties"]["height"]
                .as_f64(),
            Some(10.0)
        );
    }

    #[test]
    fn full_pipeline_uses_direct_paths_and_preserves_georeferencing() {
        let dir = TestDir::new();
        execute(&setup(&dir.0)).unwrap();
        let output = read_raster(&dir.0.join("results/total_resistance.tif")).unwrap();
        require_aligned(&grid(), &output, Path::new("output")).unwrap();
        assert!(output.data.iter().all(|v| v.is_finite() && *v > 0.0));
        let opaque = read_raster(&dir.0.join("results/surfaces.opaque.tif")).unwrap();
        assert_eq!(opaque.data[3 * 8 + 3], 10.0);
        assert_eq!(opaque.data[0], 0.0);
        assert!(!dir.0.join("results/resistance.asc").exists());
        assert!(!dir.0.join("inputs.json").exists());
        assert!(!dir.0.join("grid_info.json").exists());
        assert!(!dir.0.join("results/source.asc").exists());
    }

    #[test]
    fn geopackage_and_geojson_rasterize_the_same_feature_values() {
        let dir = TestDir::new();
        let mut args = setup(&dir.0);
        execute(&args).unwrap();
        let json = read_raster(&dir.0.join("results/surfaces.opaque.tif")).unwrap();
        gpkg(&dir.0.join("buildings.gpkg"), &grid());
        args.paths
            .insert("--buildings".into(), dir.0.join("buildings.gpkg"));
        args.paths
            .insert("--output-dir".into(), dir.0.join("gpkg-results"));
        execute(&args).unwrap();
        let gpkg = read_raster(&dir.0.join("gpkg-results/surfaces.opaque.tif")).unwrap();
        assert_eq!(json.data, gpkg.data);
        assert_eq!(gpkg.data[3 * 8 + 3], 10.0);
    }

    #[test]
    fn malformed_supplied_vectors_and_crs_mismatches_are_errors() {
        let dir = TestDir::new();
        let grid = grid();
        let path = dir.0.join("building.json");
        let mut invalid_feature = collection();
        invalid_feature["features"][0]["properties"] = json!({});
        fs::write(&path, invalid_feature.to_string()).unwrap();
        assert!(read_features(&path, &grid, Some("height"), false).is_err());
        let mut wrong_crs = collection();
        wrong_crs["crs"] = json!({
          "type": "name",
          "properties": {
            "name": "EPSG:4326"
          }
        });
        fs::write(&path, wrong_crs.to_string()).unwrap();
        assert!(read_features(&path, &grid, Some("height"), false).is_err());
        assert!(read_features(&dir.0.join("missing.json"), &grid, Some("height"), false).is_err());
        let mut geographic = grid.clone();
        geographic.crs = Some(SpatialRef::from_epsg(4326).unwrap());
        let gpkg_path = dir.0.join("wrong.gpkg");
        gpkg(&gpkg_path, &geographic);
        assert!(read_features(&gpkg_path, &grid, Some("height"), false).is_err());
    }

    #[test]
    fn misaligned_rasters_fail_before_creating_outputs() {
        let dir = TestDir::new();
        let args = setup(&dir.0);
        let mut shifted = grid();
        shifted.affine[0] = 0.5;
        write_tiff(&dir.0.join("dsm.tif"), &shifted, &vec![100.0; 64]).unwrap();
        assert!(execute(&args).is_err());
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
    fn input_flags_are_inferred_from_the_model() {
        let dir = TestDir::new();
        let path = dir.0.join("model.json");
        let mut model = json!({
          "name": "custom",
          "inputs": {
            "canopy_height": {
              "type": "raster_ranked"
            }
          },
          "nodes": {},
          "output": "inputs.canopy_height"
        });
        fs::write(&path, model.to_string()).unwrap();
        let base = vec![
            "--output-dir".to_string(),
            "results".into(),
            "--model".into(),
            path.to_str().unwrap().into(),
        ];
        assert!(parse_args(&base)
            .err()
            .unwrap()
            .to_string()
            .contains("--canopy_height is required"));
        let mut valid = vec!["--canopy_height".into(), "height.tif".into()];
        valid.extend(base.clone());
        assert!(parse_args(&valid).is_ok());
        for extra in [
            vec!["--dsm", "dsm.tif"],
            vec!["--canopy_height", "again.tif"],
            vec!["--all-layers", "--all-layers"],
            vec!["--config", "old.json"],
        ] {
            let mut arguments = valid.clone();
            arguments.extend(extra.into_iter().map(String::from));
            assert!(parse_args(&arguments).is_err());
        }
        assert!(parse_args(&["--model".into()]).is_err());
        model["inputs"] = json!({
          "output-dir": {
            "type": "raster_ranked"
          }
        });
        fs::write(&path, model.to_string()).unwrap();
        assert!(parse_args(&base)
            .err()
            .unwrap()
            .to_string()
            .contains("conflicts"));
        model["inputs"] = json!({
          "canopy_height": {
            "type": "raster_ranked",
            "path": "height.tif"
          }
        });
        fs::write(&path, model.to_string()).unwrap();
        assert!(parse_args(&valid)
            .err()
            .unwrap()
            .to_string()
            .contains("unknown field"));
    }

    fn model_args(path: &Path, output: &Path, inputs: &[(&str, &str)]) -> Args {
        let mut arguments = vec![
            "--model".into(),
            path.to_str().unwrap().into(),
            "--output-dir".into(),
            output.to_str().unwrap().into(),
        ];
        for (name, filename) in inputs {
            arguments.push(format!("--{name}"));
            arguments.push(
                path.parent()
                    .unwrap()
                    .join(filename)
                    .to_str()
                    .unwrap()
                    .into(),
            );
        }
        parse_args(&arguments).unwrap()
    }

    #[test]
    fn generic_cli_infers_grid_from_raster_inputs() {
        let dir = TestDir::new();
        let mut values = vec![2.0; 64];
        values[0] = 1.0;
        values[1] = f64::NAN;
        write_tiff(&dir.0.join("habitat.tif"), &grid(), &values).unwrap();
        let model = json!({
          "name": "another_species",
          "inputs": {
            "canopy_height": {
              "type": "raster_ranked"
            }
          },
          "nodes": {
            "a_total": {
              "op": "sum",
              "inputs": ["nodes.z_canopy_height"]
            },
            "z_canopy_height": {
              "input": "inputs.canopy_height",
              "operations": [{
                "op": "discrete_map",
                "parameters": {
                  "intervals": {
                    "[1,1]": 3,
                    "[2,2]": 7
                  },
                  "unmatched": "error"
                }
              }]
            }
          },
          "output": "nodes.a_total"
        });
        let path = dir.0.join("model.json");
        fs::write(&path, model.to_string()).unwrap();
        let output = dir.0.join("results");
        execute(&model_args(
            &path,
            &output,
            &[("canopy_height", "habitat.tif")],
        ))
        .unwrap();
        let raster = read_raster(&output.join("total_resistance.tif")).unwrap();
        require_aligned(&grid(), &raster, Path::new("output")).unwrap();
        assert_eq!(raster.data[0], 3.0);
        assert!(raster.data[1].is_nan());
        assert_eq!(raster.data[2], 7.0);
        assert!(!output.join("resistance.asc").exists());
        assert!(!output.join("grid_info.json").exists());
    }

    #[test]
    fn horseshoe_model_requires_native_fields() {
        let dir = TestDir::new();
        let mut args = setup(&dir.0);
        let line = json!({
          "type": "FeatureCollection",
          "features": [{
            "type": "Feature",
            "properties": {
              "presence": 1
            },
            "geometry": {
              "type": "LineString",
              "coordinates": [[1, 1], [7, 7]]
            }
          }]
        });
        let lamps = json!({
          "type": "FeatureCollection",
          "features": [{
            "type": "Feature",
            "properties": {
              "height": 8
            },
            "geometry": {
              "type": "Point",
              "coordinates": [1, 7]
            }
          }]
        });
        for (name, data) in [("roads", line.clone()), ("rivers", line), ("lamps", lamps)] {
            let filename = format!("{name}.json");
            fs::write(dir.0.join(&filename), data.to_string()).unwrap();
            args.paths.insert(format!("--{name}"), dir.0.join(filename));
        }
        execute(&args).unwrap();
        for (name, field) in [
            ("buildings", "height"),
            ("lamps", "height"),
            ("generic", "resistance"),
        ] {
            let mut wrong = collection();
            wrong["features"][0]["properties"] = json!({
              "value": 10
            });
            if name == "lamps" {
                wrong["features"][0]["geometry"] = json!({
                  "type": "Point",
                  "coordinates": [1, 7]
                });
            }
            fs::write(dir.0.join("wrong.json"), wrong.to_string()).unwrap();
            let flag = format!("--{name}");
            let original = args
                .paths
                .insert(flag.clone(), dir.0.join("wrong.json"))
                .unwrap();
            let output = dir.0.join(format!("invalid-{name}"));
            args.paths.insert("--output-dir".into(), output.clone());
            assert!(execute(&args)
                .unwrap_err()
                .to_string()
                .contains(&format!("properties.{field}")));
            assert!(!output.exists());
            args.paths.insert(flag, original);
        }
    }

    #[test]
    fn vector_types_read_their_fixed_fields_with_zero_outside_features() {
        let dir = TestDir::new();
        write_tiff(&dir.0.join("grid.tif"), &grid(), &vec![0.0; 64]).unwrap();
        let mut features = collection();
        features["features"][0]["properties"] = json!({
          "resistance": -2
        });
        fs::write(dir.0.join("zones.json"), features.to_string()).unwrap();
        let mut model = json!({
          "name": "direct",
          "inputs": {
            "grid": {
              "type": "raster_ranked"
            },
            "zones": {
              "type": "vector_resistance"
            }
          },
          "nodes": {},
          "output": "inputs.zones"
        });
        let path = dir.0.join("model.json");
        fs::write(&path, model.to_string()).unwrap();
        let output = dir.0.join("results");
        execute(&model_args(
            &path,
            &output,
            &[("grid", "grid.tif"), ("zones", "zones.json")],
        ))
        .unwrap();
        let raster = read_raster(&output.join("total_resistance.tif")).unwrap();
        assert_eq!(raster.data[0], 0.0);
        assert_eq!(raster.data[3 * 8 + 3], -2.0);
        // Also exercise field selection through the GeoPackage reader.
        gpkg(&dir.0.join("zones.gpkg"), &grid());
        model["inputs"]["zones"] = json!({
          "type": "vector_occluder"
        });
        fs::write(&path, model.to_string()).unwrap();
        let output = dir.0.join("gpkg-results");
        execute(&model_args(
            &path,
            &output,
            &[("grid", "grid.tif"), ("zones", "zones.gpkg")],
        ))
        .unwrap();
        assert_eq!(
            read_raster(&output.join("total_resistance.tif"))
                .unwrap()
                .data[3 * 8 + 3],
            10.0
        );
    }

    #[test]
    fn invalid_generic_models_fail_without_creating_outputs() {
        let dir = TestDir::new();
        write_tiff(&dir.0.join("grid.tif"), &grid(), &vec![0.0; 64]).unwrap();
        let mut model = json!({
          "name": "invalid",
          "inputs": {
            "grid": {
              "type": "raster_ranked"
            }
          },
          "nodes": {
            "result": {
              "input": "inputs.grid",
              "operations": [{
                "op": "discrete_map",
                "parameters": {
                  "intervals": {
                    "[0,1]": 1,
                    "[1,2]": 2
                  },
                  "unmatched": 0
                }
              }]
            }
          },
          "output": "nodes.result"
        });
        let path = dir.0.join("model.json");
        let output = dir.0.join("results");
        fs::write(&path, model.to_string()).unwrap();
        let args = model_args(&path, &output, &[("grid", "grid.tif")]);
        assert!(execute(&args)
            .unwrap_err()
            .to_string()
            .contains("overlapping interval"));
        assert!(!output.exists());
        model["nodes"]["result"]["input"] = json!("inputs.missing");
        fs::write(&path, model.to_string()).unwrap();
        assert!(
            execute(&model_args(&path, &output, &[("grid", "grid.tif")]))
                .unwrap_err()
                .to_string()
                .contains("unknown input")
        );
        model["nodes"]["result"]["input"] = json!("inputs.grid");
        model["nodes"]["result"]["operations"] = json!([]);
        let mut shifted = grid();
        shifted.affine[0] += 1.0;
        write_tiff(&dir.0.join("shifted.tif"), &shifted, &vec![0.0; 64]).unwrap();
        model["inputs"]["other"] = json!({
          "type": "raster_ranked"
        });
        fs::write(&path, model.to_string()).unwrap();
        assert!(execute(&model_args(
            &path,
            &output,
            &[("grid", "grid.tif"), ("other", "shifted.tif")]
        ))
        .unwrap_err()
        .to_string()
        .contains("not aligned"));
        lm_connect::raster::io::write_raster(&dir.0.join("grid.asc"), &grid(), &vec![0.0; 64])
            .unwrap();
        for inputs in [
            vec![("grid", "grid.asc"), ("other", "shifted.tif")],
            vec![("grid", "grid.tif"), ("other", "grid.asc")],
        ] {
            assert!(execute(&model_args(&path, &output, &inputs))
                .unwrap_err()
                .to_string()
                .contains("resistance raster inputs must be GeoTIFF"));
        }
        model["inputs"] = json!({"zones":{"type":"vector_occluder"}});
        model["nodes"] = json!({});
        model["output"] = json!("inputs.zones");
        fs::write(&path, model.to_string()).unwrap();
        assert!(
            execute(&model_args(&path, &output, &[("zones", "zones.json")]))
                .unwrap_err()
                .to_string()
                .contains("requires at least one raster input")
        );
        assert!(!output.exists());
    }

    #[test]
    fn all_layers_exports_named_rasters_and_surface_outputs_without_duplicating_total() {
        let dir = TestDir::new();
        write_tiff(&dir.0.join("terrain.tif"), &grid(), &vec![100.0; 64]).unwrap();
        let mut buildings = collection();
        buildings["features"][0]["properties"]["height"] = json!(100);
        buildings["features"][0]["geometry"]["coordinates"] =
            json!([[[0, 0], [8, 0], [8, 8], [0, 8], [0, 0]]]);
        fs::write(dir.0.join("buildings.json"), buildings.to_string()).unwrap();
        let model = json!({
          "name": "layers",
          "inputs": {
            "terrain": {
              "type": "raster_topography"
            },
            "surface": {
              "type": "raster_surface"
            },
            "buildings": {
              "type": "vector_occluder"
            }
          },
          "nodes": {
            "habitat": {
              "input": "inputs.terrain",
              "operations": [{
                "op": "discrete_map",
                "parameters": {
                  "intervals": {
                    "[100,100]": 4
                  },
                  "unmatched": "error"
                }
              }]
            },
            "surfaces": {
              "op": "surface_heights",
              "inputs": {
                "topography": "inputs.terrain",
                "surface": "inputs.surface",
                "occluders": "inputs.buildings"
              }
            },
            "total_res": {
              "op": "sum",
              "inputs": ["nodes.habitat"]
            }
          },
          "output": "nodes.total_res"
        });
        let path = dir.0.join("model.json");
        fs::write(&path, model.to_string()).unwrap();
        let output = dir.0.join("results");
        execute(&model_args(
            &path,
            &output,
            &[
                ("terrain", "terrain.tif"),
                ("surface", "terrain.tif"),
                ("buildings", "buildings.json"),
            ],
        ))
        .unwrap();
        assert_eq!(fs::read_dir(&output).unwrap().count(), 1);
        let all_output = dir.0.join("all-results");
        let mut args = parse_args(&[
            "--model".into(),
            path.to_str().unwrap().into(),
            "--output-dir".into(),
            all_output.to_str().unwrap().into(),
            "--terrain".into(),
            dir.0.join("terrain.tif").to_str().unwrap().into(),
            "--surface".into(),
            dir.0.join("terrain.tif").to_str().unwrap().into(),
            "--buildings".into(),
            dir.0.join("buildings.json").to_str().unwrap().into(),
            "--all-layers".into(),
        ])
        .unwrap();
        execute(&args).unwrap();
        let mut names: Vec<_> = fs::read_dir(&all_output)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "habitat.tif",
                "surfaces.opaque.tif",
                "surfaces.translucent.tif",
                "total_resistance.tif"
            ]
        );
        for (name, value) in [
            ("habitat", 4.0),
            ("surfaces.translucent", 0.0),
            ("surfaces.opaque", 100.0),
            ("total_resistance", 4.0),
        ] {
            let raster = read_raster(&all_output.join(format!("{name}.tif"))).unwrap();
            require_aligned(&grid(), &raster, Path::new(name)).unwrap();
            assert_eq!(raster.data, vec![value; 64]);
        }
        args.model
            .inputs
            .insert("surface".into(), ModelInput::RasterRanked {});
        let invalid = dir.0.join("invalid-type");
        args.paths.insert("--output-dir".into(), invalid.clone());
        assert!(execute(&args)
            .unwrap_err()
            .to_string()
            .contains("must be raster_surface"));
        assert!(!invalid.exists());
    }

    #[test]
    fn explicit_grid_supports_vector_only_models() {
        let dir = TestDir::new();
        fs::write(dir.0.join("zones.json"), collection().to_string()).unwrap();
        let model = json!({
          "name": "vector_only",
          "grid": {
            "crs": "EPSG:27700",
            "bounds": [0, 0, 8, 8],
            "cellsize": 1
          },
          "inputs": {
            "zones": {
              "type": "vector_occluder"
            }
          },
          "nodes": {},
          "output": "inputs.zones"
        });
        let path = dir.0.join("model.json");
        fs::write(&path, model.to_string()).unwrap();
        let output = dir.0.join("results");
        execute(&model_args(&path, &output, &[("zones", "zones.json")])).unwrap();
        let raster = read_raster(&output.join("total_resistance.tif")).unwrap();
        require_aligned(&grid(), &raster, Path::new("result")).unwrap();
        assert_eq!(raster.data[0], 0.0);
        assert_eq!(raster.data[3 * 8 + 3], 10.0);
    }

    #[test]
    fn explicit_grid_crops_pads_and_accepts_nonoverlapping_rasters() {
        let dir = TestDir::new();
        let values: Vec<f64> = (0..64).map(|v| v as f64).collect();
        write_tiff(&dir.0.join("data.tif"), &grid(), &values).unwrap();
        let mut model = json!({
          "name": "bounds",
          "grid": {
            "crs": "EPSG:27700",
            "bounds": [2, 2, 6, 6],
            "cellsize": 1
          },
          "inputs": {
            "data": {
              "type": "raster_ranked"
            }
          },
          "nodes": {},
          "output": "inputs.data"
        });
        let path = dir.0.join("model.json");
        for (name, bounds, size) in [
            ("cropped", [2, 2, 6, 6], 4),
            ("padded", [-2, -2, 10, 10], 12),
            ("disjoint", [20, 20, 28, 28], 8),
        ] {
            model["grid"]["bounds"] = json!(bounds);
            fs::write(&path, model.to_string()).unwrap();
            let output = dir.0.join(name);
            execute(&model_args(&path, &output, &[("data", "data.tif")])).unwrap();
            let raster = read_raster(&output.join("total_resistance.tif")).unwrap();
            assert_eq!((raster.grid.nrows, raster.grid.ncols), (size, size));
            assert_eq!(
                raster.grid.affine,
                [bounds[0] as f64, 1.0, 0.0, bounds[3] as f64, 0.0, -1.0]
            );
            for row in 0..size {
                for col in 0..size {
                    let x = bounds[0] + col as i32;
                    let y = 8 - bounds[3] + row as i32;
                    let actual = raster.data[row * size + col];
                    if (0..8).contains(&x) && (0..8).contains(&y) {
                        assert_eq!(actual, values[y as usize * 8 + x as usize]);
                    } else {
                        assert!(actual.is_nan());
                    }
                }
            }
        }
        for (name, spec, expected) in [
            (
                "cellsize",
                json!({
                  "crs": "EPSG:27700",
                  "bounds": [0, 0, 8, 8],
                  "cellsize": 2
                }),
                "CRS and cellsize",
            ),
            (
                "crs",
                json!({
                  "crs": "EPSG:32630",
                  "bounds": [0, 0, 8, 8],
                  "cellsize": 1
                }),
                "CRS and cellsize",
            ),
            (
                "alignment",
                json!({
                  "crs": "EPSG:27700",
                  "bounds": [0.5, 0, 8.5, 8],
                  "cellsize": 1
                }),
                "cells must align",
            ),
            (
                "extent",
                json!({
                  "crs": "EPSG:27700",
                  "bounds": [0, 0, 8.5, 8],
                  "cellsize": 1
                }),
                "whole cells",
            ),
            (
                "geographic",
                json!({
                  "crs": "EPSG:4326",
                  "bounds": [0, 0, 8, 8],
                  "cellsize": 1
                }),
                "projected with metre",
            ),
            (
                "zero",
                json!({
                  "crs": "EPSG:27700",
                  "bounds": [0, 0, 8, 8],
                  "cellsize": 0
                }),
                "positive cellsize",
            ),
            (
                "reversed",
                json!({
                  "crs": "EPSG:27700",
                  "bounds": [8, 0, 0, 8],
                  "cellsize": 1
                }),
                "ordered bounds",
            ),
        ] {
            model["grid"] = spec;
            fs::write(&path, model.to_string()).unwrap();
            let output = dir.0.join(name);
            let error = execute(&model_args(&path, &output, &[("data", "data.tif")]))
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{name}: {error}");
            assert!(!output.exists());
        }
    }

    #[test]
    fn vector_distance_uses_line_and_polygon_geometry_without_attributes() {
        let dir = TestDir::new();
        let path = dir.0.join("model.json");
        let model = json!({
          "name": "geometry",
          "grid": {
            "crs": "EPSG:27700",
            "bounds": [0, 0, 8, 8],
            "cellsize": 1
          },
          "inputs": {
            "features": {
              "type": "vector_distance"
            }
          },
          "nodes": {},
          "output": "inputs.features"
        });
        fs::write(&path, model.to_string()).unwrap();
        for (name, geometry, expected_corner) in [
            (
                "polygon",
                json!({
                  "type": "Polygon",
                  "coordinates": [[[2, 2], [6, 2], [6, 6], [2, 6], [2, 2]]]
                }),
                8.0f64.sqrt(),
            ),
            (
                "line",
                json!({
                  "type": "LineString",
                  "coordinates": [[0, 8], [7, 1]]
                }),
                0.0,
            ),
        ] {
            let features = json!({
              "type": "FeatureCollection",
              "features": [{
                "type": "Feature",
                "properties": {},
                "geometry": geometry
              }]
            });
            fs::write(dir.0.join("features.json"), features.to_string()).unwrap();
            let output = dir.0.join(name);
            execute(&model_args(
                &path,
                &output,
                &[("features", "features.json")],
            ))
            .unwrap();
            let raster = read_raster(&output.join("total_resistance.tif")).unwrap();
            assert_eq!(raster.data[0], expected_corner);
            assert_eq!(raster.data[3 * 8 + 3], 0.0);
        }
    }
}
