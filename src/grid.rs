pub struct Grid {
    pub data: Vec<f64>,
    pub nrows: usize,
    pub ncols: usize,
    pub nodata: f64,
}

impl Grid {
    pub fn new(data: Vec<f64>, nrows: usize, ncols: usize, nodata: f64) -> Self {
        assert_eq!(data.len(), nrows * ncols, "data length must match dimensions");
        Grid { data, nrows, ncols, nodata }
    }

    pub fn get(&self, row: usize, col: usize) -> f64 {
        self.data[row * self.ncols + col]
    }

    pub fn set(&mut self, row: usize, col: usize, value: f64) {
        self.data[row * self.ncols + col] = value;
    }

    pub fn to_conductance(resistance_data: &[f64], nrows: usize, ncols: usize, nodata: f64) -> Self {
        let data: Vec<f64> = resistance_data.iter()
            .map(|&r| {
                if r == nodata || r <= 0.0 {
                    0.0
                } else {
                    1.0 / r
                }
            })
            .collect();
        Grid { data, nrows, ncols, nodata: 0.0 }
    }

    pub fn is_conductive(&self, row: usize, col: usize) -> bool {
        self.get(row, col) > 0.0
    }
}

pub fn build_nodemap(conductance: &Grid) -> (Vec<i32>, usize) {
    let mut nodemap = vec![0i32; conductance.nrows * conductance.ncols];
    let mut node_id = 1i32;
    for row in 0..conductance.nrows {
        for col in 0..conductance.ncols {
            if conductance.is_conductive(row, col) {
                nodemap[row * conductance.ncols + col] = node_id;
                node_id += 1;
            }
        }
    }
    let num_nodes = (node_id - 1) as usize;
    (nodemap, num_nodes)
}

pub fn extract_focal_points(
    point_data: &[i32],
    nrows: usize,
    ncols: usize,
    nodemap: &[i32],
) -> Vec<(i32, usize)> {
    let mut points: Vec<(i32, usize)> = Vec::new();
    for row in 0..nrows {
        for col in 0..ncols {
            let pid = point_data[row * ncols + col];
            if pid > 0 {
                let node = nodemap[row * ncols + col];
                if node > 0 {
                    points.push((pid, (node - 1) as usize));
                }
            }
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
        let cond = Grid::to_conductance(&[2.0, 4.0, 0.0, -9999.0], 2, 2, -9999.0);
        assert_eq!(cond.get(0, 0), 0.5);
        assert_eq!(cond.get(0, 1), 0.25);
        assert_eq!(cond.get(1, 0), 0.0);
        assert_eq!(cond.get(1, 1), 0.0);
    }

    #[test]
    fn test_build_nodemap() {
        let cond = Grid::to_conductance(&[1.0, 0.0, 1.0, 0.0], 2, 2, 0.0);
        let (nodemap, num_nodes) = build_nodemap(&cond);
        assert_eq!(num_nodes, 2);
        assert_eq!(nodemap, vec![1, 0, 2, 0]);
    }

    #[test]
    fn test_extract_focal_points() {
        let cond = Grid::to_conductance(&vec![1.0; 9], 3, 3, -9999.0);
        let (nodemap, _) = build_nodemap(&cond);
        let mut point_data = vec![0i32; 9];
        point_data[0] = 2;
        point_data[4] = 1;
        let points = extract_focal_points(&point_data, 3, 3, &nodemap);
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].0, 1);
        assert_eq!(points[1].0, 2);
    }
}
