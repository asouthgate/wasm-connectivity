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

pub struct GeoTransform {
    pub xmin: f64,
    pub ymax: f64,
    pub cellsize: f64,
}

impl GeoTransform {
    fn geo_to_pixel(&self, x: f64, y: f64) -> (isize, isize) {
        let col = ((x - self.xmin) / self.cellsize) as isize;
        let row = ((self.ymax - y) / self.cellsize) as isize;
        (col, row)
    }

    fn pixel_to_geo(&self, col: usize, row: usize) -> Point<f64> {
        Point::new(
            self.xmin + (col as f64 + 0.5) * self.cellsize,
            self.ymax - (row as f64 + 0.5) * self.cellsize,
        )
    }
}

pub fn parse_layer_params(json: &str) -> HashMap<String, LayerParams> {
    serde_json::from_str(json).unwrap_or_default()
}

fn rasterize_polygon(
    raster: &mut [f64],
    nrows: usize,
    ncols: usize,
    poly: &Polygon<f64>,
    resistance: f64,
    transform: &GeoTransform,
) {
    let bbox = match poly.bounding_rect() {
        Some(b) => b,
        None => return,
    };
    let minc = bbox.min();
    let maxc = bbox.max();

    let (col_min, row_max) = transform.geo_to_pixel(minc.x, minc.y);
    let (col_max, row_min) = transform.geo_to_pixel(maxc.x, maxc.y);

    let col_start = col_min.max(0) as usize;
    let col_end = (col_max + 1).min(ncols as isize).max(0) as usize;
    let row_start = row_min.max(0) as usize;
    let row_end = (row_max + 1).min(nrows as isize).max(0) as usize;

    for row in row_start..row_end {
        for col in col_start..col_end {
            let pt = transform.pixel_to_geo(col, row);
            if poly.contains(&pt) {
                let idx = row * ncols + col;
                if resistance > raster[idx] {
                    raster[idx] = resistance;
                }
            }
        }
    }
}

fn rasterize_polygon_mask(
    mask: &mut [f64],
    nrows: usize,
    ncols: usize,
    poly: &Polygon<f64>,
    transform: &GeoTransform,
) {
    let bbox = match poly.bounding_rect() {
        Some(b) => b,
        None => return,
    };
    let minc = bbox.min();
    let maxc = bbox.max();

    let (col_min, row_max) = transform.geo_to_pixel(minc.x, minc.y);
    let (col_max, row_min) = transform.geo_to_pixel(maxc.x, maxc.y);

    let col_start = col_min.max(0) as usize;
    let col_end = (col_max + 1).min(ncols as isize).max(0) as usize;
    let row_start = row_min.max(0) as usize;
    let row_end = (row_max + 1).min(nrows as isize).max(0) as usize;

    for row in row_start..row_end {
        for col in col_start..col_end {
            let pt = transform.pixel_to_geo(col, row);
            if poly.contains(&pt) {
                mask[row * ncols + col] = 1.0;
            }
        }
    }
}

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

fn rasterize_lines(
    raster: &mut [f64],
    nrows: usize,
    ncols: usize,
    lines: &[LineString<f64>],
    resistance: f64,
    width: f64,
    transform: &GeoTransform,
) {
    if width <= 0.0 || lines.is_empty() {
        return;
    }

    let mut global_col_min = ncols as isize;
    let mut global_row_min = nrows as isize;
    let mut global_col_max: isize = 0;
    let mut global_row_max: isize = 0;

    for ls in lines {
        if let Some(bbox) = ls.bounding_rect() {
            let minc = bbox.min();
            let maxc = bbox.max();
            let (cmin, rmax) = transform.geo_to_pixel(minc.x - width, minc.y - width);
            let (cmax, rmin) = transform.geo_to_pixel(maxc.x + width, maxc.y + width);

            global_col_min = global_col_min.min(cmin);
            global_row_min = global_row_min.min(rmin);
            global_col_max = global_col_max.max(cmax);
            global_row_max = global_row_max.max(rmax);
        }
    }

    let col_start = global_col_min.max(0) as usize;
    let col_end = (global_col_max + 1).min(ncols as isize).max(0) as usize;
    let row_start = global_row_min.max(0) as usize;
    let row_end = (global_row_max + 1).min(nrows as isize).max(0) as usize;

    for row in row_start..row_end {
        for col in col_start..col_end {
            let pt = transform.pixel_to_geo(col, row);
            for ls in lines {
                let dist = point_to_line_distance(&pt, ls);
                if dist <= width {
                    let idx = row * ncols + col;
                    if resistance > raster[idx] {
                        raster[idx] = resistance;
                    }
                    break;
                }
            }
        }
    }
}

fn rasterize_lines_mask(
    mask: &mut [f64],
    nrows: usize,
    ncols: usize,
    lines: &[LineString<f64>],
    width: f64,
    transform: &GeoTransform,
) {
    if width <= 0.0 || lines.is_empty() {
        return;
    }

    let mut global_col_min = ncols as isize;
    let mut global_row_min = nrows as isize;
    let mut global_col_max: isize = 0;
    let mut global_row_max: isize = 0;

    for ls in lines {
        if let Some(bbox) = ls.bounding_rect() {
            let minc = bbox.min();
            let maxc = bbox.max();
            let (cmin, rmax) = transform.geo_to_pixel(minc.x - width, minc.y - width);
            let (cmax, rmin) = transform.geo_to_pixel(maxc.x + width, maxc.y + width);

            global_col_min = global_col_min.min(cmin);
            global_row_min = global_row_min.min(rmin);
            global_col_max = global_col_max.max(cmax);
            global_row_max = global_row_max.max(rmax);
        }
    }

    let col_start = global_col_min.max(0) as usize;
    let col_end = (global_col_max + 1).min(ncols as isize).max(0) as usize;
    let row_start = global_row_min.max(0) as usize;
    let row_end = (global_row_max + 1).min(nrows as isize).max(0) as usize;

    for row in row_start..row_end {
        for col in col_start..col_end {
            let pt = transform.pixel_to_geo(col, row);
            for ls in lines {
                let dist = point_to_line_distance(&pt, ls);
                if dist <= width {
                    mask[row * ncols + col] = 1.0;
                    break;
                }
            }
        }
    }
}

pub fn rasterize_features(
    geojson_str: &str,
    layer_params: &HashMap<String, LayerParams>,
    base_raster: &[f64],
    nrows: usize,
    ncols: usize,
    transform: &GeoTransform,
    _nodata: f64,
) -> (Vec<f64>, HashMap<String, Vec<f64>>) {
    let mut res_map = base_raster.to_vec();
    let n = nrows * ncols;
    let mut layer_masks: HashMap<String, Vec<f64>> = HashMap::new();

    let geojson: GeoJson = match geojson_str.parse() {
        Ok(g) => g,
        Err(_) => return (res_map, layer_masks),
    };

    let features = match geojson {
        GeoJson::FeatureCollection(fc) => fc.features,
        _ => return (res_map, layer_masks),
    };

    for feature in &features {
        let props = match &feature.properties {
            Some(p) => p,
            None => continue,
        };

        let layer_name = match props.get("layer").and_then(|v| v.as_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        let params = match layer_params.get(&layer_name) {
            Some(p) => p,
            None => continue,
        };

        let geojson_geom = match &feature.geometry {
            Some(g) => g,
            None => continue,
        };

        let geo_geom = match Geometry::<f64>::try_from(geojson_geom.clone()) {
            Ok(g) => g,
            Err(_) => continue,
        };

        let mask = layer_masks.entry(layer_name.clone()).or_insert_with(|| vec![0.0f64; n]);

        match geo_geom {
            Geometry::Polygon(poly) => {
                rasterize_polygon(&mut res_map, nrows, ncols, &poly, params.resistance, transform);
                rasterize_polygon_mask(mask, nrows, ncols, &poly, transform);
            }
            Geometry::MultiPolygon(mpoly) => {
                for poly in &mpoly {
                    rasterize_polygon(&mut res_map, nrows, ncols, poly, params.resistance, transform);
                    rasterize_polygon_mask(mask, nrows, ncols, poly, transform);
                }
            }
            Geometry::LineString(ls) => {
                let ls2 = ls.clone();
                rasterize_lines(&mut res_map, nrows, ncols, &[ls], params.resistance, params.width, transform);
                rasterize_lines_mask(mask, nrows, ncols, &[ls2], params.width, transform);
            }
            Geometry::MultiLineString(mls) => {
                let lines: Vec<LineString<f64>> = mls.into_iter().collect();
                let lines2 = lines.clone();
                rasterize_lines(&mut res_map, nrows, ncols, &lines, params.resistance, params.width, transform);
                rasterize_lines_mask(mask, nrows, ncols, &lines2, params.width, transform);
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
}

pub fn solve_geospatial(
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
) -> GeospatialOutput {
    let layer_params = parse_layer_params(layer_params_str);
    let transform = GeoTransform { xmin, ymax, cellsize };

    let (resistance_data, m) = rasterize_features(
        geojson_str, &layer_params, base_raster, nrows, ncols, &transform, nodata,
    );

    let layer_masks: Vec<LayerMask> = m.into_iter()
        .filter(|(_, v)| v.iter().any(|&x| x > 0.0))
        .map(|(name, data)| LayerMask { name, data })
        .collect();

    let raster_output = crate::raster::compute_raster(
        &resistance_data, nrows, ncols, nodata, source_data, ground_data, max_iter, tol,
    );

    GeospatialOutput {
        resistance_map: resistance_data,
        current_map: raster_output.current_map,
        voltage_map: raster_output.voltages,
        layer_masks,
        nrows: raster_output.nrows,
        ncols: raster_output.ncols,
    }
}
