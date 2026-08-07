use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use tiff::decoder::{Decoder, DecodingResult};
use tiff::encoder::{colortype, TiffEncoder};
use wasm_connect::geospatial::{rasterize_features, GeoTransform, LayerParams};
use wasm_connect::resistance::pipeline::{run_resistance_pipeline, ResistanceParams};
use wasm_connect::resistance::surface::calc_surfs;
use wasm_connect::resistance::landscape::{compute_base_conductance, get_landscape_resistance_lcm};

fn read_tiff_f32(path: &Path) -> io::Result<(Vec<f64>, usize, usize)> {
    let file = fs::File::open(path)?;
    let mut decoder = Decoder::new(BufReader::new(file))
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    let (width, height) = decoder
        .dimensions()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    let ncols = width as usize;
    let nrows = height as usize;

    match decoder
        .read_image()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?
    {
        DecodingResult::F32(data) => {
            let data_f64: Vec<f64> = data.into_iter().map(|v| v as f64).collect();
            Ok((data_f64, nrows, ncols))
        }
        DecodingResult::F64(data) => Ok((data, nrows, ncols)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected float32 or float64 TIFF",
        )),
    }
}

fn write_tiff_f32(path: &Path, data: &[f64], ncols: usize, nrows: usize) -> io::Result<()> {
    let file = fs::File::create(path)?;
    let mut tiff = TiffEncoder::new(BufWriter::new(file))
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    let data_f32: Vec<f32> = data.iter().map(|&v| v as f32).collect();

    tiff.write_image::<colortype::Gray32Float>(ncols as u32, nrows as u32, &data_f32)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    Ok(())
}

fn write_asc(
    path: &Path,
    data: &[f64],
    ncols: usize,
    nrows: usize,
    xmin: f64,
    ymin: f64,
    cellsize: f64,
    nodata: f64,
) -> io::Result<()> {
    let file = fs::File::create(path)?;
    let mut w = BufWriter::new(file);

    writeln!(w, "ncols         {}", ncols)?;
    writeln!(w, "nrows         {}", nrows)?;
    writeln!(w, "xllcorner     {}", xmin)?;
    writeln!(w, "yllcorner     {}", ymin)?;
    writeln!(w, "cellsize      {}", cellsize)?;
    writeln!(w, "NODATA_value  {}", nodata)?;

    for r in 0..nrows {
        let row_start = r * ncols;
        for c in 0..ncols {
            let v = data[row_start + c];
            if c > 0 {
                write!(w, " ")?;
            }
            if v.is_finite() {
                write!(w, "{}", v)?;
            } else {
                write!(w, "{}", nodata)?;
            }
        }
        writeln!(w)?;
    }

    Ok(())
}

fn read_json(path: &Path) -> io::Result<serde_json::Value> {
    let content = fs::read_to_string(path)?;
    serde_json::from_str(&content)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
}

fn read_optional_tiff(work_dir: &Path, name: &str) -> io::Result<Option<(Vec<f64>, usize, usize)>> {
    let path = work_dir.join(name);
    if path.exists() {
        let (data, nrows, ncols) = read_tiff_f32(&path)?;
        Ok(Some((data, nrows, ncols)))
    } else {
        Ok(None)
    }
}

fn read_optional_geojson(work_dir: &Path, name: &str) -> io::Result<Option<String>> {
    let path = work_dir.join(name);
    if path.exists() {
        let content = fs::read_to_string(path)?;
        Ok(Some(content))
    } else {
        Ok(None)
    }
}

fn emit_json_log(level: &str, msg: &str) {
    let log = serde_json::json!({
        "level": level,
        "msg": msg,
        "user_visible": true
    });
    println!("{}", serde_json::to_string(&log).unwrap_or_default());
}

fn rasterize_geojson_to_binary(
    geojson_str: &str,
    layer_name: &str,
    nrows: usize,
    ncols: usize,
    transform: &GeoTransform,
) -> Vec<f64> {
    let base = vec![0.0f64; nrows * ncols];
    let mut warnings = Vec::new();
    let mut params_map = HashMap::new();
    params_map.insert(
        layer_name.to_string(),
        LayerParams {
            resistance: 1.0,
            width: 0.0,
        },
    );

    let (result, _masks) = rasterize_features(geojson_str, &params_map, &base, nrows, ncols, transform, &mut warnings);
    let non_zero = result.iter().filter(|&&v| v > 0.0 && v.is_finite()).count();
    emit_json_log("INFO", &format!(
        "rasterized {} ({} bytes) → {} non-zero cells",
        layer_name, geojson_str.len(), non_zero
    ));
    for w in &warnings {
        emit_json_log("WARN", &format!("rasterize {}: {}", layer_name, w));
    }
    result
}

fn create_circles_raster(
    nrows: usize,
    ncols: usize,
    roost_row: usize,
    roost_col: usize,
    radius_meters: f64,
    pixw: f64,
    n_circles: usize,
) -> Vec<f64> {
    let total = nrows * ncols;
    let mut circles = vec![0.0f64; total];
    let radius_cells = radius_meters / pixw;
    let lb = (radius_cells / n_circles.max(1) as f64).max(1.0);

    let mut r = lb;
    while r <= radius_cells {
        let n_pts = (3.0 * r).max(10.0) as usize;
        for i in 0..n_pts {
            let angle = 2.0 * std::f64::consts::PI * i as f64 / n_pts as f64;
            let col = roost_col as f64 + r * angle.sin();
            let row = roost_row as f64 + r * angle.cos();
            if col >= 0.0 && col < ncols as f64 && row >= 0.0 && row < nrows as f64 {
                let idx = row as usize * ncols + col as usize;
                circles[idx] = 1.0;
            }
        }
        r += lb;
    }

    circles
}

fn run_landscape(work_dir: &Path) -> io::Result<()> {
    let input = read_json(&work_dir.join("inputs.json"))?;
    let empty_map = serde_json::Map::new();
    let params_json = input
        .get("params")
        .and_then(serde_json::Value::as_object)
        .unwrap_or(&empty_map);

    let roost = input.get("roost").and_then(|r| r.as_object());
    let resolution = params_json.get("resolution").and_then(|v| v.as_f64()).unwrap_or(10.0);
    let radius = roost.and_then(|r| r.get("radius")).and_then(|v| v.as_f64()).unwrap_or(2500.0);
    let n_circles = params_json.get("n_circles").and_then(|v| v.as_f64()).map(|v| v as usize).unwrap_or(5);

    let grid_info_path = work_dir.join("grid_info.json");
    let (xmin, ymax, pixw, nrows, ncols) = if grid_info_path.exists() {
        let gi = read_json(&grid_info_path)?;
        (
            gi.get("xmin").and_then(|v| v.as_f64()).unwrap_or(0.0),
            gi.get("ymax").and_then(|v| v.as_f64()).unwrap_or(0.0),
            gi.get("pixw").and_then(|v| v.as_f64()).unwrap_or(resolution),
            gi.get("nrows").and_then(|v| v.as_u64()).map(|v| v as usize).unwrap_or(0),
            gi.get("ncols").and_then(|v| v.as_u64()).map(|v| v as usize).unwrap_or(0),
        )
    } else {
        let e = roost.and_then(|r| r.get("easting")).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let n = roost.and_then(|r| r.get("northing")).and_then(|v| v.as_f64()).unwrap_or(0.0);
        (e - radius, n + radius, resolution, (2.0 * radius / resolution) as usize, (2.0 * radius / resolution) as usize)
    };

    emit_json_log("INFO", "Landscape resistance stage starting");
    emit_json_log("INFO", &format!("Grid: {}x{}, pixw={}, xmin={}", ncols, nrows, pixw, xmin));

    let (lcm, _, _) = read_tiff_f32(&work_dir.join("lcm.tif"))?;
    let (dtm, nrows_dtm, ncols_dtm) = read_tiff_f32(&work_dir.join("dtm.tif"))?;
    let (dsm, nrows_dsm, _ncols_dsm) = read_tiff_f32(&work_dir.join("dsm.tif"))?;
    if nrows_dtm != nrows_dsm {
        emit_json_log("ERROR", "DTM and DSM dimensions must match");
        std::process::exit(1);
    }
    let (nrows, ncols) = (nrows_dtm, ncols_dtm);

    let transform = GeoTransform { xmin, ymax, cellsize: pixw };
    let buildings_path = work_dir.join("buildings.tif");
    let buildings_geojson_path = work_dir.join("buildings.geojson");
    let building_mask: Vec<f64> = if buildings_path.exists() {
        read_tiff_f32(&buildings_path)?.0
    } else if buildings_geojson_path.exists() {
        emit_json_log("INFO", "Rasterizing buildings.geojson...");
        let gj = fs::read_to_string(&buildings_geojson_path)?;
        let gj_len = gj.len();
        let base = vec![0.0f64; nrows * ncols];
        let mut warnings = Vec::new();
        let mut params_map = HashMap::new();
        params_map.insert("buildings".to_string(), LayerParams { resistance: 1.0, width: 0.0 });
        let (result, _masks) = rasterize_features(&gj, &params_map, &base, nrows, ncols, &transform, &mut warnings);
        let nz = result.iter().filter(|&&v| v > 0.0 && v.is_finite()).count();
        emit_json_log("INFO", &format!("rasterized buildings ({}) → {} non-zero cells", gj_len, nz));
        for w in &warnings {
            emit_json_log("WARN", &format!("rasterize buildings: {}", w));
        }
        result
    } else {
        vec![0.0; nrows * ncols]
    };

    let surfs = calc_surfs(&dtm, &dsm, &building_mask, nrows, ncols);

    let rankmax = params_json.get("landscape_rankmax").and_then(|v| v.as_f64()).unwrap_or(8.0);
    let resmax = params_json.get("landscape_resmax").and_then(|v| v.as_f64()).unwrap_or(100.0);
    let xmax = params_json.get("landscape_xmax").and_then(|v| v.as_f64()).unwrap_or(5.0);

    let base_soft_surf: Vec<f64> = dtm.iter().zip(dsm.iter())
        .map(|(&t, &s)| if t.is_finite() && s.is_finite() { s - t } else { 0.0 })
        .collect();
    let landscape_conductance = compute_base_conductance(&base_soft_surf, &lcm);
    emit_json_log("INFO", "Writing landscape_conductance.tif");
    write_tiff_f32(&work_dir.join("landscape_conductance.tif"), &landscape_conductance, ncols, nrows)?;

    let landscape_res = get_landscape_resistance_lcm(
        &lcm, &building_mask, &surfs.soft_surf, nrows, ncols, rankmax, resmax, xmax,
    );

    emit_json_log("INFO", "Writing landscape_res.tif");
    write_tiff_f32(&work_dir.join("landscape_res.tif"), &landscape_res, ncols, nrows)?;

    write_grid_info(work_dir, xmin, ymax, pixw, nrows, ncols);

    write_asc_files(work_dir, &landscape_res, ncols, nrows, xmin, ymax, pixw, roost, radius, n_circles)?;

    emit_json_log("INFO", "Landscape resistance stage complete");
    Ok(())
}

fn write_grid_info(work_dir: &Path, xmin: f64, ymax: f64, pixw: f64, nrows: usize, ncols: usize) {
    let gi = serde_json::json!({
        "xmin": xmin,
        "ymax": ymax,
        "pixw": pixw,
        "nrows": nrows,
        "ncols": ncols,
    });
    let path = work_dir.join("grid_info.json");
    if let Ok(json_str) = serde_json::to_string(&gi) {
        fs::write(path, json_str).ok();
    }
}

fn write_asc_files(
    work_dir: &Path,
    total_res: &[f64],
    ncols: usize, nrows: usize,
    xmin: f64, ymax: f64, pixw: f64,
    roost: Option<&serde_json::Map<String, serde_json::Value>>,
    radius: f64, n_circles: usize,
) -> io::Result<()> {
    let cs_dir = work_dir.join("circuitscape");
    fs::create_dir_all(&cs_dir).map_err(|e| io::Error::new(io::ErrorKind::Other, format!("create_dir: {}", e)))?;

    let ymin = ymax - nrows as f64 * pixw;

    write_asc(&cs_dir.join("resistance.asc"), total_res, ncols, nrows, xmin, ymin, pixw, -9999.0)?;

    let roost_easting = roost.and_then(|r| r.get("easting")).and_then(|v| v.as_f64());
    let roost_northing = roost.and_then(|r| r.get("northing")).and_then(|v| v.as_f64());
    let roost_col = if let Some(e) = roost_easting { ((e - xmin) / pixw) as usize } else { ncols / 2 };
    let roost_row = if let Some(n) = roost_northing { ((ymax - n) / pixw) as usize } else { nrows / 2 };

    let circles = create_circles_raster(nrows, ncols, roost_row, roost_col, radius, pixw, n_circles);
    write_asc(&cs_dir.join("source.asc"), &circles, ncols, nrows, xmin, ymin, pixw, -9999.0)?;

    let mut ground = vec![0.0f64; nrows * ncols];
    if roost_row < nrows && roost_col < ncols {
        ground[roost_row * ncols + roost_col] = 1.0;
    }
    write_asc(&cs_dir.join("ground.asc"), &ground, ncols, nrows, xmin, ymin, pixw, -9999.0)?;

    Ok(())
}

fn run() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: resistance-pipeline <work_dir> [--stage landscape|full]");
        std::process::exit(1);
    }

    let work_dir = PathBuf::from(&args[1]);
    let stage = if args.len() >= 3 && args[2] == "--stage" {
        args.get(3).map(|s| s.as_str()).unwrap_or("full")
    } else {
        "full"
    };

    if stage == "landscape" {
        return run_landscape(&work_dir);
    }

    // --- full pipeline below (existing code) ---
    let input_path = work_dir.join("inputs.json");

    emit_json_log("INFO", "Resistance pipeline starting");

    let input = read_json(&input_path)?;
    let empty_map = serde_json::Map::new();
    let params_json = input
        .get("params")
        .and_then(serde_json::Value::as_object)
        .unwrap_or(&empty_map);

    let roost = input.get("roost").and_then(|r| r.as_object());
    let resolution = params_json
        .get("resolution")
        .and_then(|v| v.as_f64())
        .unwrap_or(10.0);
    let radius = roost
        .and_then(|r| r.get("radius"))
        .and_then(|v| v.as_f64())
        .unwrap_or(2500.0);
    let n_circles = params_json
        .get("n_circles")
        .and_then(|v| v.as_f64())
        .map(|v| v as usize)
        .unwrap_or(5);

    let grid_info_path = work_dir.join("grid_info.json");
    let (xmin, ymax, pixw, nrows, ncols) = if grid_info_path.exists() {
        let gi = read_json(&grid_info_path)?;
        (
            gi.get("xmin").and_then(|v| v.as_f64()).unwrap_or(0.0),
            gi.get("ymax").and_then(|v| v.as_f64()).unwrap_or(0.0),
            gi.get("pixw").and_then(|v| v.as_f64()).unwrap_or(resolution),
            gi.get("nrows").and_then(|v| v.as_u64()).map(|v| v as usize).unwrap_or(0),
            gi.get("ncols").and_then(|v| v.as_u64()).map(|v| v as usize).unwrap_or(0),
        )
    } else {
        let e = roost
            .and_then(|r| r.get("easting"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let n = roost
            .and_then(|r| r.get("northing"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let p = resolution;
        (
            e - radius,
            n + radius,
            p,
            (2.0 * radius / p) as usize,
            (2.0 * radius / p) as usize,
        )
    };

    emit_json_log("INFO", &format!(
        "Reading raster inputs ({}x{}, pixw={}, xmin={})",
        ncols, nrows, pixw, xmin
    ));

    let required = ["dtm", "dsm", "lcm"];
    for name in &required {
        let path = work_dir.join(format!("{}.tif", name));
        if !path.exists() {
            emit_json_log("ERROR", &format!("Missing required raster: {}.tif", name));
            std::process::exit(1);
        }
    }

    let (dtm, nrows_dtm, ncols_dtm) = read_tiff_f32(&work_dir.join("dtm.tif"))?;
    let (dsm, nrows_dsm, _ncols_dsm) = read_tiff_f32(&work_dir.join("dsm.tif"))?;
    let (lcm, _, _) = read_tiff_f32(&work_dir.join("lcm.tif"))?;

    if nrows_dtm != nrows_dsm {
        emit_json_log("ERROR", "DTM and DSM dimensions must match");
        std::process::exit(1);
    }
    let (nrows, ncols) = (nrows_dtm, ncols_dtm);

    let transform = GeoTransform {
        xmin,
        ymax,
        cellsize: pixw,
    };

    let road_binary = match read_optional_geojson(&work_dir, "roads.geojson")? {
        Some(gj) => {
            emit_json_log("INFO", &format!("Found roads.geojson ({} bytes), rasterizing...", gj.len()));
            rasterize_geojson_to_binary(&gj, "roads", nrows, ncols, &transform)
        }
        None => read_optional_tiff(&work_dir, "road_binary.tif")?
            .map(|(d, _, _)| d)
            .unwrap_or_else(|| {
                emit_json_log("WARN", "No roads.geojson or road_binary.tif found — road resistance will be zero");
                vec![0.0; nrows * ncols]
            }),
    };

    let river_binary = match read_optional_geojson(&work_dir, "rivers.geojson")? {
        Some(gj) => {
            emit_json_log("INFO", &format!("Found rivers.geojson ({} bytes), rasterizing...", gj.len()));
            rasterize_geojson_to_binary(&gj, "rivers", nrows, ncols, &transform)
        }
        None => read_optional_tiff(&work_dir, "river_binary.tif")?
            .map(|(d, _, _)| d)
            .unwrap_or_else(|| {
                emit_json_log("WARN", "No rivers.geojson or river_binary.tif found — river resistance will be max");
                vec![0.0; nrows * ncols]
            }),
    };

    let building_mask = match read_optional_geojson(&work_dir, "buildings.geojson")? {
        Some(gj) => {
            emit_json_log("INFO", &format!("Found buildings.geojson ({} bytes), rasterizing...", gj.len()));
            rasterize_geojson_to_binary(&gj, "buildings", nrows, ncols, &transform)
        }
        None => read_optional_tiff(&work_dir, "buildings.tif")?
            .map(|(d, _, _)| d)
            .unwrap_or_else(|| vec![0.0; nrows * ncols]),
    };

    let generic_res = match read_optional_geojson(&work_dir, "generic_resistance.geojson")? {
        Some(gj) => {
            let base = vec![0.0f64; nrows * ncols];
            let mut warnings = Vec::new();
            let mut params_map = HashMap::new();
            params_map.insert(
                "generic_resistance".to_string(),
                LayerParams {
                    resistance: 100.0,
                    width: 0.0,
                },
            );
            let (result, _) = rasterize_features(
                &gj,
                &params_map,
                &base,
                nrows,
                ncols,
                &transform,
                &mut warnings,
            );
            let nz = result.iter().filter(|&&v| v > 0.0 && v.is_finite()).count();
            emit_json_log("INFO", &format!(
                "rasterized generic_resistance ({} bytes) → {} non-zero cells",
                gj.len(), nz
            ));
            for w in &warnings {
                emit_json_log("WARN", &format!("rasterize generic_resistance: {}", w));
            }
            result
        }
        None => read_optional_tiff(&work_dir, "generic_resistance.tif")?
            .map(|(d, _, _)| d)
            .unwrap_or_else(|| vec![0.0; nrows * ncols]),
    };

    let lamps: Vec<f64> = input
        .get("lamps")
        .and_then(|l| l.as_array())
        .map(|arr| {
            arr.iter()
                .flat_map(|v| {
                    if let Some(arr2) = v.as_array() {
                        arr2.iter()
                            .filter_map(|n| n.as_f64())
                            .collect::<Vec<_>>()
                    } else if let Some(n) = v.as_f64() {
                        vec![n]
                    } else {
                        vec![]
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let n_pixels = nrows * ncols;
    emit_json_log("INFO", &format!(
        "Processing {}x{} grid ({} pixels), {} lamp(s)",
        nrows,
        ncols,
        n_pixels,
        (lamps.len() / 3).max(1)
    ));

    let params = ResistanceParams {
        road_buffer: params_json.get("road_buffer").and_then(|v| v.as_f64()).unwrap_or(200.0),
        road_resmax: params_json.get("road_resmax").and_then(|v| v.as_f64()).unwrap_or(10.0),
        road_xmax: params_json.get("road_xmax").and_then(|v| v.as_f64()).unwrap_or(5.0),
        river_buffer: params_json.get("river_buffer").and_then(|v| v.as_f64()).unwrap_or(10.0),
        river_resmax: params_json.get("river_resmax").and_then(|v| v.as_f64()).unwrap_or(2000.0),
        river_xmax: params_json.get("river_xmax").and_then(|v| v.as_f64()).unwrap_or(4.0),
        landscape_rankmax: params_json.get("landscape_rankmax").and_then(|v| v.as_f64()).unwrap_or(8.0),
        landscape_resmax: params_json.get("landscape_resmax").and_then(|v| v.as_f64()).unwrap_or(100.0),
        landscape_xmax: params_json.get("landscape_xmax").and_then(|v| v.as_f64()).unwrap_or(5.0),
        linear_buffer: params_json.get("linear_buffer").and_then(|v| v.as_f64()).unwrap_or(10.0),
        linear_rankmax: params_json.get("linear_rankmax").and_then(|v| v.as_f64()).unwrap_or(4.0),
        linear_resmax: params_json.get("linear_resmax").and_then(|v| v.as_f64()).unwrap_or(22000.0),
        linear_xmax: params_json.get("linear_xmax").and_then(|v| v.as_f64()).unwrap_or(3.0),
        lamp_resmax: params_json.get("lamp_resmax").and_then(|v| v.as_f64()).unwrap_or(1e8),
        lamp_xmax: params_json.get("lamp_xmax").and_then(|v| v.as_f64()).unwrap_or(1.0),
        lamp_ext: params_json.get("lamp_ext").and_then(|v| v.as_f64()).unwrap_or(100.0),
        pixw,
        nrows,
        ncols,
    };

    emit_json_log("INFO", "Computing resistance rasters");
    let output = run_resistance_pipeline(
        &road_binary,
        &river_binary,
        &building_mask,
        &lcm,
        &dtm,
        &dsm,
        &generic_res,
        &lamps,
        &params,
        None,
    );

    let count_non_zero = |label: &str, data: &[f64]| {
        let nz = data.iter().filter(|&&v| v > 0.0 && v.is_finite()).count();
        emit_json_log("INFO", &format!("{}: {} non-zero cells", label, nz));
    };
    count_non_zero("road_res", &output.road_res);
    count_non_zero("river_res", &output.river_res);
    count_non_zero("landscape_res", &output.landscape_res);
    count_non_zero("linear_res", &output.linear_res);
    count_non_zero("lamp_res", &output.lamp_res);
    count_non_zero("total_res", &output.total_res);

    emit_json_log("INFO", "Writing output GeoTIFFs and ASC files");

    let mut write_errors: Vec<String> = Vec::new();

    let mut write_layer = |name: &str, data: &[f64]| {
        let path = work_dir.join(format!("{}.tif", name));
        if let Err(e) = write_tiff_f32(&path, data, ncols, nrows) {
            write_errors.push(format!("{}: {}", name, e));
        }
    };

    write_layer("road_res", &output.road_res);
    write_layer("river_res", &output.river_res);
    write_layer("landscape_res", &output.landscape_res);
    write_layer("linear_res", &output.linear_res);
    write_layer("lamp_res", &output.lamp_res);
    write_layer("generic_res", &output.generic_res);
    write_layer("total_res", &output.total_res);
    write_layer("soft_surf", &output.soft_surf);
    write_layer("hard_surf", &output.hard_surf);
    write_layer("manhedge", &output.manhedge);
    write_layer("unmanhedge", &output.unmanhedge);
    write_layer("tree", &output.tree);

    let log_total_res: Vec<f64> = output
        .total_res
        .iter()
        .map(|&v| if v.is_finite() && v > 0.0 { v.ln() } else { f64::NAN })
        .collect();
    write_layer("log_total_res", &log_total_res);

    if output.lamp_res.iter().any(|&v| v > 0.0) {
        let log_lamp_res: Vec<f64> = output
            .lamp_res
            .iter()
            .map(|&v| if v.is_finite() && v > 0.0 { v.ln() } else { f64::NAN })
            .collect();
        write_layer("log_lamp_res", &log_lamp_res);
    }

    if !write_errors.is_empty() {
        emit_json_log("ERROR", &format!("Failed to write layers: {}", write_errors.join(", ")));
        std::process::exit(1);
    }

    write_asc_files(&work_dir, &output.total_res, ncols, nrows, xmin, ymax, pixw, roost, radius, n_circles)?;

    emit_json_log("INFO", "Resistance pipeline complete");

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        emit_json_log("ERROR", &format!("{}", e));
        std::process::exit(1);
    }
}
