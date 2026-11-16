pub struct SurfaceOutput {
    pub surf: Vec<f64>,
    pub soft_surf: Vec<f64>,
    pub hard_surf: Vec<f64>,
}

/// Building heights are relative to DTM, with zero outside footprints.
/// Supplied heights override DSM minus DTM, even when the DSM cell is missing.
pub fn calc_surfs(dtm: &[f64], dsm: &[f64], buildings: &[f64], nrows: usize, ncols: usize) -> SurfaceOutput {
    let total = nrows * ncols;
    let mut surf = vec![0.0f64; total];
    let mut soft_surf = vec![0.0f64; total];
    let mut hard_surf = vec![0.0f64; total];

    for i in 0..total {
        if !dtm[i].is_finite() {
            surf[i] = f64::NAN;
            soft_surf[i] = f64::NAN;
            hard_surf[i] = f64::NAN;
        } else if buildings[i].is_finite() && buildings[i] > 0.0 {
            surf[i] = buildings[i];
            hard_surf[i] = buildings[i];
            soft_surf[i] = 0.0;
        } else if dsm[i].is_finite() {
            surf[i] = dsm[i] - dtm[i];
            soft_surf[i] = surf[i];
        } else {
            surf[i] = f64::NAN;
            soft_surf[i] = f64::NAN;
            hard_surf[i] = f64::NAN;
        }
    }

    SurfaceOutput {
        surf,
        soft_surf,
        hard_surf,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_heights_override_background_and_zero_means_no_building() {
        let dtm = [10.0, 10.0, 10.0, 10.0, f64::NAN];
        let dsm = [20.0, 20.0, 20.0, f64::NAN, 20.0];
        let buildings = [1.0, 0.0, 0.0, 3.0, 5.0];
        let result = calc_surfs(&dtm, &dsm, &buildings, 1, 5);
        assert_eq!(&result.surf[..4], &[1.0, 10.0, 10.0, 3.0]);
        assert_eq!(&result.hard_surf[..4], &[1.0, 0.0, 0.0, 3.0]);
        assert_eq!(&result.soft_surf[..4], &[0.0, 10.0, 10.0, 0.0]);
        assert!(result.surf[4].is_nan());
        assert!(result.hard_surf[4].is_nan());
    }

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
        let buildings = vec![10.0, 10.0, 0.0, 0.0];
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

}
