use super::distance::distance_transform_with_buffer;

const LIDAR_BUFFER_CELLS: f64 = 10.0;

pub struct SurfaceOutput {
    pub surf: Vec<f64>,
    pub soft_surf: Vec<f64>,
    pub hard_surf: Vec<f64>,
}

pub fn calc_surfs(dtm: &[f64], dsm: &[f64], buildings: &[f64], nrows: usize, ncols: usize) -> SurfaceOutput {
    let total = nrows * ncols;
    let mut surf = vec![0.0f64; total];
    let mut soft_surf = vec![0.0f64; total];
    let mut hard_surf = vec![0.0f64; total];

    for i in 0..total {
        let dtm_v = if dtm[i].is_finite() { dtm[i] } else { 0.0 };
        let dsm_v = if dsm[i].is_finite() { dsm[i] } else { 0.0 };
        let sv = dsm_v - dtm_v;
        surf[i] = sv;
        soft_surf[i] = sv;
    }

    for i in 0..total {
        if buildings[i].is_finite() && buildings[i] > 0.0 {
            soft_surf[i] = 0.0;
            hard_surf[i] = if buildings[i] > 1.0 {
                // Drawn building with explicit height in metres
                buildings[i]
            } else {
                // Server building mask (value 1.0): height comes from DSM-DTM
                surf[i]
            };
        }
    }

    SurfaceOutput {
        surf,
        soft_surf,
        hard_surf,
    }
}

pub struct LidarOutput {
    pub manhedge: Vec<f64>,
    pub unmanhedge: Vec<f64>,
    pub tree: Vec<f64>,
    pub distance_rasters: Vec<(Vec<f64>, f64)>,
}

pub fn prep_lidar_rasters(soft_surf: &[f64], nrows: usize, ncols: usize) -> LidarOutput {
    let total = nrows * ncols;

    let mut manhedge = vec![0.0f64; total];
    let mut unmanhedge = vec![0.0f64; total];
    let mut tree = vec![0.0f64; total];

    for i in 0..total {
        let h = soft_surf[i];
        if !h.is_finite() {
            continue;
        }
        if h > 1.0 && h < 3.0 {
            manhedge[i] = 1.0;
        }
        if h > 3.0 && h < 6.0 {
            unmanhedge[i] = 1.0;
        }
        if h >= 6.0 {
            tree[i] = 1.0;
        }
    }

    let mh_has_na = manhedge.iter().any(|&v| v == 0.0 || v.is_nan());
    let mh_has_features = manhedge.iter().any(|&v| v == 1.0);

    let mh_dist = if !mh_has_na {
        vec![0.0; total]
    } else if !mh_has_features {
        vec![f64::NAN; total]
    } else {
        distance_transform_with_buffer(&manhedge, nrows, ncols, LIDAR_BUFFER_CELLS)
    };

    let umh_has_na = unmanhedge.iter().any(|&v| v == 0.0 || v.is_nan());
    let umh_has_features = unmanhedge.iter().any(|&v| v == 1.0);

    let umh_dist = if !umh_has_na {
        vec![0.0; total]
    } else if !umh_has_features {
        vec![f64::NAN; total]
    } else {
        distance_transform_with_buffer(&unmanhedge, nrows, ncols, LIDAR_BUFFER_CELLS)
    };

    let t_has_na = tree.iter().any(|&v| v == 0.0 || v.is_nan());
    let t_has_features = tree.iter().any(|&v| v == 1.0);

    let tree_dist = if !t_has_na {
        vec![0.0; total]
    } else if !t_has_features {
        vec![f64::NAN; total]
    } else {
        distance_transform_with_buffer(&tree, nrows, ncols, LIDAR_BUFFER_CELLS)
    };

    let distance_rasters = vec![
        (umh_dist, 1.0),
        (tree_dist, 2.0),
        (mh_dist, 4.0),
    ];

    LidarOutput {
        manhedge,
        unmanhedge,
        tree,
        distance_rasters,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_surface_height() {
        let dtm = vec![10.0; 36];
        let dsm = vec![15.0; 36];
        let buildings = vec![0.0; 36];
        let result = calc_surfs(&dtm, &dsm, &buildings, 6, 6);
        assert!((result.surf[0] - 5.0).abs() < 0.01);
        assert!((result.soft_surf[0] - 5.0).abs() < 0.01);
        assert!((result.hard_surf[0] - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_building_hard_surf() {
        let nrows = 2;
        let ncols = 2;
        let dtm = vec![10.0; 4];
        let dsm = vec![20.0; 4];
        let buildings = vec![1.0, 1.0, 0.0, 0.0];
        let result = calc_surfs(&dtm, &dsm, &buildings, nrows, ncols);
        assert!(result.hard_surf[0] > 0.0);
        assert!(result.soft_surf[0] < 0.001);
        assert!(result.soft_surf[3] > 0.0);
    }

    #[test]
    fn test_lidar_classification() {
        let nrows = 5;
        let ncols = 5;
        let mut soft = vec![0.0f64; 25];
        soft[0] = 0.3;
        soft[1] = 2.0;
        soft[2] = 4.0;
        soft[3] = 7.0;
        soft[4] = f64::NAN;
        let result = prep_lidar_rasters(&soft, nrows, ncols);
        assert!(result.manhedge[1] == 1.0, "2m → manhedge");
        assert!(result.unmanhedge[2] == 1.0, "4m → unmanhedge");
        assert!(result.tree[3] == 1.0, "7m → tree");
        assert!(result.manhedge[0] == 0.0, "0.3m → not manhedge");
    }
}
