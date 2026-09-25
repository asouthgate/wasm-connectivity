use super::distance::distance_transform_with_buffer;

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
        if !dtm[i].is_finite() || !dsm[i].is_finite() {
            surf[i] = f64::NAN;
            soft_surf[i] = f64::NAN;
            hard_surf[i] = f64::NAN;
            continue;
        }
        let sv = dsm[i] - dtm[i];
        surf[i] = sv;
        soft_surf[i] = sv;
    }

    for i in 0..total {
        if !surf[i].is_finite() {
            continue;
        }
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
    pub missing: Vec<bool>,
}

pub fn prep_lidar_rasters(soft_surf: &[f64], nrows: usize, ncols: usize, pixw: f64) -> LidarOutput {
    let total = nrows * ncols;
    let buf_cells = (10.0 / pixw).max(1.0);

    let missing: Vec<bool> = soft_surf.iter().map(|&h| !h.is_finite()).collect();

    let mut manhedge = vec![f64::NAN; total];
    let mut unmanhedge = vec![f64::NAN; total];
    let mut tree = vec![f64::NAN; total];

    for i in 0..total {
        if missing[i] {
            continue;
        }
        let h = soft_surf[i];
        manhedge[i] = if h > 1.0 && h < 3.0 { 1.0 } else { 0.0 };
        unmanhedge[i] = if h > 3.0 && h < 6.0 { 1.0 } else { 0.0 };
        tree[i] = if h >= 6.0 { 1.0 } else { 0.0 };
    }

    let compute_dist = |mask: &[f64]| -> Vec<f64> {
        let has_features = mask.iter().any(|&v| v == 1.0);
        let has_empty = mask.iter().any(|&v| v == 0.0);

        if !has_features {
            // No features anywhere: valid cells have no contribution (NaN),
            // missing cells stay NaN.
            vec![f64::NAN; total]
        } else if !has_empty {
            // Every non-missing cell is a feature: distance 0; missing stays NaN.
            mask.iter()
                .map(|&v| if v.is_nan() { f64::NAN } else { 0.0 })
                .collect()
        } else {
            let mut d = distance_transform_with_buffer(mask, nrows, ncols, buf_cells);
            for i in 0..total {
                if missing[i] {
                    d[i] = f64::NAN;
                }
            }
            d
        }
    };

    let mh_dist = compute_dist(&manhedge);
    let umh_dist = compute_dist(&unmanhedge);
    let tree_dist = compute_dist(&tree);

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
        missing,
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
    fn test_surface_na_propagation() {
        let nrows = 1;
        let ncols = 3;
        let dtm = vec![10.0, f64::NAN, 10.0];
        let dsm = vec![15.0, 15.0, f64::NAN];
        let buildings = vec![0.0; 3];
        let result = calc_surfs(&dtm, &dsm, &buildings, nrows, ncols);
        assert!(result.soft_surf[1].is_nan(), "dtm NA should yield NA soft_surf");
        assert!(result.soft_surf[2].is_nan(), "dsm NA should yield NA soft_surf");
        assert!(result.hard_surf[1].is_nan(), "dtm NA should yield NA hard_surf");
        assert!(result.hard_surf[2].is_nan(), "dsm NA should yield NA hard_surf");
        assert!((result.soft_surf[0] - 5.0).abs() < 0.01);
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
        let result = prep_lidar_rasters(&soft, nrows, ncols, 10.0);
        assert!(result.manhedge[1] == 1.0, "2m → manhedge");
        assert!(result.unmanhedge[2] == 1.0, "4m → unmanhedge");
        assert!(result.tree[3] == 1.0, "7m → tree");
        assert!(result.manhedge[0] == 0.0, "0.3m → not manhedge");
        assert!(result.manhedge[4].is_nan(), "NA soft_surf → NA manhedge");
        assert!(result.tree[4].is_nan(), "NA soft_surf → NA tree");
        assert!(result.missing[4], "NA soft_surf → missing");
        assert!(!result.missing[0], "finite soft_surf → not missing");
    }
}
