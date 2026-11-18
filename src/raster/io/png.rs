use super::ascii::parse_asc;
use std::path::Path;

pub fn asc_to_png<P: AsRef<Path>, Q: AsRef<Path>>(asc_path: P, png_path: Q) {
    let grid = parse_asc(&asc_path);
    let n = grid.data.len();

    let mut min_val = f64::MAX;
    let mut max_val = f64::MIN;
    for &v in &grid.data {
        if v.is_finite() && v != grid.nodata {
            min_val = min_val.min(v);
            max_val = max_val.max(v);
        }
    }
    if max_val <= min_val {
        max_val = min_val + 1.0;
    }

    let scale = 255.0 / (max_val - min_val);
    let mut pixels = vec![0u8; n];
    for i in 0..n {
        let v = grid.data[i];
        if v.is_finite() && v != grid.nodata {
            let scaled = ((v - min_val) * scale).round() as u32;
            pixels[i] = scaled.min(255) as u8;
        }
    }

    let mut img = image::GrayImage::new(grid.ncols as u32, grid.nrows as u32);
    for row in 0..grid.nrows {
        for col in 0..grid.ncols {
            img.put_pixel(col as u32, row as u32, image::Luma([pixels[row * grid.ncols + col]]));
        }
    }
    img.save(&png_path).unwrap();
    eprintln!("wrote {}", png_path.as_ref().display());
}
