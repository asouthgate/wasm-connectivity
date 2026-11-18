use std::collections::HashMap;
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
fn value_rasterization_preserves_negative_values_and_f32_output() {
    let features = r#"{"type":"FeatureCollection","features":[
        {"type":"Feature","properties":{"value":-3},"geometry":{
            "type":"Polygon","coordinates":[[[1,1],[3,1],[3,3],[1,3],[1,1]]]}}
    ]}"#;
    let transform = GeoTransform { xmin: 0.0, ymax: 4.0, cellsize: 1.0 };
    let raster = rasterize_values::<f32>(features, 4, 4, &transform, 0.0).unwrap();
    assert_eq!(raster[0], 0.0);
    assert_eq!(raster[5], -3.0);
    assert!(raster.iter().all(|v| v.is_finite()));
    assert!(rasterize_values::<f32>(features, 0, 4, &transform, 0.0).is_err());
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
