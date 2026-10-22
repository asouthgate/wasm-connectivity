use crate::linalg::real::{Real, scalar};
pub struct Grid<F: Real = f64> {
    pub data: Vec<F>,
    pub nrows: usize,
    pub ncols: usize,
}

impl<F: Real> Grid<F> {
    pub fn get(&self, row: usize, col: usize) -> F {
        self.data[row * self.ncols + col]
    }

    pub fn to_conductance(resistance_data: &[F], nrows: usize, ncols: usize, nodata: F) -> Self {
        let data: Vec<F> = resistance_data.iter()
            .map(|&r| {
                if !r.is_finite() || r == nodata || r <= scalar::<F>(0.0) {
                    scalar::<F>(0.0)
                } else {
                    scalar::<F>(1.0) / r
                }
            })
            .collect();
        Grid::<F> { data, nrows, ncols }
    }

    pub fn is_conductive(&self, row: usize, col: usize) -> bool {
        self.get(row, col) > scalar::<F>(0.0)
    }
}

pub fn build_cell_to_node<F: Real>(conductance: &Grid<F>) -> (Vec<i32>, usize) {
    let mut cell_to_node = vec![0i32; conductance.nrows * conductance.ncols];
    let mut node_id = 1i32;
    for row in 0..conductance.nrows {
        for col in 0..conductance.ncols {
            if conductance.is_conductive(row, col) {
                cell_to_node[row * conductance.ncols + col] = node_id;
                node_id += 1;
            }
        }
    }
    let num_nodes = (node_id - 1) as usize;
    (cell_to_node, num_nodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_conductance() {
        let cond = Grid::to_conductance(&[2.0, 4.0, 0.0, crate::NODATA_SENTINEL], 2, 2, crate::NODATA_SENTINEL);
        assert_eq!(cond.get(0, 0), 0.5);
        assert_eq!(cond.get(0, 1), 0.25);
        assert_eq!(cond.get(1, 0), 0.0);
        assert_eq!(cond.get(1, 1), 0.0);
    }

    #[test]
    fn test_build_cell_to_node() {
        let cond = Grid::to_conductance(&[1.0, 0.0, 1.0, 0.0], 2, 2, 0.0);
        let (cell_to_node, num_nodes) = build_cell_to_node(&cond);
        assert_eq!(num_nodes, 2);
        assert_eq!(cell_to_node, vec![1, 0, 2, 0]);
    }

}
