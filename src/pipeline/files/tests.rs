use super::super::model::schema::JsonPipeline;

use super::*;
use super::inputs::{lamps_from_collection, read_features, require_aligned};
use crate::pipeline::model::schema::Input as ModelInput;
use crate::raster::io::{read_raster, write_raster as write_tiff, RasterGrid as Grid};
use gdal::{spatial_ref::SpatialRef, vector::LayerAccess};
use serde_json::{json, Value};
use std::fs;
use gdal::vector::{Feature, Geometry, LayerOptions, OGRFieldType, OGRwkbGeometryType};
use gdal::{raster::Buffer, DriverManager};
use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = env::temp_dir().join(format!(
            "lmc-pipeline-test-{}-{}",
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
fn point_coordinates_are_converted_without_filtering_before_model_execution() {
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
        vec![
            -1.0, 4.0, 3.0, 8.0, 4.0, 3.0, 4.0, -1.0, 3.0, 4.0, 8.0, 3.0, 0.0, 0.0, 3.0, 4.0, 4.0,
            3.0
        ]
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
    serde_json::from_str(include_str!("../../../example/horseshoe-bat-model.json")).unwrap()
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
fn setup(dir: &Path) -> TestRun {
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
        read_features(&path, &grid(), Some("height"), false).unwrap()["features"][0]["properties"]
            ["height"]
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
        .insert("buildings".into(), dir.0.join("buildings.gpkg"));
    args.output_dir = dir.0.join("gpkg-results");
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

struct TestRun {
    model: JsonPipeline,
    paths: HashMap<String, PathBuf>,
    output_dir: PathBuf,
    all_layers: bool,
}
fn execute(run: &TestRun) -> Result<()> {
    let graph = PipelineGraph::from_definition(run.model.clone()).map_err(invalid)?;
    execute_pipeline_graph_from_files(
        &graph,
        &run.paths,
        &run.output_dir,
        ExecutionOptions {
            threads: 1,
            samples: 1,
            seed: 0,
            all_layers: run.all_layers,
            statistics: false,
        },
        false,
        None,
        None,
        |_| Ok(()),
    )?;
    Ok(())
}
fn model_args(path: &Path, output: &Path, inputs: &[(&str, &str)]) -> TestRun {
    TestRun {
        model: serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap(),
        paths: inputs
            .iter()
            .map(|(name, filename)| (name.to_string(), path.parent().unwrap().join(filename)))
            .collect(),
        output_dir: output.to_owned(),
        all_layers: false,
    }
}

#[test]
fn infers_grid_from_raster_inputs() {
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
        args.paths.insert(name.to_string(), dir.0.join(filename));
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
        let flag = name.to_string();
        let original = args
            .paths
            .insert(flag.clone(), dir.0.join("wrong.json"))
            .unwrap();
        let output = dir.0.join(format!("invalid-{name}"));
        args.output_dir = output.clone();
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
    crate::raster::io::write_raster(&dir.0.join("grid.asc"), &grid(), &vec![0.0; 64]).unwrap();
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
    let mut args = model_args(
        &path,
        &all_output,
        &[
            ("terrain", "terrain.tif"),
            ("surface", "terrain.tif"),
            ("buildings", "buildings.json"),
        ],
    );
    args.all_layers = true;
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
    args.output_dir = invalid.clone();
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
fn vector_mask_uses_line_and_polygon_geometry_without_attributes() {
    let dir = TestDir::new();
    let path = dir.0.join("model.json");
    let mut model = json!({
      "name": "geometry",
      "grid": {
        "crs": "EPSG:27700",
        "bounds": [0, 0, 8, 8],
        "cellsize": 1
      },
      "inputs": {
        "features": {
          "type": "vector_mask"
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
        // The input supplies a mask, without an implicit distance calculation.
        assert_eq!(raster.data[0], if name == "polygon" { 0.0 } else { 1.0 });
        assert_eq!(raster.data[3 * 8 + 3], 1.0);

        model["nodes"] = json!({"distance": {
            "op": "distance", "inputs": "inputs.features",
            "parameters": {"units": "metres", "when_empty": "infinity"}
        }});
        model["output"] = json!("nodes.distance");
        fs::write(&path, model.to_string()).unwrap();
        let distance_output = dir.0.join(format!("{name}-distance"));
        execute(&model_args(
            &path,
            &distance_output,
            &[("features", "features.json")],
        ))
        .unwrap();
        let distance = read_raster(&distance_output.join("total_resistance.tif")).unwrap();
        assert_eq!(distance.data[0], expected_corner);
        assert_eq!(distance.data[3 * 8 + 3], 0.0);
        model["nodes"] = json!({});
        model["output"] = json!("inputs.features");
        fs::write(&path, model.to_string()).unwrap();
    }
}
