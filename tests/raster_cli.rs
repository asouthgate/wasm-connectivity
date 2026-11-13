#![cfg(feature = "bin")]

use gdal::spatial_ref::SpatialRef;
use lm_connect::raster::io::{read_raster, write_raster, RasterGrid};
use serde_json::json;
use std::{fs, path::PathBuf, process::Command};

struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "lmc-raster-cli-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn resistance_output_flows_directly_into_all_current_solver_modes() {
    let dir = TestDir::new();
    let grid = RasterGrid {
        nrows: 8,
        ncols: 8,
        affine: [0.0, 1.0, 0.0, 8.0, 0.0, -1.0],
        crs: Some(SpatialRef::from_epsg(27700).unwrap()),
    };
    let mut resistance: Vec<f64> = (0..64).map(|i| 1.0 + (i % 7) as f64).collect();
    resistance[9] = f64::NAN;
    let mut source = vec![0.0; 64];
    source[0] = 1.0;
    source[1] = f64::NAN;
    let mut ground = vec![0.0; 64];
    ground[56..].fill(0.25);
    ground[2] = f64::NAN;
    for (name, values) in [
        ("resistance", &resistance),
        ("source", &source),
        ("ground", &ground),
    ] {
        for extension in ["tif", "asc"] {
            write_raster(&dir.0.join(format!("{name}.{extension}")), &grid, values).unwrap();
        }
    }
    let model = json!({
      "name": "test",
      "inputs": {
        "base": {
          "type": "raster_ranked"
        }
      },
      "nodes": {
        "total": {
          "input": "inputs.base",
          "operations": []
        }
      },
      "output": "nodes.total"
    });
    fs::create_dir(dir.0.join("models")).unwrap();
    fs::write(dir.0.join("models/model.json"), model.to_string()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lmc-resistance"))
        .current_dir(&dir.0)
        .args([
            "--model",
            "models/model.json",
            "--base",
            "resistance.tif",
            "--output-dir",
            "results",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let files: Vec<_> = fs::read_dir(dir.0.join("results"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        files,
        vec![std::ffi::OsString::from("total_resistance.tif")]
    );
    for (precision, explicit) in [
        ("f64", false),
        ("f32", false),
        ("mixed", false),
        ("f64", true),
        ("f32", true),
    ] {
        let mut outputs = Vec::new();
        for extension in ["tif", "asc"] {
            let result_file = format!("current-{precision}-{explicit}.{extension}");
            let mut command = Command::new(env!("CARGO_BIN_EXE_lmc-current"));
            command.current_dir(&dir.0).args([
                "--resistance",
                if extension == "tif" {
                    "results/total_resistance.tif"
                } else {
                    "resistance.asc"
                },
                "--source",
                &format!("source.{extension}"),
                "--ground",
                &format!("ground.{extension}"),
                "--precision",
                precision,
                "--l0postnu",
                "0",
                "--output",
                &result_file,
            ]);
            if explicit {
                command.arg("--explicit");
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{precision}/{explicit}/{extension}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let raster = read_raster(&dir.0.join(&result_file)).unwrap();
            assert_eq!(raster.grid.affine, grid.affine);
            assert_eq!(raster.grid.crs, grid.crs);
            assert!(raster.data[9].is_nan());
            assert!(raster
                .data
                .iter()
                .enumerate()
                .all(|(i, v)| i == 9 || v.is_finite()));
            assert!(raster.data.iter().any(|v| *v > 0.0));
            outputs.push(raster.data);
        }
        for (&a, &b) in outputs[0].iter().zip(&outputs[1]) {
            assert!(
                a.is_nan() && b.is_nan() || (a - b).abs() <= 1e-12 * a.abs().max(1.0),
                "{precision}: {a} != {b}"
            );
        }
    }
    // A shifted ground must fail at file validation, before solving or writing.
    let mut shifted = grid.clone();
    shifted.affine[0] += 1.0;
    write_raster(&dir.0.join("shifted.tif"), &shifted, &ground).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lmc-current"))
        .current_dir(&dir.0)
        .args([
            "--resistance",
            "results/total_resistance.tif",
            "--source",
            "source.tif",
            "--ground",
            "shifted.tif",
            "--output",
            "invalid.tif",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not aligned"));
    assert!(!dir.0.join("invalid.tif").exists());
}
