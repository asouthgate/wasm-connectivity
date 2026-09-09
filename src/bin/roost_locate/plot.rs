//! PNG rendering of the error surface.

use image::DynamicImage;
use plotters::prelude::*;
use wasm_connect::roost::render::render_surface;

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

/// Presentation settings for rendering. The caller owns these values
/// (pixel sizes and contour definitions); the render code bakes in no tuning.
pub struct PlotConfig<'a> {
    /// Plotting area height in pixels; width follows the data's aspect ratio.
    pub plot_height: f64,
    /// Pixel margins around the plotting area (axis labels / caption / legend).
    pub margin_left: f64,
    pub margin_right: f64,
    pub margin_top: f64,
    pub margin_bottom: f64,
    /// Contour levels as fractions of the maximum loss, drawn as white bands.
    /// Must be non-empty.
    pub contour_levels: &'a [f64],
    /// Contour band half-width in pixels.
    pub contour_width: f64,
}

pub fn render(path: &str, data: &PlotData, config: &PlotConfig) -> Result<(), String> {
    let xspan = data.xmax - data.xmin;
    let yspan = data.ymax - data.ymin;
    let aspect = xspan / yspan;

    // Size the canvas so the plotting area has the data's aspect ratio.
    let plot_w = config.plot_height * aspect;
    let width = (plot_w + config.margin_left + config.margin_right)
        .round()
        .max(400.0) as u32;
    let height = (config.plot_height + config.margin_top + config.margin_bottom).round() as u32;

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
    let heatmap = render_surface(
        data.surface,
        data.grid_size,
        pw,
        ph,
        config.contour_levels,
        config.contour_width,
    )?;
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

fn legend_diamond((x, y): (i32, i32)) -> Polygon<(i32, i32)> {
    Polygon::new(
        vec![(x, y + 7), (x + 7, y), (x, y - 7), (x - 7, y)],
        RED.filled(),
    )
}

fn legend_square((x, y): (i32, i32)) -> Rectangle<(i32, i32)> {
    Rectangle::new([(x - 5, y - 5), (x + 5, y + 5)], BLUE.filled())
}
