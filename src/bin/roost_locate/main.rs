//! Command-line entry point for bat roost location estimation.

mod io;
mod plot;

use io::{aggregate, count_calls, read_detectors, read_sunset};
use plot::{render as render_plot, PlotData};
use wasm_connect::roost::compute_error_surface;

const USAGE: &str = "\
roost-locate — estimate a bat roost location from call data

USAGE:
    roost-locate --detectors <detectors.csv> --master <master.csv> [OPTIONS]

OPTIONS:
    --detectors <path>       Path to *_detectors.csv (required)
    --master <path>          Path to *_master.csv (required)
    --filter-sunset <path>   Keep only calls within [sunset, sunset+90min],
                             using a Date,Sunset CSV (optional)
    --t0 <seconds>           Integration lower bound (default 0.01)
    --t1 <seconds>           Integration upper bound (default 5400)
    --diffusivity <m^2/s>    Diffusion coefficient D (default 81.7)
    --capture-radius <m>     Detector capture radius r (default 15)
    --grid-size <n>          Grid points per axis (default 500)
    --loss <l2|l1>           Loss metric (default l2)
    --raw-counts             Use raw counts instead of per-night averages
    --output <path>          Write the full surface as x,y,loss CSV
    --plot <path.png>        Render the surface to a PNG image
    --roost <x> <y>          Known roost coordinates to mark on the plot
    --help                   Show this help
";

#[derive(Default)]
struct Args {
    detectors: Option<String>,
    master: Option<String>,
    filter_sunset: Option<String>,
    t0: f64,
    t1: f64,
    diffusivity: f64,
    capture_radius: f64,
    grid_size: usize,
    loss: String,
    raw_counts: bool,
    output: Option<String>,
    plot: Option<String>,
    roost: Option<(f64, f64)>,
}

impl Args {
    fn defaults() -> Self {
        Args {
            t0: 0.01,
            t1: 5400.0,
            diffusivity: 81.7,
            capture_radius: 15.0,
            grid_size: 500,
            loss: "l2".to_string(),
            ..Default::default()
        }
    }
}

fn take_value(
    inline: &Option<String>,
    it: &mut impl Iterator<Item = String>,
    key: &str,
) -> Result<String, String> {
    if let Some(v) = inline {
        Ok(v.clone())
    } else {
        it.next().ok_or_else(|| format!("missing value for --{key}"))
    }
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args::defaults();
    let mut it = std::env::args().skip(1);

    while let Some(arg) = it.next() {
        if arg == "--help" || arg == "-h" {
            print!("{USAGE}");
            std::process::exit(0);
        }
        let body = arg
            .strip_prefix("--")
            .ok_or_else(|| format!("unexpected argument {arg:?}"))?;
        let (key, inline) = match body.split_once('=') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (body.to_string(), None),
        };

        match key.as_str() {
            "detectors" => args.detectors = Some(take_value(&inline, &mut it, &key)?),
            "master" => args.master = Some(take_value(&inline, &mut it, &key)?),
            "filter-sunset" => args.filter_sunset = Some(take_value(&inline, &mut it, &key)?),
            "output" => args.output = Some(take_value(&inline, &mut it, &key)?),
            "plot" => args.plot = Some(take_value(&inline, &mut it, &key)?),
            "t0" => {
                let v = take_value(&inline, &mut it, &key)?;
                args.t0 = v.parse().map_err(|_| format!("invalid number for --t0: {v}"))?;
            }
            "t1" => {
                let v = take_value(&inline, &mut it, &key)?;
                args.t1 = v.parse().map_err(|_| format!("invalid number for --t1: {v}"))?;
            }
            "diffusivity" => {
                let v = take_value(&inline, &mut it, &key)?;
                args.diffusivity = v
                    .parse()
                    .map_err(|_| format!("invalid number for --diffusivity: {v}"))?;
            }
            "capture-radius" => {
                let v = take_value(&inline, &mut it, &key)?;
                args.capture_radius = v
                    .parse()
                    .map_err(|_| format!("invalid number for --capture-radius: {v}"))?;
            }
            "grid-size" => {
                let v = take_value(&inline, &mut it, &key)?;
                args.grid_size = v
                    .parse()
                    .map_err(|_| format!("invalid number for --grid-size: {v}"))?;
            }
            "loss" => args.loss = take_value(&inline, &mut it, &key)?,
            "raw-counts" => {
                if inline.is_some() {
                    return Err("--raw-counts takes no value".to_string());
                }
                args.raw_counts = true;
            }
            "roost" => {
                let x = take_value(&inline, &mut it, &key)?;
                let y = it
                    .next()
                    .ok_or_else(|| format!("missing second value for --{key}"))?;
                let x = x.parse().map_err(|_| format!("invalid --roost x: {x}"))?;
                let y = y.parse().map_err(|_| format!("invalid --roost y: {y}"))?;
                args.roost = Some((x, y));
            }
            _ => return Err(format!("unknown option --{key}")),
        }
    }

    Ok(args)
}

fn run() -> Result<(), String> {
    let args = parse_args()?;

    if args.loss != "l2" && args.loss != "l1" {
        return Err(format!("invalid loss {:?}, expected l2 or l1", args.loss));
    }
    if args.grid_size < 2 {
        return Err("grid-size must be >= 2".to_string());
    }
    if !(args.t1 > args.t0 && args.t0 > 0.0) {
        return Err("require 0 < t0 < t1".to_string());
    }

    let detectors_path = args.detectors.as_deref().ok_or("--detectors is required")?;
    let master_path = args.master.as_deref().ok_or("--master is required")?;

    let detectors = read_detectors(detectors_path)?;

    let sunset = match &args.filter_sunset {
        Some(p) => Some(read_sunset(p)?),
        None => None,
    };

    let counts = count_calls(master_path, sunset.as_ref())?;
    let agg = aggregate(&detectors, &counts, !args.raw_counts);

    if agg.x.is_empty() {
        return Err("no detectors with calls found".to_string());
    }

    // Grid extents (the search grid spans the detector bounding box).
    let xmin = agg.x.iter().cloned().fold(f64::INFINITY, f64::min);
    let xmax = agg.x.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let ymin = agg.y.iter().cloned().fold(f64::INFINITY, f64::min);
    let ymax = agg.y.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    let mut wtr = match &args.output {
        Some(p) => {
            let mut w = csv::Writer::from_path(p).map_err(|e| e.to_string())?;
            w.write_record(["x", "y", "loss"]).map_err(|e| e.to_string())?;
            Some(w)
        }
        None => None,
    };

    let want_surface = args.plot.is_some();
    let mut surface = Vec::with_capacity(if want_surface {
        args.grid_size * args.grid_size
    } else {
        0
    });

    let result = compute_error_surface(
        &agg.x,
        &agg.y,
        &agg.counts,
        args.grid_size,
        args.capture_radius,
        args.diffusivity,
        args.t0,
        args.t1,
        &args.loss,
        |x, y, loss| {
            if let Some(w) = wtr.as_mut() {
                let _ = w.write_record([x.to_string(), y.to_string(), loss.to_string()]);
            }
            if want_surface {
                surface.push(loss);
            }
        },
    );

    if let Some(w) = wtr.as_mut() {
        w.flush().map_err(|e| e.to_string())?;
    }
    if let Some(p) = &args.output {
        eprintln!("Wrote surface to {p}");
    }

    // Weighted mean of detector positions by their (averaged) call counts.
    let total: f64 = agg.counts.iter().sum();
    let wmx = agg
        .x
        .iter()
        .zip(agg.counts.iter())
        .map(|(x, c)| x * c)
        .sum::<f64>()
        / total;
    let wmy = agg
        .y
        .iter()
        .zip(agg.counts.iter())
        .map(|(y, c)| y * c)
        .sum::<f64>()
        / total;

    if let Some(plot_path) = &args.plot {
        let data = PlotData {
            surface: &surface,
            grid_size: args.grid_size,
            xmin,
            xmax,
            ymin,
            ymax,
            detectors_x: &agg.x,
            detectors_y: &agg.y,
            counts: &agg.counts,
            predicted: (result.x, result.y),
            weighted_mean: (wmx, wmy),
            known_roost: args.roost,
            loss: result.loss,
        };
        render_plot(plot_path, &data)?;
        eprintln!("Wrote plot to {plot_path}");
    }

    println!(
        "Predicted roost: ({:.1}, {:.1})  loss={:.9}",
        result.x, result.y, result.loss
    );

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        eprintln!("run with --help for usage");
        std::process::exit(2);
    }
}
