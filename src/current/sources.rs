use crate::real::{Real, scalar};

pub(super) fn valid_positive_value<F: Real>(value: F, nodata: F) -> bool {
    value.is_finite() && value > scalar::<F>(0.0) && (value - nodata).abs() > scalar::<F>(1e-10)
}

pub(super) fn build_node_values<F: Real>(
    cell_to_node: &[i32],
    num_nodes: usize,
    nrows: usize,
    ncols: usize,
    nodata: F,
    data: &[F],
) -> Vec<F> {
    let mut values = vec![scalar::<F>(0.0); num_nodes];
    for idx in 0..nrows * ncols {
        let node = cell_to_node[idx];
        if node > 0 && valid_positive_value(data[idx], nodata) {
            values[(node - 1) as usize] += data[idx];
        }
    }
    values
}

/// Balance injection only when requested and there are no physical grounds.
pub(super) fn balance_sources<F: Real>(s: &mut [F], remove_average: bool) {
    if remove_average {
        let sum: F = s.iter().copied().sum();
        if sum.abs() > scalar::<F>(1e-15) {
            let mean = sum / scalar::<F>(s.len() as f64);
            for value in s {
                *value -= mean;
            }
        }
    }
}
