//! Perceptually-ramped heatmap colormap for surface rendering.

/// Blue to yellow
/// Anchor stops of the colormap: `[r, g, b]` in 0..1, evenly spaced in value.
const STOPS: [[f64; 3]; 9] = [
    [0.14, 0.10, 0.46],
    [0.18, 0.22, 0.62],
    [0.12, 0.40, 0.71],
    [0.05, 0.57, 0.66],
    [0.08, 0.70, 0.48],
    [0.30, 0.80, 0.30],
    [0.63, 0.85, 0.23],
    [0.88, 0.84, 0.28],
    [0.98, 0.88, 0.42],
];

fn interpolate(stops: &[[f64; 3]], n: usize) -> Vec<[f64; 3]> {
    debug_assert!(!stops.is_empty());
    if n == 1 {
        return vec![stops[0]];
    }
    let last = stops.len() - 1;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let x = (i as f64) / ((n - 1) as f64) * (last as f64);
        let idx = (x.floor() as usize).min(last.saturating_sub(1));
        let frac = x - idx as f64;
        let a = stops[idx];
        let b = stops[idx + 1];
        out.push([
            a[0] + (b[0] - a[0]) * frac,
            a[1] + (b[1] - a[1]) * frac,
            a[2] + (b[2] - a[2]) * frac,
        ]);
    }
    out
}

/// Return an `n`-entry colormap as `[r, g, b]` triplets in 0..1, from the low
/// end (dark blue) to the high end (warm yellow).
pub fn colormap(n: usize) -> Vec<[f64; 3]> {
    interpolate(&STOPS, n)
}

/// Return an `n`-entry colormap as 8-bit `[r, g, b]` triplets.
pub fn colormap_u8(n: usize) -> Vec<[u8; 3]> {
    colormap(n)
        .into_iter()
        .map(|c| {
            [
                (c[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                (c[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                (c[2].clamp(0.0, 1.0) * 255.0).round() as u8,
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::colormap;

    #[test]
    fn endpoints() {
        let p = colormap(256);
        assert_eq!(p.len(), 256);
        assert!((p[0][0] - 0.14).abs() < 1e-12);
        assert!((p[255][2] - 0.42).abs() < 1e-12);
        assert!(p[255][0] > 0.9 && p[255][1] > 0.8 && p[255][2] < 0.6);
    }

    #[test]
    fn single_entry() {
        let p = colormap(1);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0], [0.14, 0.10, 0.46]);
    }
}
