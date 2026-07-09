pub struct Grid {
    pub data: Vec<f64>,
    pub nrows: usize,
    pub ncols: usize,
}

impl Grid {
    pub fn get(&self, row: usize, col: usize) -> f64 {
        self.data[row * self.ncols + col]
    }

    pub fn to_conductance(resistance_data: &[f64], nrows: usize, ncols: usize, nodata: f64) -> Self {
        let data: Vec<f64> = resistance_data.iter()
            .map(|&r| {
                if !r.is_finite() || r == nodata || r <= 0.0 {
                    0.0
                } else {
                    1.0 / r
                }
            })
            .collect();
        Grid { data, nrows, ncols }
    }

    pub fn is_conductive(&self, row: usize, col: usize) -> bool {
        self.get(row, col) > 0.0
    }
}

pub fn build_cell_to_node(conductance: &Grid) -> (Vec<i32>, usize) {
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

pub fn extract_focal_points(
    point_data: &[i32],
    nrows: usize,
    ncols: usize,
    cell_to_node: &[i32],
) -> Vec<(i32, usize)> {
    let expected_len = nrows * ncols;
    assert!(point_data.len() >= expected_len, "point_data slice is too small ({} < {})", point_data.len(), expected_len);
    assert!(cell_to_node.len() >= expected_len, "cell_to_node slice is too small ({} < {})", cell_to_node.len(), expected_len);

    let mut points: Vec<(i32, usize)> = Vec::new();

    for (&pid, &node) in point_data[..expected_len].iter().zip(&cell_to_node[..expected_len]) {
        if pid > 0 && node > 0 {
            points.push((pid, (node - 1) as usize));
        }
    }

    points.sort_by_key(|(pid, _)| *pid);
    points.dedup_by_key(|(pid, _)| *pid);
    points
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

    #[test]
    fn test_extract_focal_points() {
        let cond = Grid::to_conductance(&[1.0; 9], 3, 3, crate::NODATA_SENTINEL);
        let (cell_to_node, _) = build_cell_to_node(&cond);
        let mut point_data = vec![0i32; 9];
        point_data[0] = 2;
        point_data[4] = 1;
        let points = extract_focal_points(&point_data, 3, 3, &cell_to_node);
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].0, 1);
        assert_eq!(points[1].0, 2);
    }
}
