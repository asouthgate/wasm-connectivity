use super::irradiance::{irradiance_run, irradiance_to_resistance};
use super::landscape::{get_landscape_resistance_from_conductance, get_landscape_resistance_lcm};
use super::linear::{initialize_linear_resistance, update_linear_resistance};
use super::road::cal_road_resistance;
use super::river::cal_river_resistance;
use super::surface::{calc_surfs, vegetation_distance, vegetation_mask, VegetationClass};
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

// Combine the resistance rasters into a single additive resistance raster.
// Each layer contributes its resistance value; the total is the plain sum of
// all layers, which is the additive multivariate model fed into Circuitscape.
//
// # Arguments
// * lamp: a 2D array of f64 values representing the lamp resistance
// * road: a 2D array of f64 values representing the road resistance
// * river: a 2D array of f64 values representing the river resistance
// * landscape: a 2D array of f64 values representing the landscape resistance
// * linear: a 2D array of f64 values representing the linear resistance
// * generic: a 2D array of f64 values representing the generic resistance
// * m: the number of rows in the rasters
// * n: the number of columns in the rasters
// # Returns
// A 2D array of f64 values representing the combined resistance for each pixel
pub fn combine_resistance(
    lamp: &[f64],
    road: &[f64],
    river: &[f64],
    landscape: &[f64],
    linear: &[f64],
    generic: &[f64],
    m: usize,
    n: usize,
) -> Vec<f64> {
    let sz = m * n;
    let mut total = vec![0.0f64; sz];
    for i in 0..sz {
        total[i] = lamp[i] + road[i] + river[i] + landscape[i] + linear[i] + generic[i];
    }
    total
}

// Run the geospatial resistance pipeline used as input into Circuitscape. 
// This pipeline was designed for Greater Horseshoe Bats (Rhinolophus ferrumequinum) in the UK,
// but can be adapted for other species and regions.
//
// See Finch, D., Corbacho, D.P., Schofield, H., Davison,
// S., Wright, P.G., Broughton, R.K. and Mathews, F., 2020.
// Modelling the functional connectivity of landscapes for
// greater horseshoe bats Rhinolophus ferrumequinum at a local scale.
// Landscape Ecology, 35(3), pp.577-589.
// 
// # Arguments
// * road_binary: a 2D array of f64 values where non-zero values indicate the presence of a road
// * river_binary: a 2D array of f64 values where non-zero values indicate the presence of a river
// * building_mask: a 2D array of f64 values where non-zero values indicate the presence of a building
// * lcm: the land cover map; optional when landscape conductance is supplied
// * dtm: a 2D array of f64 values representing the digital terrain model
// * dsm: a 2D array of f64 values representing the digital surface model
// * generic_resistance: a 2D array of f64 values representing the generic resistance
// * lamps: a 2D array of f64 values representing the lamp locations and properties
// * params: a ResistanceParams struct containing the parameters for the resistance calculations
// * landscape_conductance_override: an optional 2D array of f64 values representing the landscape conductance to override the default calculation
// * lightmap: an optional 2D array of f64 values representing a pre-computed
//   irradiance/light map. When present, it replaces the raycasted irradiance
//   from `lamps`; it is normalised and scaled to resistance via
//   `irradiance_to_resistance` (see below). Must be `m * n` elements when set.
// # Returns
// A ResistanceOutput struct containing the resistance rasters and other outputs from the pipeline
pub fn run_resistance_pipeline(
    road_binary: &[f64],
    river_binary: &[f64],
    building_mask: &[f64],
    lcm: Option<&[f64]>,
    dtm: &[f64],
    dsm: &[f64],
    generic_resistance: &[f64],
    lamps: &[f64],
    params: &ResistanceParams,
    landscape_conductance_override: Option<&[f64]>,
    lightmap: Option<&[f64]>,
) -> ResistanceOutput {
    run_resistance_pipeline_impl(
        road_binary, river_binary, building_mask, lcm, dtm, dsm, generic_resistance,
        lamps, params, landscape_conductance_override, lightmap, true,
    )
}

pub(crate) fn run_resistance_pipeline_impl(
    road_binary: &[f64],
    river_binary: &[f64],
    building_mask: &[f64],
    lcm: Option<&[f64]>,
    dtm: &[f64],
    dsm: &[f64],
    generic_resistance: &[f64],
    lamps: &[f64],
    params: &ResistanceParams,
    landscape_conductance_override: Option<&[f64]>,
    lightmap: Option<&[f64]>,
    retain_vegetation_masks: bool,
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
        params.pixw,
    );

    let river_res = cal_river_resistance(
        river_binary,
        m,
        n,
        params.river_buffer,
        params.river_resmax,
        params.river_xmax,
        params.pixw,
    );

    let surfs = calc_surfs(dtm, dsm, building_mask, m, n);
    drop(surfs.surf);

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
            lcm.expect("LCM is required without a landscape conductance override"),
            building_mask,
            params.landscape_rankmax,
            params.landscape_resmax,
            params.landscape_xmax,
        )
    };

    let mut linear_res = initialize_linear_resistance(surfs.soft_surf.iter().map(|h| !h.is_finite()));
    let mut vegetation_masks = [Vec::new(), Vec::new(), Vec::new()];
    // Preserve the unmanaged hedge / tree / managed hedge ranking order.
    for (class, ranking, index) in [
        (VegetationClass::UnmanagedHedge, 1.0, 1),
        (VegetationClass::Tree, 2.0, 2),
        (VegetationClass::ManagedHedge, 4.0, 0),
    ] {
        let mask = vegetation_mask(&surfs.soft_surf, class);
        let distance = vegetation_distance(&mask, m, n);
        update_linear_resistance(
            &mut linear_res, &distance, ranking, params.linear_buffer,
            params.linear_rankmax, params.linear_resmax, params.linear_xmax, params.pixw,
        );
        drop(distance);
        if retain_vegetation_masks {
            vegetation_masks[index] = mask;
        }
    }
    let [manhedge, unmanhedge, tree] = vegetation_masks;

    let mut lamp_res = if let Some(lightmap) = lightmap {
        // Pre-computed light map: skip the raycasted irradiance and just
        // normalise (max -> 1) and scale by the lamp resistance factor.
        debug_assert_eq!(lightmap.len(), total, "lightmap must be m * n");
        let mut lr = lightmap.to_vec();
        irradiance_to_resistance(&mut lr, m, n, params.lamp_resmax, params.lamp_xmax);
        lr
    } else if lamps.len() >= 3 {
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
        .map(|&v| if v.is_finite() { v } else { f64::NAN })
        .collect();

    for i in 0..total {
        let terrain_missing = !dsm[i].is_finite() || !dtm[i].is_finite();
        if terrain_missing || lcm.is_some_and(|data| !data[i].is_finite()) {
            landscape_res[i] = f64::NAN;
        }
        if terrain_missing {
            linear_res[i] = f64::NAN;
            lamp_res[i] = f64::NAN;
        }
    }

    let total_res = combine_resistance(
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
        if !dsm[i].is_finite() || !dtm[i].is_finite() {
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
        manhedge,
        unmanhedge,
        tree,
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
            &zeros, &zeros, &zeros, Some(&lcm), &dtm, &dsm, &zeros, &[],
            &params, None, None,
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
            &zeros, &zeros, &zeros, Some(&lcm), &dtm, &dsm, &zeros, &lamps,
            &params, None, None,
        );

        assert!(output.lamp_res.iter().any(|&v| v > 0.0), "lamp should produce non-zero resistance");
    }

    #[test]
    fn test_lightmap_path() {
        let total = 25;
        let zeros = vec![0.0f64; total];
        let dtm = vec![1.0f64; total];
        let dsm = vec![2.0f64; total];
        let lcm = vec![1.0f64; total];
        // Brightest cell in the middle; everything else dimmer.
        let mut lightmap = vec![0.1f64; total];
        lightmap[2 * 5 + 2] = 10.0;
        let params = make_params();

        let output = run_resistance_pipeline(
            &zeros, &zeros, &zeros, Some(&lcm), &dtm, &dsm, &zeros, &[],
            &params, None, Some(&lightmap),
        );

        let max = output
            .lamp_res
            .iter()
            .cloned()
            .fold(0.0f64, f64::max);
        assert!(
            (max - params.lamp_resmax).abs() < 1e-3,
            "brightest light-map cell should map to lamp_resmax"
        );
        assert!(
            output.lamp_res.iter().all(|&v| v >= 0.0),
            "light-map resistance should be non-negative"
        );
    }

    #[test]
    fn test_combine_resistance_additive() {
        let m = 2;
        let n = 2;
        let sz = m * n;
        let a = vec![10.0, 20.0, 30.0, 40.0];
        let b = vec![1.0, 2.0, 3.0, 4.0];
        let z = vec![0.0f64; sz];
        let result = combine_resistance(&a, &b, &z, &z, &z, &z, m, n);
        assert_eq!(result, vec![11.0, 22.0, 33.0, 44.0]);
    }

    #[test]
    fn test_nan_propagation() {
        let total = 25;
        let zeros = vec![0.0f64; total];
        let mut dtm = vec![1.0f64; total];
        let mut dsm = vec![2.0f64; total];
        let lcm = vec![1.0f64; total];
        dtm[3] = f64::NAN;
        dsm[3] = f64::NAN;
        let params = make_params();

        let output = run_resistance_pipeline(
            &zeros, &zeros, &zeros, Some(&lcm), &dtm, &dsm, &zeros, &[],
            &params, None, None,
        );

        assert!(output.soft_surf[3].is_nan(), "NA DTM/DSM -> NA soft_surf");
        assert!(output.hard_surf[3].is_nan(), "NA DTM/DSM -> NA hard_surf");
        assert!(output.landscape_res[3].is_nan(), "NA DTM/DSM -> NA landscape_res");
        assert!(output.linear_res[3].is_nan(), "NA DTM/DSM -> NA linear_res");
        assert!(output.total_res[3].is_nan(), "NA DTM/DSM -> NA total_res");
        assert!(output.soft_surf[0].is_finite(), "valid cell stays finite");
    }

    #[test]
    fn test_generic_resistance_nan() {
        let total = 25;
        let zeros = vec![0.0f64; total];
        let dtm = vec![1.0f64; total];
        let dsm = vec![2.0f64; total];
        let lcm = vec![1.0f64; total];
        let mut generic = vec![5.0f64; total];
        generic[7] = f64::NAN;
        let params = make_params();

        let output = run_resistance_pipeline(
            &zeros, &zeros, &zeros, Some(&lcm), &dtm, &dsm, &generic, &[],
            &params, None, None,
        );

        assert!(output.generic_res[7].is_nan(), "NA generic -> NA generic_res");
        assert!(output.generic_res[0].is_finite());
    }

    #[test]
    fn landscape_override_without_lcm_preserves_browser_masking() {
        let params = make_params();
        let zeros = vec![0.0; 25];
        let mut dtm = vec![10.0; 25];
        let mut dsm: Vec<f64> = (0..25).map(|i| 10.0 + [0.0, 2.0, 4.0, 7.0][i % 4]).collect();
        let mut buildings = zeros.clone();
        let mut ranks = vec![4.0; 25];
        ranks[0] = f64::NAN;
        ranks[1] = f64::NAN;
        buildings[0] = 3.0;
        dtm[2] = f64::NAN;
        dsm[3] = f64::INFINITY;

        let without_lcm = run_resistance_pipeline(
            &zeros, &zeros, &buildings, None, &dtm, &dsm, &zeros, &[],
            &params, Some(&ranks), None,
        );
        let with_placeholder = run_resistance_pipeline(
            &zeros, &zeros, &buildings, Some(&zeros), &dtm, &dsm, &zeros, &[],
            &params, Some(&ranks), None,
        );
        assert_eq!(serde_json::to_value(&without_lcm).unwrap(),
            serde_json::to_value(&with_placeholder).unwrap());
        assert_eq!(without_lcm.landscape_res[0], params.landscape_resmax);
        assert!(without_lcm.landscape_res[1].is_nan());
        assert!(without_lcm.total_res[2].is_nan());
        assert!(without_lcm.total_res[3].is_nan());
    }

    #[test]
    fn native_lcm_missing_cells_still_mask_landscape() {
        let params = make_params();
        let zeros = vec![0.0; 25];
        let dtm = vec![10.0; 25];
        let dsm = vec![12.0; 25];
        let mut lcm = vec![4.0; 25];
        let mut buildings = zeros.clone();
        let ranks = vec![4.0; 25];
        lcm[7] = f64::NAN;
        buildings[7] = 3.0;
        for override_ranks in [None, Some(ranks.as_slice())] {
            let output = run_resistance_pipeline(
                &zeros, &zeros, &buildings, Some(&lcm), &dtm, &dsm, &zeros, &[],
                &params, override_ranks, None,
            );
            assert!(output.landscape_res[7].is_nan());
            assert!(output.total_res[7].is_nan());
            assert!(output.linear_res[7].is_finite());
            assert!(output.lamp_res[7].is_finite());
            assert!(output.landscape_res[0].is_finite());
        }
    }

    #[test]
    fn sequential_vegetation_preserves_far_field_and_full_class_resistance() {
        let params = make_params();
        let zeros = vec![0.0; 25];
        let dtm = vec![10.0; 25];
        let lcm = vec![4.0; 25];
        // Empty managed hedges dominate the far field; with full managed hedges,
        // the absent tree class supplies the largest contribution instead.
        for (height, expected) in [
            (0.0, 22002.0), (1.0, 22002.0), (3.0, 22002.0),
            (2.0, 9283.25), (4.0, 22002.0), (6.0, 22002.0),
            (f64::NAN, f64::NAN), (f64::INFINITY, f64::NAN),
        ] {
            let dsm = vec![10.0 + height; 25];
            for retain_masks in [true, false] {
                let output = run_resistance_pipeline_impl(
                    &zeros, &zeros, &zeros, Some(&lcm), &dtm, &dsm, &zeros, &[],
                    &params, None, None, retain_masks,
                );
                assert!(output.linear_res.iter().all(|&r|
                    r == expected || (r.is_nan() && expected.is_nan())));
                for mask in [&output.manhedge, &output.unmanhedge, &output.tree] {
                    assert_eq!(mask.len(), if retain_masks { 25 } else { 0 });
                }
            }
        }
    }
}
