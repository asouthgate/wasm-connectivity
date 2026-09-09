//! Shared heatmap rendering for the roost error surface.
//!
//! Both the CLI plot (`roost-locate`) and the WASM map overlay render the loss
//! surface through this module, so the colormap and contour logic are defined
//! in exactly one place.

use crate::roost::colormap::colormap_u8;
use image::{Rgb, RgbImage};

/// Render the `grid_size x grid_size` loss surface (row-major, `y` outer) to an
/// `out_w x out_h` heatmap image using the blue→yellow colormap.
///
/// The surface is normalised by its maximum loss; low loss (the predicted
/// roost) maps to the bright/warm end of the colormap. `contour_levels` are
/// fractions of the maximum loss drawn as white bands; pass an empty slice to
/// disable contours.
pub fn render_surface(
    surface: &[f64],
    grid_size: usize,
    out_w: u32,
    out_h: u32,
    contour_levels: &[f64],
    contour_width: f64,
) -> Result<RgbImage, String> {
    if surface.len() != grid_size * grid_size {
        return Err(format!(
            "surface length {} does not match grid_size {}",
            surface.len(),
            grid_size
        ));
    }
    let max_loss = surface.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    if !(max_loss > 0.0) {
        return Err("surface has no positive loss values; nothing to render".to_string());
    }

    let norm: Vec<f64> = surface.iter().map(|v| v / max_loss).collect();
    let lut = colormap_u8(256);
    Ok(build_heatmap(
        &norm,
        grid_size,
        &lut,
        out_w,
        out_h,
        contour_levels,
        contour_width,
    ))
}

/// Contour levels (as fractions of the maximum loss) drawn as white bands.
fn color_at(lut: &[[u8; 3]], v: f64, mag_px: f64, levels: &[f64], half_width: f64) -> Rgb<u8> {
    if mag_px > 1e-12 {
        for &level in levels {
            if (v - level).abs() / mag_px < half_width {
                return Rgb([255, 255, 255]);
            }
        }
    }
    let v = v.clamp(0.0, 1.0);
    let idx = ((1.0 - v) * 255.0).round() as usize;
    let [r, g, b] = lut[idx.min(255)];
    Rgb([r, g, b])
}

/// Central-difference gradient of the `n x n` grid, returned as two
/// `n x n` fields in value-per-grid-index units.
fn gradient(norm: &[f64], n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut gx = vec![0.0; n * n];
    let mut gy = vec![0.0; n * n];
    for i in 0..n {
        for j in 0..n {
            let jl = if j == 0 { 0 } else { j - 1 };
            let jr = if j == n - 1 { n - 1 } else { j + 1 };
            let il = if i == 0 { 0 } else { i - 1 };
            let ir = if i == n - 1 { n - 1 } else { i + 1 };
            gx[i * n + j] = (norm[i * n + jr] - norm[i * n + jl]) / ((jr - jl) as f64);
            gy[i * n + j] = (norm[ir * n + j] - norm[il * n + j]) / ((ir - il) as f64);
        }
    }
    (gx, gy)
}

/// Bilinear sample of the `n x n` grid (row-major, `y` outer) at continuous
/// index coordinates `(fx, fy)` in `[0, n-1]`.
fn sample_bilinear(norm: &[f64], n: usize, fx: f64, fy: f64) -> f64 {
    let fx = fx.clamp(0.0, (n - 1) as f64);
    let fy = fy.clamp(0.0, (n - 1) as f64);
    let x0 = fx.floor() as usize;
    let y0 = fy.floor() as usize;
    let x1 = (x0 + 1).min(n - 1);
    let y1 = (y0 + 1).min(n - 1);
    let tx = fx - x0 as f64;
    let ty = fy - y0 as f64;
    let v00 = norm[y0 * n + x0];
    let v10 = norm[y0 * n + x1];
    let v01 = norm[y1 * n + x0];
    let v11 = norm[y1 * n + x1];
    (1.0 - tx) * (1.0 - ty) * v00 + tx * (1.0 - ty) * v10 + (1.0 - tx) * ty * v01 + tx * ty * v11
}

/// Build the heatmap (with contour bands baked in) at `out_w x out_h` pixels.
/// Image row 0 corresponds to the largest y (north); the surface grid row 0 is
/// the smallest y.
fn build_heatmap(
    norm: &[f64],
    n: usize,
    lut: &[[u8; 3]],
    out_w: u32,
    out_h: u32,
    levels: &[f64],
    contour_width: f64,
) -> RgbImage {
    let (gx, gy) = gradient(norm, n);
    let mut img = RgbImage::new(out_w, out_h);
    let dw = (out_w - 1) as f64;
    let dh = (out_h - 1) as f64;
    let span = (n - 1) as f64;
    for oy in 0..out_h {
        let fy = if dh > 0.0 {
            (1.0 - oy as f64 / dh) * span
        } else {
            0.0
        };
        for ox in 0..out_w {
            let fx = if dw > 0.0 { ox as f64 / dw * span } else { 0.0 };
            let v = sample_bilinear(norm, n, fx, fy);
            // Gradient in value-per-output-pixel (index step -> pixel step).
            let gx_px = sample_bilinear(&gx, n, fx, fy) * span / dw;
            let gy_px = sample_bilinear(&gy, n, fx, fy) * span / dh;
            let mag = (gx_px * gx_px + gy_px * gy_px).sqrt();
            img.put_pixel(ox, oy, color_at(lut, v, mag, levels, contour_width));
        }
    }
    img
}
