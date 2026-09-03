//! CSV reading, sunset filtering, and per-detector aggregation.

use std::collections::HashMap;

/// A detector row from `*_detectors.csv`.
#[derive(Debug, Clone)]
pub struct Detector {
    pub x: f64,
    pub y: f64,
    /// Active nights (`Number of days`); `None` if blank.
    pub days: Option<f64>,
}

/// Aggregated per-detector call data (detectors with zero calls are absent).
#[derive(Debug, Clone)]
pub struct Aggregated {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub counts: Vec<f64>,
}

fn clean_header(h: &str) -> String {
    h.trim().trim_start_matches('\u{feff}').to_string()
}

fn header_index(headers: &csv::StringRecord, name: &str) -> Option<usize> {
    headers.iter().position(|h| clean_header(h) == name)
}

fn parse_f64(s: &str) -> Result<f64, String> {
    let t = s.trim();
    t.parse::<f64>().map_err(|_| format!("expected a number, got {t:?}"))
}

fn parse_opt_f64(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        t.parse::<f64>().ok()
    }
}

/// Read `*_detectors.csv`, keyed by detector number.
pub fn read_detectors(path: &str) -> Result<HashMap<String, Detector>, String> {
    let mut rdr = csv::Reader::from_path(path).map_err(|e| e.to_string())?;
    let headers = rdr.headers().map_err(|e| e.to_string())?.clone();

    let id_i = header_index(&headers, "Detector number").ok_or("missing 'Detector number' column")?;
    let x_i = header_index(&headers, "X coordinate").ok_or("missing 'X coordinate' column")?;
    let y_i = header_index(&headers, "Y coordinate").ok_or("missing 'Y coordinate' column")?;
    let days_i = header_index(&headers, "Number of days").ok_or("missing 'Number of days' column")?;

    let mut map = HashMap::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| e.to_string())?;
        let id = rec.get(id_i).unwrap_or("").trim().to_string();
        let x = parse_f64(rec.get(x_i).unwrap_or(""))?;
        let y = parse_f64(rec.get(y_i).unwrap_or(""))?;
        let days = parse_opt_f64(rec.get(days_i).unwrap_or(""));
        map.insert(id, Detector { x, y, days });
    }
    Ok(map)
}

/// Parse a `dd/mm/yyyy` date into `(day, month, year)`.
fn parse_date(s: &str) -> Option<(u32, u32, u32)> {
    let mut parts = s.trim().split('/');
    let d = parts.next()?.parse::<u32>().ok()?;
    let m = parts.next()?.parse::<u32>().ok()?;
    let y = parts.next()?.parse::<u32>().ok()?;
    Some((d, m, y))
}

/// Parse an `HH:MM:SS` time into a fraction of a day.
fn parse_time_fraction(s: &str) -> Option<f64> {
    let mut parts = s.trim().split(':');
    let h = parts.next()?.parse::<f64>().ok()?;
    let m = parts.next()?.parse::<f64>().ok()?;
    let sec = parts.next()?.parse::<f64>().ok()?;
    Some((h * 3600.0 + m * 60.0 + sec) / 86400.0)
}

/// Read `Sunrise_sunset.csv` into `{(day, month, year): sunset_fraction}`.
pub fn read_sunset(path: &str) -> Result<HashMap<(u32, u32, u32), f64>, String> {
    let mut rdr = csv::Reader::from_path(path).map_err(|e| e.to_string())?;
    let headers = rdr.headers().map_err(|e| e.to_string())?.clone();

    let date_i = header_index(&headers, "Date").ok_or("missing 'Date' column")?;
    let sunset_i = header_index(&headers, "Sunset").ok_or("missing 'Sunset' column")?;

    let mut map = HashMap::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| e.to_string())?;
        let date = parse_date(rec.get(date_i).unwrap_or(""))
            .ok_or("invalid date in sunset file")?;
        let sunset = parse_f64(rec.get(sunset_i).unwrap_or(""))?;
        map.insert(date, sunset);
    }
    Ok(map)
}

/// Count calls per detector from `*_master.csv`.
///
/// If `sunset` is `Some`, only rows whose `TIME` falls within
/// `[sunset, sunset + 90min]` for their `DATE` are kept (rows whose date is
/// absent from the table are dropped). Otherwise every row is counted.
pub fn count_calls(
    master_path: &str,
    sunset: Option<&HashMap<(u32, u32, u32), f64>>,
) -> Result<HashMap<String, u64>, String> {
    let mut rdr = csv::Reader::from_path(master_path).map_err(|e| e.to_string())?;
    let headers = rdr.headers().map_err(|e| e.to_string())?.clone();

    let det_i = header_index(&headers, "Detector number").ok_or("missing 'Detector number' column")?;
    let date_i = header_index(&headers, "DATE");
    let time_i = header_index(&headers, "TIME");

    if sunset.is_some() && (date_i.is_none() || time_i.is_none()) {
        return Err("filtering by sunset requires 'DATE' and 'TIME' columns".to_string());
    }

    let mut counts: HashMap<String, u64> = HashMap::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| e.to_string())?;
        let id = rec.get(det_i).unwrap_or("").trim().to_string();
        if id.is_empty() {
            continue;
        }

        let keep = match sunset {
            None => true,
            Some(map) => {
                let date = date_i.and_then(|i| parse_date(rec.get(i).unwrap_or("")));
                let time = time_i.and_then(|i| parse_time_fraction(rec.get(i).unwrap_or("")));
                match (date, time) {
                    (Some(d), Some(t)) => match map.get(&d) {
                        Some(&s) => t >= s && t <= s + 90.0 / 1440.0,
                        None => false,
                    },
                    _ => false,
                }
            }
        };

        if keep {
            *counts.entry(id).or_insert(0) += 1;
        }
    }
    Ok(counts)
}

/// Combine raw per-detector counts with detector coordinates and active nights.
///
/// With `per_night` (the paper's definition), each count is divided by the
/// detector's number of active nights. Detectors present in `counts` but
/// missing from `detectors` (or lacking a positive day count) are skipped with
/// a warning.
pub fn aggregate(
    detectors: &HashMap<String, Detector>,
    counts: &HashMap<String, u64>,
    per_night: bool,
) -> Aggregated {
    let mut ids: Vec<&String> = counts.keys().collect();
    ids.sort();

    let mut x = Vec::new();
    let mut y = Vec::new();
    let mut c = Vec::new();

    for id in ids {
        let n = counts[id] as f64;
        let Some(det) = detectors.get(id) else {
            eprintln!("warning: detector {id} has calls but no detector entry; skipping");
            continue;
        };

        if per_night {
            match det.days {
                Some(d) if d > 0.0 => c.push(n / d),
                _ => {
                    eprintln!(
                        "warning: detector {id} has calls but missing/zero 'Number of days'; skipping"
                    );
                    continue;
                }
            }
        } else {
            c.push(n);
        }
        x.push(det.x);
        y.push(det.y);
    }

    Aggregated { x, y, counts: c }
}
