//! CSV reading, sunset filtering, and per-detector aggregation for roost
//! location estimation.
//!
//! This is the single source of truth for parsing the roost-finder input CSVs.
//! It is part of the library (not the `bin` feature) so that both the WASM
//! build and the `roost-locate` CLI share the exact same parsing + aggregation
//! logic.
//!
//! # CSV contract
//!
//! This parser enforces a strict column contract. Unrecognised columns are
//! ignored (the raw survey files carry dozens of extra habitat/acoustic
//! columns), but the required columns must exist with exact (trimmed,
//! BOM-stripped) names:
//!
//! * **detectors**: one row per detector:
//!   - `detector`      (string, required)
//!   - `x`             (number, required): British National Grid easting
//!   - `y`             (number, required): British National Grid northing
//!   - `n_active_days` (number, optional; blank → `None`)
//!
//! * **master**: one row per recorded call:
//!   - `detector`      (string, required)
//!   - `date`          (`dd/mm/yyyy`, required only when a sunset table is used)
//!   - `time`          (`HH:MM:SS`, required only when a sunset table is used)
//!
//! * **sunset**: one row per survey date:
//!   - `date`         (`dd/mm/yyyy`, required)
//!   - `sunset_time`  (`HH:MM:SS`, required: parsed to a fraction of a day)

use std::collections::HashMap;

/// A detector row from the detectors CSV.
#[derive(Debug, Clone)]
pub struct Detector {
    pub x: f64,
    pub y: f64,
    /// Active nights (`n_active_days`); `None` if blank.
    pub days: Option<f64>,
}

/// Aggregated per-detector call data (detectors with zero calls are absent).
#[derive(Debug, Clone, PartialEq)]
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

/// Read detectors CSV (as text), keyed by detector id.
pub fn read_detectors(text: &str) -> Result<HashMap<String, Detector>, String> {
    let mut rdr = csv::Reader::from_reader(text.as_bytes());
    let headers = rdr.headers().map_err(|e| e.to_string())?.clone();

    let id_idx = header_index(&headers, "detector")
        .ok_or("missing 'detector' column in detectors file")?;
    let x_idx = header_index(&headers, "x").ok_or("missing 'x' column in detectors file")?;
    let y_idx = header_index(&headers, "y").ok_or("missing 'y' column in detectors file")?;
    let days_idx = header_index(&headers, "n_active_days");

    let mut map = HashMap::new();
    for (row_no, rec) in rdr.records().enumerate() {
        let rec = rec.map_err(|e| format!("detectors row {}: {e}", row_no + 2))?;
        let id = rec.get(id_idx).unwrap_or("").trim().to_string();
        if id.is_empty() {
            continue;
        }
        let x = parse_f64(rec.get(x_idx).unwrap_or(""))
            .map_err(|e| format!("detector {id}: invalid X ({e})"))?;
        let y = parse_f64(rec.get(y_idx).unwrap_or(""))
            .map_err(|e| format!("detector {id}: invalid Y ({e})"))?;
        let days = days_idx.and_then(|i| parse_opt_f64(rec.get(i).unwrap_or("")));
        if let Some(prev) = map.insert(id.clone(), Detector { x, y, days }) {
            let _ = prev;
            return Err(format!("duplicate detector id {id:?}"));
        }
    }
    Ok(map)
}

/// Parse an `HH:MM:SS` time into a fraction of a day.
fn parse_time_fraction(s: &str) -> Option<f64> {
    let mut parts = s.trim().split(':');
    let h = parts.next()?.parse::<f64>().ok()?;
    let m = parts.next()?.parse::<f64>().ok()?;
    let sec = parts.next()?.parse::<f64>().ok()?;
    Some((h * 3600.0 + m * 60.0 + sec) / 86400.0)
}

type DateKey = (u32, u32, u32);

fn parse_date(s: &str) -> Option<DateKey> {
    let mut parts = s.trim().split('/');
    let d = parts.next()?.parse::<u32>().ok()?;
    let m = parts.next()?.parse::<u32>().ok()?;
    let y = parts.next()?.parse::<u32>().ok()?;
    Some((d, m, y))
}

/// Read the sunset CSV (as text) into `{date - tuple: sunset_fraction}`.
pub fn read_sunset(text: &str) -> Result<HashMap<DateKey, f64>, String> {
    let mut rdr = csv::Reader::from_reader(text.as_bytes());
    let headers = rdr.headers().map_err(|e| e.to_string())?.clone();

    let date_idx = header_index(&headers, "date").ok_or("missing 'date' column in sunset file")?;
    let sunset_idx =
        header_index(&headers, "sunset_time").ok_or("missing 'sunset_time' column in sunset file")?;

    let mut map = HashMap::new();
    for (row_no, rec) in rdr.records().enumerate() {
        let rec = rec.map_err(|e| format!("sunset row {}: {e}", row_no + 2))?;
        let date = parse_date(rec.get(date_idx).unwrap_or(""))
            .ok_or_else(|| format!("sunset row {}: invalid date", row_no + 2))?;
        let sunset = parse_time_fraction(rec.get(sunset_idx).unwrap_or(""))
            .ok_or_else(|| format!("sunset row {}: invalid sunset_time (expected HH:MM:SS)", row_no + 2))?;
        map.insert(date, sunset);
    }
    Ok(map)
}

/// Count calls per detector from the master CSV (as text).
///
/// If `sunset` is `Some`, rows whose `time` falls within
/// `[sunset, sunset + minutes_after_sunset]` for their `date` are kept (rows
/// whose date is absent from the table are dropped). Otherwise every row is
/// counted.
pub fn count_calls(
    text: &str,
    sunset: Option<(&HashMap<DateKey, f64>, f64)>,
) -> Result<HashMap<String, u64>, String> {
    let mut rdr = csv::Reader::from_reader(text.as_bytes());
    let headers = rdr.headers().map_err(|e| e.to_string())?.clone();

    let det_idx =
        header_index(&headers, "detector").ok_or("missing 'detector' column in master file")?;
    let date_idx = header_index(&headers, "date");
    let time_idx = header_index(&headers, "time");

    if sunset.is_some() && (date_idx.is_none() || time_idx.is_none()) {
        return Err("filtering by sunset requires 'date' and 'time' columns".to_string());
    }

    let mut counts: HashMap<String, u64> = HashMap::new();
    for (row_no, rec) in rdr.records().enumerate() {
        let rec = rec.map_err(|e| format!("master row {}: {e}", row_no + 2))?;
        let id = rec.get(det_idx).unwrap_or("").trim().to_string();
        if id.is_empty() {
            continue;
        }

        let keep = match sunset {
            None => true,
            Some((map, minutes)) => {
                let date = date_idx.and_then(|i| parse_date(rec.get(i).unwrap_or("")));
                let time = time_idx.and_then(|i| parse_time_fraction(rec.get(i).unwrap_or("")));
                match (date, time) {
                    (Some(d), Some(t)) => match map.get(&d) {
                        Some(&s) => t >= s && t <= s + minutes / 1440.0,
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

/// Combine raw per-detector counts with detector coordinates and active
/// nights, returning `None` for a detector that needs to be skipped (with a
/// `Some(warning)` describing why).
fn aggregate_row(
    det: &Detector,
    n: f64,
    per_night: bool,
) -> Result<f64, String> {
    if per_night {
        match det.days {
            Some(d) if d > 0.0 => Ok(n / d),
            _ => Err("missing/zero 'n_active_days'".to_string()),
        }
    } else {
        Ok(n)
    }
}

/// Aggregate per-detector counts with coordinates, returning parallel arrays
/// and a list of non-fatal warnings.
pub fn aggregate_with_warnings(
    detectors: &HashMap<String, Detector>,
    counts: &HashMap<String, u64>,
    per_night: bool,
) -> (Aggregated, Vec<String>) {
    let mut ids: Vec<&String> = counts.keys().collect();
    ids.sort();

    let mut x = Vec::new();
    let mut y = Vec::new();
    let mut c = Vec::new();
    let mut warnings = Vec::new();

    for id in ids {
        let n = counts[id] as f64;
        let Some(det) = detectors.get(id) else {
            warnings.push(format!("detector {id} has calls but no detector entry; skipping"));
            continue;
        };
        match aggregate_row(det, n, per_night) {
            Ok(v) => {
                x.push(det.x);
                y.push(det.y);
                c.push(v);
            }
            Err(reason) => {
                warnings.push(format!("detector {id} has calls but {reason}; skipping"));
            }
        }
    }

    (Aggregated { x, y, counts: c }, warnings)
}

/// Aggregate without capturing warnings (backwards-compatible helper).
pub fn aggregate(
    detectors: &HashMap<String, Detector>,
    counts: &HashMap<String, u64>,
    per_night: bool,
) -> Aggregated {
    aggregate_with_warnings(detectors, counts, per_night).0
}

#[cfg(test)]
mod tests {
    use super::*;

    const DETECTORS: &str = "\
detector,x,y,n_active_days
S1,274530,66084,7
B2,274282,66079,2
C1,272857,65301,2
";

    const MASTER: &str = "\
detector,date,time
S1,26/06/2016,21:00:00
S1,26/06/2016,21:05:00
B2,26/06/2016,22:00:00
C1,26/06/2016,23:00:00
";

    const SUNSET: &str = "\
date,sunset_time
26/06/2016,21:00:00
";

    #[test]
    fn parses_contract_headers_case_sensitively() {
        let det = read_detectors(DETECTORS).unwrap();
        assert_eq!(det.len(), 3);
        let s1 = &det["S1"];
        assert_eq!(s1.x, 274530.0);
        assert_eq!(s1.y, 66084.0);
        assert_eq!(s1.days, Some(7.0));
    }

    #[test]
    fn rejects_missing_required_column() {
        let err = read_detectors("detector,y,n_active_days\nS1,2,7").unwrap_err();
        assert!(err.contains("'x'"), "expected missing-X error, got: {err}");
    }

    #[test]
    fn ignores_unrecognised_columns() {
        let det = read_detectors("detector,Site note,x,y,n_active_days\nS1,foo,274530,66084,7").unwrap();
        assert_eq!(det["S1"].x, 274530.0);
    }

    #[test]
    fn rejects_duplicate_detector_id() {
        let err = read_detectors("detector,x,y,n_active_days\nS1,1,2,7\nS1,3,4,7").unwrap_err();
        assert!(err.contains("duplicate"));
    }

    #[test]
    fn counts_calls_without_sunset() {
        let detectors = read_detectors(DETECTORS).unwrap();
        let counts = count_calls(MASTER, None).unwrap();
        assert_eq!(counts["S1"], 2);
        assert_eq!(counts["B2"], 1);
        assert_eq!(counts["C1"], 1);
        let agg = aggregate(&detectors, &counts, false);
        assert_eq!(agg.x.len(), 3);
    }

    #[test]
    fn filters_by_sunset_window() {
        let detectors = read_detectors(DETECTORS).unwrap();
        let sunset = read_sunset(SUNSET).unwrap();
        let counts = count_calls(MASTER, Some((&sunset, 90.0))).unwrap();
        assert_eq!(counts["S1"], 2); // within [21:00, 22:30]
        assert_eq!(counts["B2"], 1); // 22:00 within window
        assert_eq!(counts.get("C1"), None); // 23:00 outside window
        let agg = aggregate(&detectors, &counts, false);
        assert_eq!(agg.counts, vec![1.0, 2.0]);
    }

    #[test]
    fn per_night_aggregation_skips_zero_days() {
        let detectors = read_detectors(DETECTORS).unwrap();
        let counts: HashMap<String, u64> = [("S1".to_string(), 14), ("B2".to_string(), 4)].into();
        let (agg, warnings) = aggregate_with_warnings(&detectors, &counts, true);
        // S1: 14/7 = 2.0, B2: 4/2 = 2.0
        assert_eq!(agg.counts, vec![2.0, 2.0]);
        assert!(warnings.is_empty());
    }
}
