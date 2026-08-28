use geojson::GeoJson;
use geo::{
    BoundingRect, Contains, EuclideanDistance, Geometry, LineString,
    Point, Polygon,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize, Clone)]
pub struct LayerParams {
    pub resistance: f64,
    #[serde(default)]
    pub width: f64,
}

// Geospatial transformation struct to handle coordinate
//  conversions between geographic and pixel space.
pub struct GeoTransform {
    pub xmin: f64,
    pub ymax: f64,
    pub cellsize: f64,
}

impl GeoTransform {
    fn is_valid(&self) -> bool {
        self.cellsize.is_finite() && self.cellsize > 0.0
    }

    // Converts geographic coordinates (x, y) to pixel coordinates (col, row).
    fn geo_to_pixel(&self, x: f64, y: f64) -> (isize, isize) {
        if !self.is_valid() {
            return (0, 0);
        }
        let col = ((x - self.xmin) / self.cellsize) as isize;
        let row = ((self.ymax - y) / self.cellsize) as isize;
        (col, row)
    }

    // Converts pixel coordinates (col, row) back to geographic coordinates (x, y).
    fn pixel_to_geo(&self, col: usize, row: usize) -> Point<f64> {
        Point::new(
            self.xmin + (col as f64 + 0.5) * self.cellsize,
            self.ymax - (row as f64 + 0.5) * self.cellsize,
        )
    }

    // Clips the given geographic bounds to ensure they fit within the raster dimensions.
    fn clip_bounds(&self, col_min: isize, row_min: isize, col_max: isize, row_max: isize, nrows: usize, ncols: usize) -> (usize, usize, usize, usize) {
        let col_start = col_min.max(0) as usize;
        let col_end = (col_max.saturating_add(1)).min(ncols as isize).max(0) as usize;
        let row_start = row_min.max(0) as usize;
        let row_end = (row_max.saturating_add(1)).min(nrows as isize).max(0) as usize;
        (col_start, col_end, row_start, row_end)
    }
}

// Calculates the shortest distance from a point to a line string by checking each segment of the line string.
fn point_to_line_distance(pt: &Point<f64>, ls: &LineString<f64>) -> f64 {
    let mut min_dist = f64::MAX;
    for segment in ls.lines() {
        let dist = pt.euclidean_distance(&segment);
        if dist < min_dist {
            min_dist = dist;
        }
    }
    min_dist
}

fn rasterize_lines_bresenham(
    raster: &mut [f64],
    mask: &mut [f64],
    nrows: usize,
    ncols: usize,
    lines: &[&LineString<f64>],
    value: f64,
    transform: &GeoTransform,
) {
    for ls in lines {
        for segment in ls.lines() {
            let (col_s, row_s) = transform.geo_to_pixel(segment.start.x, segment.start.y);
            let (col_e, row_e) = transform.geo_to_pixel(segment.end.x, segment.end.y);

            let mut c0 = col_s;
            let mut r0 = row_s;
            let c1 = col_e;
            let r1 = row_e;

            let dc = (c1 - c0).abs();
            let dr = -(r1 - r0).abs();
            let sc: isize = if c0 < c1 { 1 } else { -1 };
            let sr: isize = if r0 < r1 { 1 } else { -1 };
            let mut err = dc + dr;

            loop {
                if c0 >= 0 && c0 < ncols as isize && r0 >= 0 && r0 < nrows as isize {
                    let idx = r0 as usize * ncols + c0 as usize;
                    if value > raster[idx] {
                        raster[idx] = value;
                    }
                    mask[idx] = 1.0;
                }
                if c0 == c1 && r0 == r1 {
                    break;
                }
                let e2 = 2 * err;
                if e2 >= dr {
                    if c0 == c1 {
                        break;
                    }
                    err += dr;
                    c0 += sc;
                }
                if e2 <= dc {
                    if r0 == r1 {
                        break;
                    }
                    err += dc;
                    r0 += sr;
                }
            }
        }
    }
}

/// Rasterizes a polygon into a raster grid, updating the raster values and mask.
//
// # Arguments
// * `raster` - A mutable slice representing the raster grid values.
/// * `mask` - A mutable slice representing the layer mask values.
/// * `nrows` - Number of rows in the raster grid.
/// * `ncols` - Number of columns in the raster grid.
/// * `poly` - The polygon to rasterize.
/// * `value` - The resistance value to assign to the raster cells covered by the polygon.
/// * `transform` - The geospatial transformation for coordinate conversions.
fn rasterize_polygon(
    raster: &mut [f64],
    mask: &mut [f64],
    nrows: usize,
    ncols: usize,
    poly: &Polygon<f64>,
    value: f64,
    transform: &GeoTransform,
) {
    let bbox = match poly.bounding_rect() {
        Some(b) => b,
        None => return,
    };
    
    let x_min = bbox.min().x;
    let x_max = bbox.max().x;
    let y_min = bbox.min().y;
    let y_max = bbox.max().y;

    // y_max is the highest latitude, matching the smallest row index (top of the image).
    // y_min is the lowest latitude, matching the largest row index (bottom of the image).
    let (col_min, row_min) = transform.geo_to_pixel(x_min, y_max);
    let (col_max, row_max) = transform.geo_to_pixel(x_max, y_min);

    let (col_start, col_end, row_start, row_end) = 
        transform.clip_bounds(col_min, row_min, col_max, row_max, nrows, ncols);

    for row in row_start..row_end {
        let row_offset = row * ncols;
        for col in col_start..col_end {
            let pt = transform.pixel_to_geo(col, row);
            if poly.contains(&pt) {
                let idx = row_offset + col;
                
                if value > raster[idx] {
                    raster[idx] = value;
                }
                // Write to layer mask (always 1.0 if inside)
                mask[idx] = 1.0;
            }
        }
    }
}

/// Rasterizes a set of lines into a raster grid, updating the raster values and mask.
fn rasterize_lines(
    raster: &mut [f64],
    mask: &mut [f64],
    nrows: usize,
    ncols: usize,
    lines: &[&LineString<f64>],
    value: f64,
    width: f64,
    transform: &GeoTransform,
) {
    if lines.is_empty() {
        return;
    }
    if width <= 0.0 {
        rasterize_lines_bresenham(raster, mask, nrows, ncols, lines, value, transform);
        return;
    }

    for ls in lines {
        let bbox = match ls.bounding_rect() {
            Some(b) => b,
            None => continue,
        };

        let x_min = bbox.min().x - width;
        let x_max = bbox.max().x + width;
        let y_min = bbox.min().y - width;
        let y_max = bbox.max().y + width;

        let (col_min, row_min) = transform.geo_to_pixel(x_min, y_max);
        let (col_max, row_max) = transform.geo_to_pixel(x_max, y_min);

        let (col_start, col_end, row_start, row_end) = 
            transform.clip_bounds(col_min, row_min, col_max, row_max, nrows, ncols);

        for row in row_start..row_end {
            let row_offset = row * ncols;
            for col in col_start..col_end {
                let pt = transform.pixel_to_geo(col, row);
                let dist = point_to_line_distance(&pt, ls);
                
                if dist <= width {
                    let idx = row_offset + col;
                    
                    if value > raster[idx] {
                        raster[idx] = value;
                    }
                    mask[idx] = 1.0;
                }
            }
        }
    }
}

/// Rasterize vector features from GeoJSON into a resistance raster and layer masks.
///
/// The behavior is max value logic: overlapping features take the max resistance.
///
/// # Arguments
///
/// * `geojson_str` - The GeoJSON string containing features to rasterize.
/// * `layer_params` - A mapping of layer names to their parameters (resistance and width).
/// * `base_raster` - The base resistance raster data.
/// * `nrows` - Number of rows in the raster.
/// * `ncols` - Number of columns in the raster.        
pub fn rasterize_features(
    geojson_str: &str,
    layer_params: &HashMap<String, LayerParams>,
    base_raster: &[f64],
    nrows: usize,
    ncols: usize,
    transform: &GeoTransform,
    warnings: &mut Vec<String>,
) -> (Vec<f64>, HashMap<String, Vec<f64>>) {
    let mut res_map = base_raster.to_vec();
    let n = nrows * ncols;
    let mut layer_masks: HashMap<String, Vec<f64>> = HashMap::new();

    let geojson: GeoJson = match geojson_str.parse() {
        Ok(g) => g,
        Err(e) => {
            warnings.push(format!("geojson parse failed: {}", e));
            return (res_map, layer_masks);
        }
    };

    let features = match geojson {
        GeoJson::FeatureCollection(fc) => fc.features,
        _ => {
            warnings.push("expected FeatureCollection at top level".to_string());
            return (res_map, layer_masks);
        }
    };

    for feature in &features {
        let props = match &feature.properties {
            Some(p) => p,
            None => continue,
        };

        let layer_name = match props.get("layer").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => continue,
        };

        let params = match layer_params.get(layer_name) {
            Some(p) => p,
            None => continue,
        };

        let geojson_geom = match &feature.geometry {
            Some(g) => g,
            None => continue,
        };

        let geo_geom = match Geometry::<f64>::try_from(geojson_geom) {
            Ok(g) => g,
            Err(_) => continue,
        };

        let feature_resistance = props
            .get("resistanceValue")
            .and_then(|v| v.as_f64())
            .unwrap_or(params.resistance);

        let mask = layer_masks.entry(layer_name.to_string()).or_insert_with(|| vec![0.0f64; n]);

        match geo_geom {
            Geometry::Polygon(poly) => {
                rasterize_polygon(&mut res_map, mask, nrows, ncols, &poly, feature_resistance, transform);
            }
            Geometry::MultiPolygon(mpoly) => {
                for poly in &mpoly {
                    rasterize_polygon(&mut res_map, mask, nrows, ncols, poly, feature_resistance, transform);
                }
            }
            Geometry::LineString(ls) => {
                let lines = [&ls];
                rasterize_lines(&mut res_map, mask, nrows, ncols, &lines, feature_resistance, params.width, transform);
            }
            Geometry::MultiLineString(mls) => {
                let lines: Vec<&LineString<f64>> = mls.0.iter().collect();
                rasterize_lines(&mut res_map, mask, nrows, ncols, &lines, feature_resistance, params.width, transform);
            }
            _ => {}
        }
    }

    (res_map, layer_masks)
}

#[derive(Serialize)]
pub struct LayerMask {
    pub name: String,
    pub data: Vec<f64>,
}

#[derive(Serialize)]
pub struct GeospatialOutput {
    pub resistance_map: Vec<f64>,
    pub current_map: Vec<f64>,
    pub voltage_map: Vec<f64>,
    pub layer_masks: Vec<LayerMask>,
    pub nrows: usize,
    pub ncols: usize,
    pub warnings: Vec<String>,
    /// Total PCG iterations across all component solves.
    pub total_iters: usize,
}

pub fn prepare_geospatial_layers(
    base_raster: &[f64],
    nrows: usize,
    ncols: usize,
    geojson_str: &str,
    layer_params_str: &str,
    xmin: f64,
    ymax: f64,
    cellsize: f64,
) -> (Vec<f64>, Vec<LayerMask>, Vec<String>) {
    let layer_params: HashMap<String, LayerParams> = match serde_json::from_str(layer_params_str) {
        Ok(p) => p,
        Err(e) => {
            let mut warnings = vec![format!("layer params parse failed: {}", e)];
            let transform = GeoTransform { xmin, ymax, cellsize };
            let (resistance_data, _m) = rasterize_features(
                geojson_str, &HashMap::new(), base_raster, nrows, ncols, &transform, &mut warnings,
            );
            return (resistance_data, Vec::new(), warnings);
        }
    };

    let transform = GeoTransform { xmin, ymax, cellsize };
    let mut warnings = Vec::new();
    let (resistance_data, m) = rasterize_features(
        geojson_str, &layer_params, base_raster, nrows, ncols, &transform, &mut warnings,
    );
    let mut layer_masks: Vec<LayerMask> = m.into_iter()
        .filter(|(_, v)| v.iter().any(|&x| x > 0.0))
        .map(|(name, data)| LayerMask { name, data })
        .collect();
    layer_masks.sort_by(|a, b| a.name.cmp(&b.name));
    (resistance_data, layer_masks, warnings)
}

pub fn run_geospatial_pipeline_cached_mg(
    base_raster: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
    geojson_str: &str,
    layer_params_str: &str,
    xmin: f64,
    ymax: f64,
    cellsize: f64,
    source_data: &[f64],
    ground_data: &[f64],
    max_iter: usize,
    tol: f64,
    ground_mode: crate::solve::GroundMode,
) -> GeospatialOutput {
    let (resistance_data, layer_masks, warnings) = prepare_geospatial_layers(
        base_raster, nrows, ncols, geojson_str, layer_params_str, xmin, ymax, cellsize,
    );

    let annotated = crate::solve::solve_raster_sources_mg(
        &resistance_data, nrows, ncols, nodata,
        source_data, ground_data, max_iter, tol, true, ground_mode,
    );

    GeospatialOutput {
        resistance_map: resistance_data,
        current_map: annotated.output.current_map,
        voltage_map: annotated.output.voltages,
        layer_masks,
        nrows: annotated.output.nrows,
        ncols: annotated.output.ncols,
        warnings,
        total_iters: annotated.total_iters,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_base_raster(nrows: usize, ncols: usize) -> Vec<f64> {
        vec![0.0; nrows * ncols]
    }

    fn make_test_transform() -> GeoTransform {
        GeoTransform {
            xmin: 0.0,
            ymax: 10.0,
            cellsize: 1.0,
        }
    }

    #[test]
    fn test_geo_to_pixel_mapping() {
        let transform = make_test_transform();
        
        // Top-left corner geographic coordinate should be pixel (0, 0)
        let (col, row) = transform.geo_to_pixel(0.5, 9.5);
        assert_eq!(col, 0);
        assert_eq!(row, 0);

        let (col, row) = transform.geo_to_pixel(5.5, 4.5);
        assert_eq!(col, 5);
        assert_eq!(row, 5);
    }

    #[test]
    fn test_rasterize_polygon_and_mask() {
        let transform = make_test_transform();
        let nrows = 10;
        let ncols = 10;
        let base = make_base_raster(nrows, ncols);

        // A 3x3 square polygon covering pixels from col 2 to 4, and row 2 to 4 geographically
        let geojson_str = r#"{
            "type": "FeatureCollection",
            "features": [
                {
                    "type": "Feature",
                    "properties": { "layer": "zone_a" },
                    "geometry": {
                        "type": "Polygon",
                        "coordinates": [[
                            [2.0, 5.0],
                            [5.0, 5.0],
                            [5.0, 8.0],
                            [2.0, 8.0],
                            [2.0, 5.0]
                        ]]
                    }
                }
            ]
        }"#;

        let mut layer_params = HashMap::new();
        layer_params.insert("zone_a".to_string(), LayerParams { resistance: 50.0, width: 0.0 });

        let (res_map, masks) = rasterize_features(geojson_str, &layer_params, &base, nrows, ncols, &transform, &mut Vec::new());

        assert!(masks.contains_key("zone_a"));
        let mask = &masks["zone_a"];

        // Check resistance inside the building is set to 50.0 and mask is 1.0
        let idx_inside = 3 * ncols + 3;
        assert_eq!(res_map[idx_inside], 50.0);
        assert_eq!(mask[idx_inside], 1.0);

        // And check outside is 0.0 and mask is 0.0
        let idx_outside = 0;
        assert_eq!(res_map[idx_outside], 0.0);
        assert_eq!(mask[idx_outside], 0.0);
    }

    #[test]
    fn test_rasterize_lines_with_width_buffer() {
        let transform = make_test_transform();
        let nrows = 10;
        let ncols = 10;
        let base = make_base_raster(nrows, ncols);

        let geojson_str = r#"{
            "type": "FeatureCollection",
            "features": [
                {
                    "type": "Feature",
                    "properties": { "layer": "road" },
                    "geometry": {
                        "type": "LineString",
                        "coordinates": [[5.0, 3.0], [5.0, 7.0]]
                    }
                }
            ]
        }"#;

        let mut layer_params = HashMap::new();
        layer_params.insert("road".to_string(), LayerParams { resistance: 25.0, width: 1.5 });

        let (res_map, masks) = rasterize_features(geojson_str, &layer_params, &base, nrows, ncols, &transform, &mut Vec::new());

        assert!(masks.contains_key("road"));
        let mask = &masks["road"];

        // Pixel exactly on the line is col 5, row 5 or geo (5.5, 4.5). Distance to line (x=5) is 0.5.
        // 0.5 <= 1.5 width, therefore within buffer
        let idx_on_line = 5 * ncols + 5;
        assert_eq!(res_map[idx_on_line], 25.0);
        assert_eq!(mask[idx_on_line], 1.0);

        // Pixel away from line but still within buffer, still should be filled
        let idx_buffered = 5 * ncols + 6;
        assert_eq!(res_map[idx_buffered], 25.0);
        assert_eq!(mask[idx_buffered], 1.0);

        // Out of line/buffer, should remain 0.0 and mask 0.0
        let idx_far = ncols + 1;
        assert_eq!(res_map[idx_far], 0.0);
        assert_eq!(mask[idx_far], 0.0);
    }

    #[test]
    fn test_max_resistance_logic() {
        let transform = make_test_transform();
        let nrows = 5;
        let ncols = 5;
        
        // Seed a base raster with large vals
        let mut base = make_base_raster(nrows, ncols);
        base[0] = 100.0; // Seed high value at top left corner pixel

        // Create a feature that overlaps with pixel 0, but has a lower resistance
        let geojson_str = r#"{
            "type": "FeatureCollection",
            "features": [
                {
                    "type": "Feature",
                    "properties": { "layer": "low_resistance_zone" },
                    "geometry": {
                        "type": "Polygon",
                        "coordinates": [[
                            [0.0, 9.0],
                            [2.0, 9.0],
                            [2.0, 10.0],
                            [0.0, 10.0],
                            [0.0, 9.0]
                        ]]
                    }
                }
            ]
        }"#;

        let mut layer_params = HashMap::new();
        layer_params.insert("low_resistance_zone".to_string(), LayerParams { resistance: 10.0, width: 0.0 });

        let (res_map, masks) = rasterize_features(geojson_str, &layer_params, &base, nrows, ncols, &transform, &mut Vec::new());

        // Should keep the 100.0 value.
        assert_eq!(res_map[0], 100.0);
        
        // However, the mask tracking should still log that the feature layer touched this pixel.
        assert_eq!(masks["low_resistance_zone"][0], 1.0);
    }
}