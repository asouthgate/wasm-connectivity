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
    let output = Command::new(env!("CARGO_BIN_EXE_lmc"))
        .arg("resistance")
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
            let mut command = Command::new(env!("CARGO_BIN_EXE_lmc"));
            command.arg("current");
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
    // Full mode with one sample must match solving its resistance output directly.
    let full = Command::new(env!("CARGO_BIN_EXE_lmc"))
        .current_dir(&dir.0)
        .args(["full", "--model", "models/model.json", "--base", "resistance.tif",
            "--source", "source.tif", "--ground", "ground.tif", "--output-dir", "full"])
        .output().unwrap();
    assert!(full.status.success(), "{}", String::from_utf8_lossy(&full.stderr));
    assert!(full.stderr.is_empty());
    let direct = Command::new(env!("CARGO_BIN_EXE_lmc"))
        .current_dir(&dir.0)
        .args(["current", "--resistance", "full/total_resistance.tif", "--source", "source.tif",
            "--ground", "ground.tif", "--output", "direct.tif"])
        .output().unwrap();
    assert!(direct.status.success(), "{}", String::from_utf8_lossy(&direct.stderr));
    let full_current = read_raster(&dir.0.join("full/current.tif")).unwrap();
    let direct_current = read_raster(&dir.0.join("direct.tif")).unwrap();
    for (&full, &direct) in full_current.data.iter().zip(&direct_current.data) {
        assert!(full.is_nan() && direct.is_nan() || full == direct);
    }
    assert!(!dir.0.join("full/current_variance.tif").exists());
    // A shifted ground must fail at file validation, before solving or writing.
    let mut shifted = grid.clone();
    shifted.affine[0] += 1.0;
    write_raster(&dir.0.join("shifted.tif"), &shifted, &ground).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lmc"))
        .arg("current")
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

#[test]
fn sampled_resistance_and_current_are_accumulated_and_optionally_saved() {
    let dir = TestDir::new();
    let grid = RasterGrid {
        nrows: 8, ncols: 8, affine: [0.0,1.0,0.0,8.0,0.0,-1.0],
        crs: Some(SpatialRef::from_epsg(27700).unwrap()),
    };
    let data: Vec<f64> = (0..64).map(|i|1.0+(i%4) as f64/4.0).collect();
    let mut source = vec![0.0;64];source[0]=1.0;
    let mut ground = vec![0.0;64];ground[63]=1.0;
    for (name, values) in [("data",data),("source",source),("ground",ground)] {
        write_raster(&dir.0.join(format!("{name}.tif")),&grid,&values).unwrap();
    }
    let model = json!({"name":"samples","inputs":{"data":{"type":"raster_ranked"}},
        "nodes":{
            "fixed":{"op":"round","inputs":"inputs.data"},
            "resistance":{"op":"power_response","inputs":"nodes.fixed","parameters":{
                "input_range":[1,2],"base_range":[1,2],"exponent":{"sample":{"values":[1,2,3]}},"scale":1,"offset":1
            }}
        },"output":"nodes.resistance"});
    std::fs::write(dir.0.join("model.json"),model.to_string()).unwrap();
    let run = |mode: &str, directory: &str, save: bool, all_layers: bool, threads: usize| {
        let mut command=Command::new(env!("CARGO_BIN_EXE_lmc"));
        command.arg(mode).arg("--model").arg(dir.0.join("model.json"))
            .arg("--data").arg(dir.0.join("data.tif"))
            .arg("--output-dir").arg(dir.0.join(directory))
            .args(["--samples","3"])
            .arg("--threads").arg(threads.to_string());
        if all_layers { command.arg("--all-layers"); }
        if mode == "full" {
            command.arg("--source").arg(dir.0.join("source.tif"))
                .arg("--ground").arg(dir.0.join("ground.tif"));
        }
        if save { command.arg("--save-samples"); }
        let output=command.output().unwrap();
        assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("\rCompleted sample 1/3\rCompleted sample 2/3\rCompleted sample 3/3\n"), "{stderr}");
        assert!(output.stdout.is_empty());
    };
    run("full","saved",true, true, 1);
    for name in ["total_resistance","current"] {
        let maps: Vec<_>=(0..3).map(|i|read_raster(&dir.0.join(format!("saved/samples/{i:06}/{name}.tif"))).unwrap().data).collect();
        let mean=read_raster(&dir.0.join(format!("saved/{name}.tif"))).unwrap();
        let variance=read_raster(&dir.0.join(format!("saved/{name}_variance.tif"))).unwrap();
        for i in 0..64 {
            let expected=maps.iter().map(|m|m[i]).sum::<f64>()/3.0;
            let expected_variance=maps.iter().map(|m|(m[i]-expected).powi(2)).sum::<f64>()/2.0;
            assert!((mean.data[i]-expected).abs()<1e-10);
            assert!((variance.data[i]-expected_variance).abs()<1e-10);
        }
    }
    let fixed = read_raster(&dir.0.join("saved/fixed.tif")).unwrap();
    assert_eq!(fixed.data, [1.0, 1.0, 2.0, 2.0].repeat(16));
    assert!(!dir.0.join("saved/fixed_variance.tif").exists());
    run("full","statistics-only",false, true, 1);
    assert!(!dir.0.join("statistics-only/samples").exists());
    for name in ["total_resistance","total_resistance_variance","current","current_variance"] {
        assert_eq!(read_raster(&dir.0.join(format!("saved/{name}.tif"))).unwrap().data,
            read_raster(&dir.0.join(format!("statistics-only/{name}.tif"))).unwrap().data);
    }
    run("resistance", "resistance-only", true, true, 1);
    assert!(!dir.0.join("resistance-only/current.tif").exists());
    for name in ["total_resistance", "total_resistance_variance"] {
        assert_eq!(read_raster(&dir.0.join(format!("saved/{name}.tif"))).unwrap().data,
            read_raster(&dir.0.join(format!("resistance-only/{name}.tif"))).unwrap().data);
    }
    run("full", "save-implies-all-layers", true, false, 3);
    assert!(dir.0.join("save-implies-all-layers/fixed.tif").exists());
    assert!(!dir.0.join("save-implies-all-layers/fixed_variance.tif").exists());
    for index in 0..3 {
        assert!(!dir.0.join(format!("save-implies-all-layers/samples/{index:06}/fixed.tif")).exists());
    }
    run("full", "default-summaries", false, false, 3);
    assert!(!dir.0.join("default-summaries/samples").exists());
    assert!(!dir.0.join("default-summaries/fixed.tif").exists());
    for name in ["total_resistance", "total_resistance_variance", "current", "current_variance"] {
        let actual = read_raster(&dir.0.join(format!("default-summaries/{name}.tif"))).unwrap().data;
        let expected = read_raster(&dir.0.join(format!("saved/{name}.tif"))).unwrap().data;
        for (actual, expected) in actual.iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-10);
        }
    }
    for index in 0..3 {
        let relative = format!("samples/{index:06}/total_resistance.tif");
        assert_eq!(read_raster(&dir.0.join("saved").join(&relative)).unwrap().data,
            read_raster(&dir.0.join("resistance-only").join(relative)).unwrap().data);
        assert!(!dir.0.join(format!("resistance-only/samples/{index:06}/current.tif")).exists());
    }

}
