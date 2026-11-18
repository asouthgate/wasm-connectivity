use super::GeoTransform;
use crate::real::{Real, scalar};
use geojson::GeoJson;
use geo::{
    BoundingRect, Contains, EuclideanDistance, Geometry, LineString,
    Point, Polygon,
};
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

fn rasterize_lines_bresenham<F: Real>(
    raster: &mut [F],
    mask: &mut [F],
    nrows: usize,
    ncols: usize,
    lines: &[&LineString<f64>],
    value: F,
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
                    mask[idx] = F::one();
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
fn rasterize_polygon<F: Real>(
    raster: &mut [F],
    mask: &mut [F],
    nrows: usize,
    ncols: usize,
    poly: &Polygon<f64>,
    value: F,
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
                mask[idx] = F::one();
            }
        }
    }
}

/// Rasterizes a set of lines into a raster grid, updating the raster values and mask.
fn rasterize_lines<F: Real>(
    raster: &mut [F],
    mask: &mut [F],
    nrows: usize,
    ncols: usize,
    lines: &[&LineString<f64>],
    value: F,
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
                    mask[idx] = F::one();
                }
            }
        }
    }
}

/// Rasterize each feature's numeric `properties.value` onto a grid filled with zero outside features.
/// Overlaps take the maximum value. `buffer` is the line buffer radius in grid units.
pub fn rasterize_values<F: Real>(
    geojson_str: &str, nrows: usize, ncols: usize,
    transform: &GeoTransform, buffer: f64,
) -> Result<Vec<F>, String> {
    rasterize_values_with_background(geojson_str, nrows, ncols, transform, buffer, 0.0)
}

/// Rasterize numeric feature values with an explicit value outside all features.
/// Overlaps take the maximum feature value, independently of the background.
pub fn rasterize_values_with_background<F: Real>(
    geojson_str: &str, nrows: usize, ncols: usize,
    transform: &GeoTransform, buffer: f64, background: f64,
) -> Result<Vec<F>, String> {
    rasterize_field(geojson_str, "value", nrows, ncols, transform, buffer, background)
}

/// Rasterize a required numeric property without renaming feature attributes.
pub fn rasterize_field<F: Real>(
    geojson_str: &str, field: &str, nrows: usize, ncols: usize,
    transform: &GeoTransform, buffer: f64, background: f64,
) -> Result<Vec<F>, String> {
    rasterize_numeric(geojson_str, Some(field), nrows, ncols, transform, buffer, background)
}

/// Rasterize geometry alone: one on features, zero elsewhere.
pub fn rasterize_mask<F: Real>(
    geojson_str: &str, nrows: usize, ncols: usize, transform: &GeoTransform,
) -> Result<Vec<F>, String> {
    rasterize_numeric(geojson_str, None, nrows, ncols, transform, 0.0, 0.0)
}

fn rasterize_numeric<F: Real>(
    geojson_str: &str, field: Option<&str>, nrows: usize, ncols: usize,
    transform: &GeoTransform, buffer: f64, background: f64,
) -> Result<Vec<F>, String> {
    let background = scalar::<F>(background);
    if !background.is_finite() {
        return Err("background must be finite and fit raster precision".into());
    }
    let size = nrows.checked_mul(ncols).filter(|&n| n > 0)
        .ok_or("raster dimensions must be positive and fit in memory")?;
    if !transform.xmin.is_finite() || !transform.ymax.is_finite()
        || !transform.cellsize.is_finite() || transform.cellsize <= 0.0
        || !buffer.is_finite() || buffer < 0.0 {
        return Err("grid coordinates must be finite, cellsize positive, and buffer nonnegative".into());
    }
    let collection = match geojson_str.parse::<GeoJson>().map_err(|e| e.to_string())? {
        GeoJson::FeatureCollection(collection) => collection,
        _ => return Err("expected a GeoJSON FeatureCollection".into()),
    };
    let mut raster = vec![F::neg_infinity(); size];
    let mut mask = vec![F::zero(); size];
    for (index, feature) in collection.features.iter().enumerate() {
        let value = if let Some(field) = field {
            feature.properties.as_ref().and_then(|p| p.get(field))
                .and_then(|v| v.as_f64()).filter(|v| v.is_finite())
                .ok_or_else(|| format!("feature {index} requires a finite numeric properties.{field}"))?
        } else { 1.0 };
        let value = scalar::<F>(value);
        if !value.is_finite() { return Err(format!("feature {index} value exceeds raster precision")); }
        let geometry = feature.geometry.as_ref().ok_or_else(|| format!("feature {index} has no geometry"))?;
        let geometry = Geometry::<f64>::try_from(geometry)
            .map_err(|e| format!("feature {index}: {e}"))?;
        match geometry {
            Geometry::Polygon(poly) => rasterize_polygon(&mut raster, &mut mask, nrows, ncols, &poly, value, transform),
            Geometry::MultiPolygon(polys) => {
                for poly in &polys { rasterize_polygon(&mut raster, &mut mask, nrows, ncols, poly, value, transform); }
            }
            Geometry::LineString(line) => rasterize_lines(&mut raster, &mut mask, nrows, ncols, &[&line], value, buffer, transform),
            Geometry::MultiLineString(lines) => {
                let lines: Vec<_> = lines.0.iter().collect();
                rasterize_lines(&mut raster, &mut mask, nrows, ncols, &lines, value, buffer, transform);
            }
            _ => return Err(format!("feature {index}: expected polygon or line geometry")),
        }
    }
    for (value, covered) in raster.iter_mut().zip(mask) {
        if covered == F::zero() { *value = background; }
    }
    Ok(raster)
}
