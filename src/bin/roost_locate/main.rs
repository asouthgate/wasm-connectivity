//! Command-line entry point for bat roost location estimation.

use wasm_connect::roost::io::{aggregate_with_warnings, count_calls, read_detectors, read_sunset};
use wasm_connect::roost::compute_error_surface;

fn read_to_string(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}

const USAGE: &str = "\
roost-locate: estimate a bat roost location from call data

USAGE:
    roost-locate --detectors <detectors.csv> --master <master.csv> [OPTIONS]

OPTIONS:
    --detectors <path>       Path to *_detectors.csv (required)
    --master <path>          Path to *_master.csv (required)
    --filter-sunset <path>   Keep only calls within [sunset, sunset+t1/60],
                             using a date,sunset_time CSV (optional)
    --t0 <seconds>           Integration lower bound (default 0.01)
    --t1 <seconds>           Integration upper bound (default 5400); also sets
                             the --filter-sunset window to t1/60 minutes
    --diffusivity <m^2/s>    Diffusion coefficient D (default 81.7)
    --grid-size <n>          Grid points per axis (default 500)
    --output <path>          Write the full surface as x,y,loss CSV
    --help                   Show this help

METHOD:
    Henley et al. (2024), \"A simple and fast method for estimating bat roost
    locations\", Royal Society Open Science 11(4): 231999.
    https://doi.org/10.1098/rsos.231999
";

#[derive(Default)]
struct Args {
    detectors: Option<String>,
    master: Option<String>,
    filter_sunset: Option<String>,
    t0: f64,
    t1: f64,
    diffusivity: f64,
    grid_size: usize,
    output: Option<String>,
}

impl Args {
    fn defaults() -> Self {
        Args {
            t0: 0.01,
            t1: 5400.0,
            diffusivity: 81.7,
            grid_size: 500,
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
            "grid-size" => {
                let v = take_value(&inline, &mut it, &key)?;
                args.grid_size = v
                    .parse()
                    .map_err(|_| format!("invalid number for --grid-size: {v}"))?;
            }
            _ => return Err(format!("unknown option --{key}")),
        }
    }

    Ok(args)
}

fn run() -> Result<(), String> {
    let args = parse_args()?;

    if args.grid_size < 2 {
        return Err("grid-size must be >= 2".to_string());
    }
    if !(args.t1 > args.t0 && args.t0 > 0.0) {
        return Err("require 0 < t0 < t1".to_string());
    }

    let detectors_path = args.detectors.as_deref().ok_or("--detectors is required")?;
    let master_path = args.master.as_deref().ok_or("--master is required")?;

    let detectors = read_detectors(&read_to_string(detectors_path)?)?;

    let sunset = match &args.filter_sunset {
        Some(p) => Some(read_sunset(&read_to_string(p)?)?),
        None => None,
    };

    // Observation window tied to t1: sunset + t1/60 minutes.
    let counts = count_calls(
        &read_to_string(master_path)?,
        sunset.as_ref().map(|s| (s, args.t1 / 60.0)),
    )?;
    let (agg, warnings) = aggregate_with_warnings(&detectors, &counts);

    for w in &warnings {
        eprintln!("warning: {w}");
    }

    if agg.x.is_empty() {
        return Err("no detectors with calls found".to_string());
    }

    let mut wtr = match &args.output {
        Some(p) => {
            let mut w = csv::Writer::from_path(p).map_err(|e| e.to_string())?;
            w.write_record(["x", "y", "loss"]).map_err(|e| e.to_string())?;
            Some(w)
        }
        None => None,
    };

    let result = compute_error_surface(
        &agg.x,
        &agg.y,
        &agg.counts,
        args.grid_size,
        args.diffusivity,
        args.t0,
        args.t1,
        |x, y, loss| {
            if let Some(w) = wtr.as_mut() {
                let _ = w.write_record([x.to_string(), y.to_string(), loss.to_string()]);
            }
        },
    );

    if let Some(w) = wtr.as_mut() {
        w.flush().map_err(|e| e.to_string())?;
    }
    if let Some(p) = &args.output {
        eprintln!("Wrote surface to {p}");
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
