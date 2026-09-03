//! PNG rendering of the error surface.
//!
//! Reproduces the layout of `python/main.py --plot`: a parula heatmap of the
//! normalised loss, white contour lines, detector markers (size proportional
//! to calls), the predicted roost, the weighted mean, and an optional known
//! roost marker. The axes use an equal aspect ratio (metres map to pixels the
//! same in x and y), matching the paper.

use image::{DynamicImage, Rgb, RgbImage};
use plotters::prelude::*;
use wasm_connect::roost::parula::parula_u8;

pub struct PlotData<'a> {
    /// Surface loss values, row-major (`y` outer, `x` inner).
    pub surface: &'a [f64],
    pub grid_size: usize,
    pub xmin: f64,
    pub xmax: f64,
    pub ymin: f64,
    pub ymax: f64,
    pub detectors_x: &'a [f64],
    pub detectors_y: &'a [f64],
    pub counts: &'a [f64],
    pub predicted: (f64, f64),
    pub weighted_mean: (f64, f64),
    pub known_roost: Option<(f64, f64)>,
    pub loss: f64,
}

const ORANGE: RGBColor = RGBColor(255, 140, 0);

// Layout estimates (label areas + margins). The plotting area is filled by the
// heatmap; everything else is axis labels / caption / legend.
const LEFT: f64 = 63.0;
const RIGHT: f64 = 14.0;
const TOP: f64 = 40.0;
const BOTTOM: f64 = 52.0;
const PLOT_HEIGHT: f64 = 560.0;

pub fn render(path: &str, data: &PlotData) -> Result<(), String> {
    let max_loss = data
        .surface
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    if !(max_loss > 0.0) {
        return Err("surface has no positive loss values; nothing to plot".to_string());
    }

    let norm: Vec<f64> = data.surface.iter().map(|v| v / max_loss).collect();
    let lut = parula_u8(256, "new");

    let xspan = data.xmax - data.xmin;
    let yspan = data.ymax - data.ymin;
    let aspect = xspan / yspan;

    // Size the canvas so the plotting area has the data's aspect ratio.
    let plot_w = PLOT_HEIGHT * aspect;
    let width = (plot_w + LEFT + RIGHT).round().max(400.0) as u32;
    let height = (PLOT_HEIGHT + TOP + BOTTOM).round() as u32;

    let root = BitMapBackend::new(path, (width, height)).into_drawing_area();
    root.fill(&WHITE).map_err(|e| format!("{e:?}"))?;

    let mut chart = ChartBuilder::on(&root)
        .caption(
            format!(
                "Predicted roost: ({:.0}, {:.0})  loss={:.4}",
                data.predicted.0, data.predicted.1, data.loss
            ),
            ("sans-serif", 18),
        )
        .margin(8)
        .x_label_area_size(40)
        .y_label_area_size(55)
        .build_cartesian_2d(data.xmin..data.xmax, data.ymin..data.ymax)
        .map_err(|e| format!("{e:?}"))?;

    chart
        .configure_mesh()
        .x_desc("Eastings (m)")
        .y_desc("Northings (m)")
        .x_label_style(("sans-serif", 14))
        .y_label_style(("sans-serif", 14))
        .draw()
        .map_err(|e| format!("{e:?}"))?;

    // Heatmap + contour bands, rendered directly at the plot resolution so
    // both the colormap and the contour lines come out smooth.
    let (pw, ph) = chart.plotting_area().dim_in_pixel();
    let heatmap = build_heatmap(&norm, data.grid_size, &lut, pw, ph);
    let elem: BitMapElement<(f64, f64)> = ((data.xmin, data.ymax), DynamicImage::ImageRgb8(heatmap)).into();
    chart
        .draw_series(std::iter::once(elem))
        .map_err(|e| format!("{e:?}"))?;

    // Metres-per-pixel, for building fixed-size markers in data coordinates.
    let ppx = pw as f64 / xspan;
    let ppy = ph as f64 / yspan;

    // Detectors: black circles, size proportional to calls.
    let cmax = data.counts.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let detector_series = data
        .detectors_x
        .iter()
        .zip(data.detectors_y.iter())
        .zip(data.counts.iter())
        .map(|((&x, &y), &c)| {
            let r = (2.0 + 6.0 * (c / cmax)).round() as u32;
            Circle::new((x, y), r, BLACK.filled())
        });
    chart
        .draw_series(detector_series)
        .map_err(|e| format!("{e:?}"))?
        .label("Detectors")
        .legend(|(x, y)| Circle::new((x, y), 3, BLACK.filled()));

    // Predicted roost: red diamond (~8 px half-size).
    let diamond = |(px, py): (f64, f64)| {
        let dx = 8.0 / ppx;
        let dy = 8.0 / ppy;
        Polygon::new(
            vec![(px, py + dy), (px + dx, py), (px, py - dy), (px - dx, py)],
            RED.filled(),
        )
    };
    chart
        .draw_series(std::iter::once(diamond(data.predicted)))
        .map_err(|e| format!("{e:?}"))?
        .label("Predicted roost")
        .legend(legend_diamond);

    // Weighted mean: blue square (~8 px half-size).
    let square = |(px, py): (f64, f64)| {
        let dx = 8.0 / ppx;
        let dy = 8.0 / ppy;
        Rectangle::new([(px - dx, py - dy), (px + dx, py + dy)], BLUE.filled())
    };
    chart
        .draw_series(std::iter::once(square(data.weighted_mean)))
        .map_err(|e| format!("{e:?}"))?
        .label("Weighted mean")
        .legend(legend_square);

    // Known roost: orange circle.
    if let Some(roost) = data.known_roost {
        chart
            .draw_series(std::iter::once(Circle::new(roost, 9, ORANGE.filled())))
            .map_err(|e| format!("{e:?}"))?
            .label("Known roost")
            .legend(|(x, y)| Circle::new((x, y), 4, ORANGE.filled()));
    }

    chart
        .configure_series_labels()
        .background_style(WHITE.mix(0.8))
        .border_style(BLACK)
        .draw()
        .map_err(|e| format!("{e:?}"))?;

    root.present().map_err(|e| format!("{e:?}"))
}

/// Contour levels (as fractions of the maximum loss) drawn as white bands.
const CONTOUR_LEVELS: [f64; 4] = [0.1, 0.2, 0.3, 0.4];
/// Contour line half-width in pixels (band thickness stays roughly constant
/// by scaling the value-epsilon by the local gradient magnitude).
const CONTOUR_WIDTH: f64 = 0.75;

fn color_at(lut: &[[u8; 3]], v: f64, mag_px: f64) -> Rgb<u8> {
    if mag_px > 1e-12 {
        for &level in &CONTOUR_LEVELS {
            if (v - level).abs() / mag_px < CONTOUR_WIDTH {
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
fn build_heatmap(norm: &[f64], n: usize, lut: &[[u8; 3]], out_w: u32, out_h: u32) -> RgbImage {
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
            img.put_pixel(ox, oy, color_at(lut, v, mag));
        }
    }
    img
}

fn legend_diamond((x, y): (i32, i32)) -> Polygon<(i32, i32)> {
    Polygon::new(
        vec![(x, y + 7), (x + 7, y), (x, y - 7), (x - 7, y)],
        RED.filled(),
    )
}

fn legend_square((x, y): (i32, i32)) -> Rectangle<(i32, i32)> {
    Rectangle::new([(x - 5, y - 5), (x + 5, y + 5)], BLUE.filled())
}
