use super::*;

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
    let transform = GeoTransform { xmin: 0.0, ymax: 10.0, cellsize: 1.0 };
    
    // Top-left corner geographic coordinate should be pixel (0, 0)
    let (col, row) = transform.geo_to_pixel(0.5, 9.5);
    assert_eq!(col, 0);
    assert_eq!(row, 0);

    let (col, row) = transform.geo_to_pixel(5.5, 4.5);
    assert_eq!(col, 5);
    assert_eq!(row, 5);
}
