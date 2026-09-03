use serde::Serialize;

/// Resistance value used to fill nodata cells. 1e9 Ω makes the edge
/// conductance ~1e-9 — effectively an insulator for the physics but
/// keeps every pixel as a node so the grid is fully rectangular.
pub const FILL_RESISTANCE: f64 = 1e9;

/// Replace nodata / non-positive / non-finite resistance with
/// [`FILL_RESISTANCE`] so every cell becomes a graph node.
pub fn fill_nodata(data: &[f64], nodata: f64) -> Vec<f64> {
    data.iter()
        .map(|&v| {
            if v == nodata || !v.is_finite() || v <= 0.0 {
                FILL_RESISTANCE
            } else {
                v
            }
        })
        .collect()
}

#[derive(Serialize)]
pub struct DownsampleOutput {
    pub data: Vec<f64>,
    pub nrows: usize,
    pub ncols: usize,
}

/// Downsample a raster using block averaging. Ignores nodata pixels in each block.
/// If a block has NO valid data, the output pixel is nodata.
pub fn downsample_raster(
    data: &[f64],
    nrows: usize,
    ncols: usize,
    nodata: f64,
    target_rows: usize,
    target_cols: usize,
) -> DownsampleOutput {
    if target_rows >= nrows && target_cols >= ncols {
        return DownsampleOutput {
            data: data.to_vec(),
            nrows,
            ncols,
        };
    }

    let row_ratio = if target_rows >= nrows {
        1.0
    } else {
        nrows as f64 / target_rows as f64
    };
    let col_ratio = if target_cols >= ncols {
        1.0
    } else {
        ncols as f64 / target_cols as f64
    };

    let mut out = vec![nodata; target_rows * target_cols];

    for or in 0..target_rows {
        let src_row_start = (or as f64 * row_ratio).round() as usize;
        let src_row_end = ((or + 1) as f64 * row_ratio).round() as usize;
        let src_row_end = src_row_end.min(nrows);

        for oc in 0..target_cols {
            let src_col_start = (oc as f64 * col_ratio).round() as usize;
            let src_col_end = ((oc + 1) as f64 * col_ratio).round() as usize;
            let src_col_end = src_col_end.min(ncols);

            let mut sum = 0.0;
            let mut count = 0usize;

            for sr in src_row_start..src_row_end {
                for sc in src_col_start..src_col_end {
                    let v = data[sr * ncols + sc];
                    if v != nodata && !v.is_nan() {
                        sum += v;
                        count += 1;
                    }
                }
            }

            if count > 0 {
                out[or * target_cols + oc] = sum / count as f64;
            }
        }
    }

    DownsampleOutput {
        data: out,
        nrows: target_rows,
        ncols: target_cols,
    }
}
