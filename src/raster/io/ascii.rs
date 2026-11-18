use super::{raster_error, RasterResult};
use crate::linalg::real::{Real, scalar};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

pub struct AscGrid<F: Real = f64> {
    pub data: Vec<F>,
    pub nrows: usize,
    pub ncols: usize,
    pub xllcorner: f64,
    pub yllcorner: f64,
    pub cellsize: f64,
    pub nodata: F,
    pub ymax: f64,
}

pub fn parse_asc<P: AsRef<Path>>(path: P) -> AscGrid { parse_asc_typed::<f64, P>(path) }

pub fn parse_asc_typed<F: Real, P: AsRef<Path>>(path: P) -> AscGrid<F> {
    let text = fs::read_to_string(path.as_ref())
        .unwrap_or_else(|e| panic!("cannot read {}: {}", path.as_ref().display(), e));

    let mut ncols = 0usize;
    let mut nrows = 0usize;
    let mut xllcorner = 0.0f64;
    let mut yllcorner = 0.0f64;
    let mut cellsize = 0.0f64;
    let mut nodata = scalar::<F>(-9999.0);
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
                    "nodata_value" => nodata = scalar::<F>(val),
                    _ => {}
                }
            }
        } else {
            for token in trimmed.split_whitespace() {
                data.push(token.parse::<F>().unwrap_or(nodata));
            }
        }
    }

    let ymax = yllcorner + nrows as f64 * cellsize;
    AscGrid { data, nrows, ncols, xllcorner, yllcorner, cellsize, nodata, ymax }
}

pub fn write_asc<F: Real, P: AsRef<Path>>(
    path: P, data: &[F], nrows: usize, ncols: usize,
    xllcorner: f64, yllcorner: f64, cellsize: f64, nodata: F,
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
            line.push_str(&format!("{:.16e}", data[row * ncols + col]));
        }
        writeln!(f, "{}", line).unwrap();
    }
}

pub(super) fn read_ascii_cells<F: Real>(
    path: &Path,
    expected_cells: usize,
) -> RasterResult<Vec<F>> {
    let text = fs::read_to_string(path)?;
    let mut values = Vec::with_capacity(expected_cells);
    for line in text.lines() {
        let mut tokens = line.split_whitespace();
        let Some(first) = tokens.next() else {
            continue;
        };
        if matches!(
            first.to_ascii_lowercase().as_str(),
            "ncols"
                | "nrows"
                | "xllcorner"
                | "yllcorner"
                | "xllcenter"
                | "yllcenter"
                | "cellsize"
                | "nodata_value"
        ) {
            continue;
        }
        for token in std::iter::once(first).chain(tokens) {
            values.push(token.parse::<F>().map_err(|_| {
                raster_error(format!("{}: invalid raster cell {token}", path.display()))
            })?);
        }
    }
    if values.len() != expected_cells {
        return Err(raster_error(format!("{}: incomplete raster data", path.display())).into());
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asc_export_preserves_small_currents_and_f64_precision() {
        let path = std::env::temp_dir().join(format!(
            "lmConnect-asc-{}-{}.asc",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        ));
        let values = [0.0, 5.3454610306142845e-43, 1e-300,
                      366.321540637836, -9999.0, f64::MIN_POSITIVE];
        write_asc(&path, &values, 2, 3, 10.0, 20.0, 1.0, -9999.0);
        let grid = parse_asc(&path);
        fs::remove_file(&path).unwrap();
        assert_eq!(grid.data, values);
        assert_eq!((grid.nrows, grid.ncols), (2, 3));
        assert_eq!((grid.xllcorner, grid.yllcorner, grid.cellsize, grid.nodata),
                   (10.0, 20.0, 1.0, -9999.0));
    }
}
