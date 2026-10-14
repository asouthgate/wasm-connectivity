use super::distance::euclidean_distance_transform;

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

// R reference uses ``max_d = 999999999999`` for the "no features" case, so a
// missing feature class resolves to the far-field resistance rather than NaN.
const MAX_D: f64 = 1.0e12;

#[derive(Clone, Copy)]
pub(crate) enum VegetationClass {
    UnmanagedHedge,
    Tree,
    ManagedHedge,
}

pub(crate) fn vegetation_mask(soft_surf: &[f64], class: VegetationClass) -> Vec<f64> {
    soft_surf.iter().map(|&height| {
        if !height.is_finite() {
            return f64::NAN;
        }
        let present = match class {
            VegetationClass::UnmanagedHedge => height > 3.0 && height < 6.0,
            VegetationClass::Tree => height >= 6.0,
            VegetationClass::ManagedHedge => height > 1.0 && height < 3.0,
        };
        if present { 1.0 } else { 0.0 }
    }).collect()
}

pub(crate) fn vegetation_distance(mask: &[f64], nrows: usize, ncols: usize) -> Vec<f64> {
    let has_features = mask.iter().any(|&v| v == 1.0);
    let has_empty = mask.iter().any(|&v| v == 0.0);
    if !has_features {
        // Missing classes retain the R reference's finite far-field distance.
        mask.iter().map(|&v| if v.is_nan() { f64::NAN } else { MAX_D }).collect()
    } else if !has_empty {
        mask.iter().map(|&v| if v.is_nan() { f64::NAN } else { 0.0 }).collect()
    } else {
        let mut distance = euclidean_distance_transform(mask, nrows, ncols);
        for (value, &cell) in distance.iter_mut().zip(mask) {
            if cell.is_nan() {
                *value = f64::NAN;
            }
        }
        distance
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
        let mut soft = vec![0.0f64; 25];
        soft[0] = 0.3;
        soft[1] = 2.0;
        soft[2] = 4.0;
        soft[3] = 7.0;
        soft[4] = f64::NAN;
        let managed = vegetation_mask(&soft, VegetationClass::ManagedHedge);
        let unmanaged = vegetation_mask(&soft, VegetationClass::UnmanagedHedge);
        let tree = vegetation_mask(&soft, VegetationClass::Tree);
        assert_eq!(managed[1], 1.0, "2m -> manhedge");
        assert_eq!(unmanaged[2], 1.0, "4m -> unmanhedge");
        assert_eq!(tree[3], 1.0, "7m -> tree");
        assert_eq!(managed[0], 0.0, "0.3m -> not manhedge");
        assert!(managed[4].is_nan(), "NA soft_surf -> NA manhedge");
        assert!(tree[4].is_nan(), "NA soft_surf -> NA tree");
    }

    #[test]
    fn vegetation_thresholds_and_nonfinite_heights() {
        let heights = [1.0, 3.0, 6.0, f64::INFINITY, f64::NEG_INFINITY];
        for (class, expected) in [
            (VegetationClass::ManagedHedge, [0.0, 0.0, 0.0]),
            (VegetationClass::UnmanagedHedge, [0.0, 0.0, 0.0]),
            (VegetationClass::Tree, [0.0, 0.0, 1.0]),
        ] {
            let mask = vegetation_mask(&heights, class);
            assert_eq!(&mask[..3], &expected);
            assert!(mask[3].is_nan() && mask[4].is_nan());
        }
    }

    #[test]
    fn vegetation_distances_preserve_empty_full_and_missing_cells() {
        for (mask, expected) in [
            ([0.0, 0.0, f64::NAN, 0.0], [MAX_D, MAX_D, f64::NAN, MAX_D]),
            ([1.0, 1.0, f64::NAN, 1.0], [0.0, 0.0, f64::NAN, 0.0]),
            ([1.0, 0.0, f64::NAN, 0.0], [0.0, 1.0, f64::NAN, 2.0_f64.sqrt()]),
        ] {
            let distance = vegetation_distance(&mask, 2, 2);
            for (actual, expected) in distance.iter().zip(expected) {
                assert!(*actual == expected || (actual.is_nan() && expected.is_nan()));
            }
        }
        assert!(vegetation_distance(&[f64::NAN; 4], 2, 2).iter().all(|d| d.is_nan()));
    }
}
