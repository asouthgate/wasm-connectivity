use gdal::{
    spatial_ref::SpatialRef,
    vector::{FieldValue, LayerAccess},
    Dataset,
};
use lm_connect::{
    geospatial::{rasterize_values_with_background, GeoTransform},
    raster::io::{read_raster, write_raster as write_tiff, Raster, RasterGrid as Grid},
    resistance::model::{Field as ModelField, Grid as ModelGrid, Input as ModelInput, Model},
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    env, fs, io,
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const USAGE: &str = "Usage: lmc-resistance --model model.json --output-dir results/ [--all-layers] [-v|--verbose]

Input rasters must be single-band, north-up, square-pixel grids aligned in the same projected CRS with metre units.
Vector inputs select a numeric value field; point inputs select a nonnegative height field.
GeoJSON coordinates must already use the raster CRS. GeoPackages must contain one vector layer in that CRS.
The --model JSON defines inline inputs and named operation layers; paths are relative to the model file.
--all-layers also writes each named raster layer and surface output.
The final output is total_resistance.tif. Outputs are georeferenced GeoTIFF layers.";

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

struct Args {
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
            "--model" | "--output-dir" => {
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
    Ok(Args {
        paths,
        all_layers,
        verbose,
    })
}

fn require_aligned(reference: &Grid, raster: &Raster, path: &Path) -> Result<()> {
    lm_connect::raster::io::require_aligned(reference, &raster.grid, path)
}

fn read_features(path: &Path, grid: &Grid, value_field: &str, nonnegative: bool) -> Result<Value> {
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

fn execute(args: &Args) -> Result<()> {
    let model_path = &args.paths["--model"];
    let model: Model = serde_json::from_str(&fs::read_to_string(model_path)?)
        .map_err(|e| invalid(format!("{}: {e}", model_path.display())))?;
    let directory = model_path
        .parent()
        .ok_or_else(|| invalid("model path has no parent"))?;
    let (reference_name, path) = model
        .inputs
        .iter()
        .find_map(|(name, input)| {
            if let ModelInput::Raster { path } = input {
                Some((name, path))
            } else {
                None
            }
        })
        .ok_or_else(|| invalid("model requires at least one raster input"))?;
    let reference = read_raster(&directory.join(path))?;
    let grid = &reference.grid;
    let mut fields = HashMap::new();
    for (name, input) in &model.inputs {
        if args.verbose {
            eprintln!("Reading input {name}");
        }
        let field = match input {
            ModelInput::Raster { path } => {
                if name == reference_name {
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
                let collection = read_features(&directory.join(path), grid, value_field, false)?;
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
                let collection = read_features(&directory.join(path), grid, height_field, true)?;
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
        json!({"type":"FeatureCollection","features":[{
            "type":"Feature", "properties":{"value":10},
            "geometry":{"type":"Polygon","coordinates":[[[2,2],[6,2],[6,6],[2,6],[2,2]]]}
        }]})
    }
    fn setup(dir: &Path) -> Args {
        for (name, value) in [("dtm", 100.0), ("dsm", 100.0), ("lcm", 1.0)] {
            write_tiff(&dir.join(format!("{name}.tif")), &grid(), &vec![value; 64]).unwrap();
        }
        fs::write(dir.join("buildings.json"), collection().to_string()).unwrap();
        fs::write(
            dir.join("empty.json"),
            json!({"type":"FeatureCollection","features":[]}).to_string(),
        )
        .unwrap();
        let mut model = model();
        for (name, path) in [
            ("dtm", "dtm.tif"),
            ("dsm", "dsm.tif"),
            ("landcover", "lcm.tif"),
            ("buildings", "buildings.json"),
            ("roads", "empty.json"),
            ("rivers", "empty.json"),
            ("generic", "empty.json"),
            ("lamps", "empty.json"),
        ] {
            model["inputs"][name]["path"] = json!(path);
        }
        let path = dir.join("model.json");
        fs::write(&path, model.to_string()).unwrap();
        let mut args = model_args(&path, &dir.join("results"));
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
    fn vector_fields_require_finite_numeric_values() {
        let dir = TestDir::new();
        let path = dir.0.join("buildings.json");
        let mut features = collection();
        for invalid in [json!({}), json!({"value":null}), json!({"value":"10"})] {
            features["features"][0]["properties"] = invalid;
            fs::write(&path, features.to_string()).unwrap();
            assert!(read_features(&path, &grid(), "value", false)
                .unwrap_err()
                .to_string()
                .contains("properties.value"));
        }
        features["features"][0]["properties"] = json!({"value":10});
        fs::write(&path, features.to_string()).unwrap();
        assert_eq!(
            read_features(&path, &grid(), "value", false).unwrap()["features"][0]["properties"]
                ["value"],
            json!(10.0)
        );
    }

    #[test]
    fn full_pipeline_uses_direct_paths_and_preserves_georeferencing() {
        let dir = TestDir::new();
        execute(&setup(&dir.0)).unwrap();
        let output = read_raster(&dir.0.join("results/total_resistance.tif")).unwrap();
        require_aligned(&grid(), &output, Path::new("output")).unwrap();
        assert!(output.data.iter().all(|v| v.is_finite() && *v > 0.0));
        let hard = read_raster(&dir.0.join("results/surfaces.hard.tif")).unwrap();
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
        let mut args = setup(&dir.0);
        execute(&args).unwrap();
        let json = read_raster(&dir.0.join("results/surfaces.hard.tif")).unwrap();
        gpkg(&dir.0.join("buildings.gpkg"), &grid());
        let mut model: Value =
            serde_json::from_str(&fs::read_to_string(dir.0.join("model.json")).unwrap()).unwrap();
        model["inputs"]["buildings"]["path"] = json!("buildings.gpkg");
        fs::write(dir.0.join("model.json"), model.to_string()).unwrap();
        args.paths
            .insert("--output-dir".into(), dir.0.join("gpkg-results"));
        execute(&args).unwrap();
        let gpkg = read_raster(&dir.0.join("gpkg-results/surfaces.hard.tif")).unwrap();
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
        assert!(read_features(&path, &grid, "value", false).is_err());
        let mut wrong_crs = collection();
        wrong_crs["crs"] = json!({"type":"name","properties":{"name":"EPSG:4326"}});
        fs::write(&path, wrong_crs.to_string()).unwrap();
        assert!(read_features(&path, &grid, "value", false).is_err());
        assert!(read_features(&dir.0.join("missing.json"), &grid, "value", false).is_err());
        let mut geographic = grid.clone();
        geographic.crs = Some(SpatialRef::from_epsg(4326).unwrap());
        let gpkg_path = dir.0.join("wrong.gpkg");
        gpkg(&gpkg_path, &geographic);
        assert!(read_features(&gpkg_path, &grid, "value", false).is_err());
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
    fn only_model_flags_are_accepted() {
        let parse =
            |values: &[&str]| parse_args(&values.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        for values in [
            vec!["--config", "old.json"],
            vec!["--dtm", "terrain.tif"],
            vec!["--stage", "landscape"],
            vec!["--model"],
            vec!["--model", "x", "--model", "y"],
            vec!["--all-layers", "--all-layers"],
            vec!["--output-dir", "results"],
        ] {
            assert!(parse(&values).is_err());
        }
        assert!(parse(&[
            "--model",
            "model.json",
            "--output-dir",
            "results",
            "--all-layers"
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
    fn generic_cli_infers_grid_from_raster_inputs() {
        let dir = TestDir::new();
        let mut values = vec![2.0; 64];
        values[0] = 1.0;
        values[1] = f64::NAN;
        write_tiff(&dir.0.join("habitat.tif"), &grid(), &values).unwrap();
        let model = json!({
            "name":"another_species",
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
        execute(&model_args(&path, &output)).unwrap();
        let raster = read_raster(&output.join("total_resistance.tif")).unwrap();
        require_aligned(&grid(), &raster, Path::new("output")).unwrap();
        assert_eq!(raster.data[0], 3.0);
        assert!(raster.data[1].is_nan());
        assert_eq!(raster.data[2], 7.0);
        assert!(!output.join("resistance.asc").exists());
        assert!(!output.join("grid_info.json").exists());
    }

    #[test]
    fn horseshoe_model_uses_custom_vector_fields() {
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
        let mut model: Value =
            serde_json::from_str(&fs::read_to_string(dir.0.join("model.json")).unwrap()).unwrap();
        for (name, data) in [
            ("roads", line.clone()),
            ("rivers", line),
            ("lamps", lamps),
            ("generic", collection()),
        ] {
            let filename = format!("{name}.json");
            fs::write(dir.0.join(&filename), data.to_string()).unwrap();
            model["inputs"][name]["path"] = json!(filename);
        }
        fs::write(dir.0.join("model.json"), model.to_string()).unwrap();
        execute(&args).unwrap();
        model["inputs"]["roads"]["value_field"] = json!("presence");
        model["inputs"]["rivers"]["value_field"] = json!("presence");
        model["inputs"]["lamps"]["height_field"] = json!("height");
        fs::write(dir.0.join("model.json"), model.to_string()).unwrap();
        args.paths
            .insert("--output-dir".into(), dir.0.join("custom-results"));
        execute(&args).unwrap();
        let original = read_raster(&dir.0.join("results/total_resistance.tif")).unwrap();
        let custom = read_raster(&dir.0.join("custom-results/total_resistance.tif")).unwrap();
        assert_eq!(original.data, custom.data);
    }

    #[test]
    fn generic_vectors_use_explicit_background_and_selected_fields() {
        let dir = TestDir::new();
        write_tiff(&dir.0.join("grid.tif"), &grid(), &vec![0.0; 64]).unwrap();
        let mut features = collection();
        features["features"][0]["properties"] = json!({"cost":-2});
        fs::write(dir.0.join("zones.json"), features.to_string()).unwrap();
        let mut model = json!({
            "name":"direct",
            "inputs":{
                "grid":{"type":"raster","path":"grid.tif"},
                "zones":{"type":"vector","path":"zones.json","value_field":"cost","background":5}
            },
            "layers":{}, "output":"zones"
        });
        let path = dir.0.join("model.json");
        fs::write(&path, model.to_string()).unwrap();
        let output = dir.0.join("results");
        execute(&model_args(&path, &output)).unwrap();
        let raster = read_raster(&output.join("total_resistance.tif")).unwrap();
        assert_eq!(raster.data[0], 5.0);
        assert_eq!(raster.data[3 * 8 + 3], -2.0);
        // Also exercise field selection through the GeoPackage reader.
        gpkg(&dir.0.join("zones.gpkg"), &grid());
        model["inputs"]["zones"]["path"] = json!("zones.gpkg");
        model["inputs"]["zones"]["value_field"] = json!("value");
        fs::write(&path, model.to_string()).unwrap();
        let output = dir.0.join("gpkg-results");
        execute(&model_args(&path, &output)).unwrap();
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
            "name":"invalid",
            "inputs":{"grid":{"type":"raster","path":"grid.tif"}},
            "layers":{"result":{"input":"grid","operations":[
                {"op":"discrete_map","intervals":{"[0,1]":1,"[1,2]":2},"unmatched":0}
            ]}}, "output":"result"
        });
        let path = dir.0.join("model.json");
        let output = dir.0.join("results");
        fs::write(&path, model.to_string()).unwrap();
        assert!(execute(&model_args(&path, &output))
            .unwrap_err()
            .to_string()
            .contains("overlapping interval"));
        assert!(!output.exists());
        model["layers"]["result"]["input"] = json!("missing");
        fs::write(&path, model.to_string()).unwrap();
        assert!(execute(&model_args(&path, &output))
            .unwrap_err()
            .to_string()
            .contains("unknown input/layer"));
        assert!(!output.exists());
        let mut wrong_grid = grid();
        wrong_grid.affine[0] += 1.0;
        write_tiff(&dir.0.join("shifted.tif"), &wrong_grid, &vec![0.0; 64]).unwrap();
        model["inputs"]["other"] = json!({"type":"raster","path":"shifted.tif"});
        fs::write(&path, model.to_string()).unwrap();
        assert!(execute(&model_args(&path, &output))
            .unwrap_err()
            .to_string()
            .contains("not aligned"));
        assert!(!output.exists());
        model["inputs"] = json!({});
        fs::write(&path, model.to_string()).unwrap();
        assert!(execute(&model_args(&path, &output))
            .unwrap_err()
            .to_string()
            .contains("requires at least one raster input"));
        assert!(!output.exists());
        assert!(parse_args(
            &[
                "--model",
                "model.json",
                "--config",
                "other.json",
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
            "name":"layers",
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
        execute(&model_args(&path, &output)).unwrap();
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
