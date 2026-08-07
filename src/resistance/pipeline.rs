use super::irradiance::{combine_and_squash, irradiance_run, irradiance_to_resistance};
use super::landscape::{get_landscape_resistance_from_conductance, get_landscape_resistance_lcm};
use super::linear::get_linear_resistance;
use super::road::cal_road_resistance;
use super::river::cal_river_resistance;
use super::surface::{calc_surfs, prep_lidar_rasters};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ResistanceParams {
    pub road_buffer: f64,
    pub road_resmax: f64,
    pub road_xmax: f64,
    pub river_buffer: f64,
    pub river_resmax: f64,
    pub river_xmax: f64,
    pub landscape_rankmax: f64,
    pub landscape_resmax: f64,
    pub landscape_xmax: f64,
    pub linear_buffer: f64,
    pub linear_rankmax: f64,
    pub linear_resmax: f64,
    pub linear_xmax: f64,
    pub lamp_resmax: f64,
    pub lamp_xmax: f64,
    pub lamp_ext: f64,
    pub pixw: f64,
    pub nrows: usize,
    pub ncols: usize,
}

#[derive(Serialize)]
pub struct ResistanceOutput {
    pub road_res: Vec<f64>,
    pub river_res: Vec<f64>,
    pub landscape_res: Vec<f64>,
    pub linear_res: Vec<f64>,
    pub lamp_res: Vec<f64>,
    pub generic_res: Vec<f64>,
    pub total_res: Vec<f64>,
    pub soft_surf: Vec<f64>,
    pub hard_surf: Vec<f64>,
    pub manhedge: Vec<f64>,
    pub unmanhedge: Vec<f64>,
    pub tree: Vec<f64>,
    pub nrows: usize,
    pub ncols: usize,
}

pub fn run_resistance_pipeline(
    road_binary: &[f64],
    river_binary: &[f64],
    building_mask: &[f64],
    lcm: &[f64],
    dtm: &[f64],
    dsm: &[f64],
    generic_resistance: &[f64],
    lamps: &[f64],
    params: &ResistanceParams,
    landscape_conductance_override: Option<&[f64]>,
) -> ResistanceOutput {
    let m = params.nrows;
    let n = params.ncols;
    let total = m * n;

    let road_res = cal_road_resistance(
        road_binary,
        m,
        n,
        params.road_buffer,
        params.road_resmax,
        params.road_xmax,
    );

    let river_res = cal_river_resistance(
        river_binary,
        m,
        n,
        params.river_buffer,
        params.river_resmax,
        params.river_xmax,
    );

    let surfs = calc_surfs(dtm, dsm, building_mask, m, n);

    let mut landscape_res = if let Some(conductance) = landscape_conductance_override {
        get_landscape_resistance_from_conductance(
            conductance,
            building_mask,
            params.landscape_rankmax,
            params.landscape_resmax,
            params.landscape_xmax,
        )
    } else {
        get_landscape_resistance_lcm(
            lcm,
            building_mask,
            &surfs.soft_surf,
            m,
            n,
            params.landscape_rankmax,
            params.landscape_resmax,
            params.landscape_xmax,
        )
    };

    let lidar = prep_lidar_rasters(&surfs.soft_surf, m, n);
    let mut linear_res = get_linear_resistance(
        &lidar.distance_rasters,
        m,
        n,
        params.linear_buffer,
        params.linear_rankmax,
        params.linear_resmax,
        params.linear_xmax,
    );

    let mut lamp_res = if lamps.len() >= 3 {
        let irradiance = irradiance_run(
            lamps,
            &surfs.soft_surf,
            &surfs.hard_surf,
            dtm,
            m,
            n,
            params.pixw,
            params.lamp_ext,
            0.0,
            0.5,
        );
        let mut lr = irradiance;
        irradiance_to_resistance(&mut lr, m, n, params.lamp_resmax, params.lamp_xmax);
        lr
    } else {
        vec![0.0f64; total]
    };

    let gen_res: Vec<f64> = generic_resistance
        .iter()
        .map(|&v| if v.is_finite() { v } else { 0.0 })
        .collect();

    let dsm_na: Vec<bool> = dsm.iter().map(|&v| !v.is_finite()).collect();
    let dtm_na: Vec<bool> = dtm.iter().map(|&v| !v.is_finite()).collect();
    let lcm_na: Vec<bool> = lcm.iter().map(|&v| !v.is_finite()).collect();

    for i in 0..total {
        if dsm_na[i] || dtm_na[i] || lcm_na[i] {
            landscape_res[i] = f64::NAN;
        }
        if dsm_na[i] || dtm_na[i] {
            linear_res[i] = f64::NAN;
            lamp_res[i] = f64::NAN;
        }
    }

    let total_res = combine_and_squash(
        &lamp_res,
        &road_res,
        &river_res,
        &landscape_res,
        &linear_res,
        &gen_res,
        m,
        n,
    );

    let mut final_total = total_res;
    for i in 0..total {
        if dsm_na[i] || dtm_na[i] {
            final_total[i] = f64::NAN;
        }
    }

    ResistanceOutput {
        road_res,
        river_res,
        landscape_res,
        linear_res,
        lamp_res,
        generic_res: gen_res,
        total_res: final_total,
        soft_surf: surfs.soft_surf,
        hard_surf: surfs.hard_surf,
        manhedge: lidar.manhedge,
        unmanhedge: lidar.unmanhedge,
        tree: lidar.tree,
        nrows: m,
        ncols: n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_params() -> ResistanceParams {
        ResistanceParams {
            road_buffer: 200.0,
            road_resmax: 10.0,
            road_xmax: 5.0,
            river_buffer: 10.0,
            river_resmax: 2000.0,
            river_xmax: 4.0,
            landscape_rankmax: 8.0,
            landscape_resmax: 100.0,
            landscape_xmax: 5.0,
            linear_buffer: 10.0,
            linear_rankmax: 4.0,
            linear_resmax: 22000.0,
            linear_xmax: 3.0,
            lamp_resmax: 1e8,
            lamp_xmax: 1.0,
            lamp_ext: 100.0,
            pixw: 1.0,
            nrows: 5,
            ncols: 5,
        }
    }

    #[test]
    fn test_empty_pipeline() {
        let total = 25;
        let zeros = vec![0.0f64; total];
        let dtm = vec![1.0f64; total];
        let dsm = vec![2.0f64; total];
        let lcm = vec![1.0f64; total];
        let params = make_params();

        let output = run_resistance_pipeline(
            &zeros, &zeros, &zeros, &lcm, &dtm, &dsm, &zeros, &[],
            &params, None,
        );

        assert_eq!(output.total_res.len(), total);
        assert!(output.total_res.iter().all(|&v| v.is_finite()));
    }

    #[test]
    fn test_single_lamp() {
        let total = 25;
        let zeros = vec![0.0f64; total];
        let dtm = vec![1.0f64; total];
        let dsm = vec![2.0f64; total];
        let lcm = vec![1.0f64; total];
        let lamps = vec![2.0, 2.0, 10.0];
        let params = make_params();

        let output = run_resistance_pipeline(
            &zeros, &zeros, &zeros, &lcm, &dtm, &dsm, &zeros, &lamps,
            &params, None,
        );

        assert!(output.lamp_res.iter().any(|&v| v > 0.0), "lamp should produce non-zero resistance");
    }
}
