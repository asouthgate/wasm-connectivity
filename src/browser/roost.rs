use super::output::{encode_f64_as_f32_base64, json_response};
use crate::roost;
use serde::Serialize;
use serde_json::json;
use wasm_bindgen::prelude::*;

#[derive(Serialize)]
struct RoostDetectorPoint {
    x: f64,
    y: f64,
    count: f64,
}

#[derive(Serialize)]
struct RoostFinderOutput {
    /// Predicted roost (BNG easting/northing) + loss.
    x: f64,
    y: f64,
    loss: f64,
    /// Loss surface as base64-encoded little-endian `f32`, row-major from south to north.
    surface_base64: String,
    /// Number of grid points per axis.
    grid_size: usize,
    /// Grid extents in BNG: `[xmin, ymin, xmax, ymax]`.
    bounds_bng: [f64; 4],
    /// Weighted-mean centroid of detectors by count (BNG).
    weighted_mean_x: f64,
    weighted_mean_y: f64,
    /// Per-detector points that contributed (BNG), with their (per-night or
    /// raw) counts.
    detectors: Vec<RoostDetectorPoint>,
    /// Non-fatal warnings (e.g. skipped detectors).
    warnings: Vec<String>,
}

/// Parse roost-finder CSVs, average calls per detector per active night, and
/// compute the error surface + predicted roost in one WASM call.
///
/// When `sunset_csv` is supplied (non-empty), master rows are filtered to
/// `[sunset, sunset + t1/60]`, so the observation window is tied to the
/// kernel's integration upper bound `t1`. `t0`/`t1` are the integration bounds
/// in seconds (defaults 0.01 / 5400).
///
/// Method: Henley et al. (2024), "A simple and fast method for estimating bat
/// roost locations", *Royal Society Open Science* 11(4): 231999.
/// <https://doi.org/10.1098/rsos.231999>
#[wasm_bindgen]
pub fn roost_finder_compute(
    detectors_csv: String,
    master_csv: String,
    sunset_csv: String,
    grid_size: usize,
    diffusivity: f64,
    t0: f64,
    t1: f64,
) -> String {
    run_roost_finder(
        &detectors_csv,
        &master_csv,
        &sunset_csv,
        grid_size,
        diffusivity,
        t0,
        t1,
    )
    .map(|out| json_response(&out))
    .unwrap_or_else(|e| {
        serde_json::to_string(&json!({ "error": e }))
            .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())
    })
}

fn run_roost_finder(
    detectors_csv: &str,
    master_csv: &str,
    sunset_csv: &str,
    grid_size: usize,
    diffusivity: f64,
    t0: f64,
    t1: f64,
) -> Result<RoostFinderOutput, String> {
    use roost::io::{aggregate_with_warnings, count_calls, read_detectors, read_sunset};

    if grid_size < 2 {
        return Err("grid_size must be >= 2".to_string());
    }
    if !(t1 > t0 && t0 > 0.0) {
        return Err("require 0 < t0 < t1".to_string());
    }

    // The observation window is tied to t1: calls are counted from sunset to
    // sunset + t1/60 minutes, matching the kernel's integration upper bound.
    let minutes_after_sunset = t1 / 60.0;

    let detectors = read_detectors(detectors_csv)?;

    let sunset = if sunset_csv.trim().is_empty() {
        None
    } else {
        Some(read_sunset(sunset_csv)?)
    };

    let counts = count_calls(master_csv, sunset.as_ref().map(|s| (s, minutes_after_sunset)))?;
    let (agg, warnings) = aggregate_with_warnings(&detectors, &counts);

    if agg.x.is_empty() {
        return Err("no detectors with calls found after aggregation".to_string());
    }

    let mut surface = Vec::with_capacity(grid_size * grid_size);
    let result = roost::compute_error_surface(
        &agg.x, &agg.y, &agg.counts, grid_size, diffusivity, t0, t1,
        |_cx, _cy, l| surface.push(l),
    );

    let total: f64 = agg.counts.iter().sum();
    let weighted_mean_x = agg
        .x
        .iter()
        .zip(agg.counts.iter())
        .map(|(x, c)| x * c)
        .sum::<f64>()
        / total;
    let weighted_mean_y = agg
        .y
        .iter()
        .zip(agg.counts.iter())
        .map(|(y, c)| y * c)
        .sum::<f64>()
        / total;

    let detector_points = agg
        .x
        .iter()
        .zip(agg.y.iter())
        .zip(agg.counts.iter())
        .map(|((&x, &y), &count)| RoostDetectorPoint { x, y, count })
        .collect();

    // Grid extents match the search grid built by `compute_error_surface`.
    let xmin = agg.x.iter().cloned().fold(f64::INFINITY, f64::min);
    let xmax = agg.x.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let ymin = agg.y.iter().cloned().fold(f64::INFINITY, f64::min);
    let ymax = agg.y.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    let surface_base64 = encode_f64_as_f32_base64(&surface);

    Ok(RoostFinderOutput {
        x: result.x,
        y: result.y,
        loss: result.loss,
        surface_base64,
        grid_size,
        bounds_bng: [xmin, ymin, xmax, ymax],
        weighted_mean_x,
        weighted_mean_y,
        detectors: detector_points,
        warnings,
    })
}
