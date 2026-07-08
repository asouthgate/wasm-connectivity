use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

pub struct AscGrid {
    pub data: Vec<f64>,
    pub nrows: usize,
    pub ncols: usize,
    pub xllcorner: f64,
    pub yllcorner: f64,
    pub cellsize: f64,
    pub nodata: f64,
    pub ymax: f64,
}

pub fn parse_asc<P: AsRef<Path>>(path: P) -> AscGrid {
    let text = fs::read_to_string(path.as_ref())
        .unwrap_or_else(|e| panic!("cannot read {}: {}", path.as_ref().display(), e));
    parse_asc_str(&text)
}

pub fn parse_asc_str(text: &str) -> AscGrid {
    let mut ncols = 0usize;
    let mut nrows = 0usize;
    let mut xllcorner = 0.0f64;
    let mut yllcorner = 0.0f64;
    let mut cellsize = 0.0f64;
    let mut nodata = -9999.0f64;
    let mut data = Vec::new();

    for (i, line) in BufReader::new(text.as_bytes()).lines().enumerate() {
        let line = line.unwrap_or_default();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if i < 6 {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 2 {
                let val = parts[1].parse::<f64>().unwrap_or(0.0);
                match parts[0].to_lowercase().as_str() {
                    "ncols" => ncols = val as usize,
                    "nrows" => nrows = val as usize,
                    "xllcorner" | "xllcenter" => xllcorner = val,
                    "yllcorner" | "yllcenter" => yllcorner = val,
                    "cellsize" => cellsize = val,
                    "nodata_value" => nodata = val,
                    _ => {}
                }
            }
        } else {
            for token in trimmed.split_whitespace() {
                data.push(token.parse::<f64>().unwrap_or(nodata));
            }
        }
    }

    let ymax = yllcorner + nrows as f64 * cellsize;
    AscGrid { data, nrows, ncols, xllcorner, yllcorner, cellsize, nodata, ymax }
}

pub fn write_asc<P: AsRef<Path>>(
    path: P, data: &[f64], nrows: usize, ncols: usize,
    xllcorner: f64, yllcorner: f64, cellsize: f64, nodata: f64,
) {
    let mut f = fs::File::create(path.as_ref())
        .unwrap_or_else(|e| panic!("cannot create {}: {}", path.as_ref().display(), e));
    writeln!(f, "ncols {}", ncols).unwrap();
    writeln!(f, "nrows {}", nrows).unwrap();
    writeln!(f, "xllcorner {:.6}", xllcorner).unwrap();
    writeln!(f, "yllcorner {:.6}", yllcorner).unwrap();
    writeln!(f, "cellsize {:.6}", cellsize).unwrap();
    writeln!(f, "NODATA_value {}", nodata).unwrap();
    for row in 0..nrows {
        let mut line = String::new();
        for col in 0..ncols {
            if col > 0 {
                line.push(' ');
            }
            line.push_str(&format!("{:.6}", data[row * ncols + col]));
        }
        writeln!(f, "{}", line).unwrap();
    }
}

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
