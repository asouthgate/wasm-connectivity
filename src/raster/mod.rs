#[cfg(all(feature = "bin", not(target_arch = "wasm32")))]
pub mod io;

use serde::Serialize;

/// Resistance value used to fill nodata cells. 1e9 Ω makes the edge
/// conductance ~1e-9 — effectively an insulator for the physics but
/// keeps every pixel as a node so the grid is fully rectangular.
pub const FILL_RESISTANCE: f64 = 1e9;

/// Replace nodata / non-positive / non-finite resistance with
/// [`FILL_RESISTANCE`] so every cell becomes a graph node.
pub fn fill_nodata<F: crate::linalg::real::Real>(data: &[F], nodata: F) -> Vec<F> {
    data.iter()
        .map(|&v| {
            if v == nodata || !v.is_finite() || v <= F::zero() {
                F::from_f64(FILL_RESISTANCE).unwrap()
            } else {
                v
            }
        })
        .collect()
}

/// Nearest-neighbour upsampling in row-major order. Values, including nodata,
/// are copied unchanged; callers scale cell size to preserve the extent.
pub fn upsample_raster<F: crate::linalg::real::Real>(
    data: &[F], nrows: usize, ncols: usize, target_rows: usize, target_cols: usize,
) -> Vec<F> {
    assert!(nrows > 0 && ncols > 0, "source dimensions must be positive");
    assert_eq!(data.len(), nrows * ncols, "source dimensions must match its data");
    assert!(target_rows >= nrows && target_cols >= ncols, "upsampling cannot shrink a dimension");
    let mut output = vec![F::zero(); target_rows * target_cols];
    for row in 0..target_rows {
        let source_row = row * nrows / target_rows;
        for col in 0..target_cols {
            let source_col = col * ncols / target_cols;
            output[row * target_cols + col] = data[source_row * ncols + source_col];
        }
    }
    output
}

#[derive(Serialize)]
pub struct DownsampleOutput {
    pub data: Vec<f64>,
    pub nrows: usize,
    pub ncols: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_upsampling_preserves_alignment_values_and_nodata() {
        let input = [1.0, -9999.0, 0.0, f64::NAN];
        let output = upsample_raster(&input, 2, 2, 4, 4);
        for row in 0..4 {
            for col in 0..4 {
                assert_eq!(output[row * 4 + col].to_bits(), input[(row / 2) * 2 + col / 2].to_bits());
            }
        }
        assert_eq!(upsample_raster(&[1.0, 2.0, 3.0, 4.0], 2, 2, 3, 3),
            vec![1.0, 1.0, 2.0, 1.0, 1.0, 2.0, 3.0, 3.0, 4.0]);
        assert_eq!(upsample_raster(&[1.0, 2.0], 1, 2, 1, 2), vec![1.0, 2.0]);
    }
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
