const LOG10: f64 = 2.302585092994046;

#[derive(Debug, Clone, Copy)]
struct Lamp {
    ri: usize,
    cj: usize,
    z: f64,
}

// Compute the raycasted irradiance from a single lamp to the terrain, taking into account soft and hard shadows
//
// Works by iterating over the pixels in the terrain that are within the cutoff distance of the lamp, 
// and for each pixel, computing the distance to the lamp,
// checking if there are any hard or soft shadows in the way.
// If there are no hard shadows, the irradiance is computed based on the inverse square law,
// and if there are soft shadows, the irradiance is reduced based on the amount of shading. 
//
// # Arguments
// * irr: a mutable 2D array of f64 values, where the computed irradiance values will be stored
// * m: number of rows in the terrain
// * n: number of columns in the terrain
// * ri_lamp: row index of the lamp
// * cj_lamp: column index of the lamp
// * z: height of the lamp above the terrain
// * soft: a 2D array of f64 values representing the soft shadow heights
// * hard: a 2D array of f64 values representing the hard shadow heights
// * terr: a 2D array of f64 values representing the terrain heights
// * absorb: the absorption coefficient for the irradiance
// * pixw: the width of a pixel in the terrain
// * cutoff: the maximum distance for the raycast
// * sensor_ht: the height of the sensor above the terrain
fn raycast(
    irr: &mut [f64],
    m: usize,
    n: usize,
    ri_lamp: usize,
    cj_lamp: usize,
    z: f64,
    soft: &[f64],
    hard: &[f64],
    terr: &[f64],
    absorb: f64,
    pixw: f64,
    cutoff: f64,
    sensor_ht: f64,
) {
    // ignore a lamp out of bounds
    if ri_lamp >= m || cj_lamp >= n {
        return;
    }

    // compute the pixel cutoff distance and bounds
    let px_cutoff = (cutoff / pixw).ceil() as isize;
    let minj = if cj_lamp as isize - px_cutoff < 0 {
        0
    } else {
        cj_lamp as isize - px_cutoff
    } as usize;
    let maxj = ((cj_lamp as isize + px_cutoff) as usize).min(n);
    let mini = if ri_lamp as isize - px_cutoff < 0 {
        0
    } else {
        ri_lamp as isize - px_cutoff
    } as usize;
    let maxi = ((ri_lamp as isize + px_cutoff) as usize).min(m);

    // compute the lamp elevation above the terrain
    let lamp_elev = terr[ri_lamp * n + cj_lamp] + z;

    for ri in mini..maxi {

        let pydist_base = ri_lamp as f64 - ri as f64;

        for cj in minj..maxj {
            // compute euclidean distance in xy plane
            let pxdist_base = cj_lamp as f64 - cj as f64;
            let pxdist2 = pxdist_base * pxdist_base;
            let pxydist = (pxdist2 + pydist_base * pydist_base).sqrt();
            let pdist = (pxydist + 0.5).floor() as isize;
            // compute z distance from lamp to terrain at this pixel
            let zdist = lamp_elev - (terr[ri * n + cj] + sensor_ht);
            let xydist = pxydist * pixw;
            let xyzdist2 = xydist * xydist + zdist * zdist;

            // skip if the pixel is out of range or below the lamp base
            // NOTE: in principle, we could compute the irradiance for pixels below the lamp base,
            // this could apply in some scenarios
            if xydist >= cutoff || zdist <= 0.0 || pdist <= 0 {
                continue;
            }

            // initialize shadow and shading values
            let mut shadow = 1.0f64;
            let mut shading = 0.0f64;

            // now raycast from the lamp to the pixel, checking for hard and soft shadows
            let step_i = pydist_base / pdist as f64;
            let step_j = pxdist_base / pdist as f64;
            let step_h = zdist / pdist as f64;
            let cell_elev = terr[ri * n + cj] + sensor_ht;

            for d in 1..=pdist {
                let frac = d as f64;
                let dii = (ri as f64 + step_i * frac).round() as isize;
                let djj = (cj as f64 + step_j * frac).round() as isize;

                if dii < 0 || djj < 0 {
                    continue;
                }
                let dii = dii as usize;
                let djj = djj as usize;
                if dii >= m || djj >= n {
                    continue;
                }

                let hiijj = cell_elev + step_h * frac;

                let hard_val = hard[dii * n + djj];
                let terr_val = terr[dii * n + djj];
                let soft_val = soft[dii * n + djj];
                
                // if the hard shadow is above the ray, then the pixel is in shadow and we can break early
                if hard_val.is_finite() && terr_val.is_finite() && hard_val + terr_val >= hiijj {
                    shadow = 0.0;
                    break;
                }
                if soft_val.is_finite() && terr_val.is_finite() && soft_val + terr_val >= hiijj {
                    shading += pixw * xyzdist2.sqrt() / xydist;
                }
            }

            let invd = 1.0 / xyzdist2;
            // compute the irradiance at this pixel, taking into account the shadow and shading values
            // this is exponential decay based on the distance and the absorption coefficient
            let occ = 1.0 / (absorb * shading * LOG10).exp();
            irr[ri * n + cj] += occ * shadow * invd;
        }
    }
}

// Compute the irradiance from multiple lamps, taking into account soft and hard shadows
//
// # Arguments
// * lamps: a 1D array of f64 values, where each lamp is represented
//   by three consecutive values: row index, column index, and height above the terrain
// * soft: a 2D array of f64 values representing the soft shadow heights
// * hard: a 2D array of f64 values representing the hard shadow heights
// * terr: a 2D array of f64 values representing the terrain heights
// * m: number of rows in the terrain
// * n: number of columns in the terrain
// * pixw: the width of a pixel in the terrain
// * cutoff: the maximum distance for the raycast
// * sensor_ht: the height of the sensor above the terrain
// * absorb: the absorption coefficient for the irradiance
// # Returns
// A 2D array of f64 values, where each value is the computed irradiance
pub fn irradiance_run(
    lamps: &[f64],
    soft: &[f64],
    hard: &[f64],
    terr: &[f64],
    m: usize,
    n: usize,
    pixw: f64,
    cutoff: f64,
    sensor_ht: f64,
    absorb: f64,
) -> Vec<f64> {
    assert_eq!(lamps.len() % 3, 0, "lamps array length must be a multiple of 3");
    let nlamps = lamps.len() / 3;
    let total = m * n;
    assert_eq!(soft.len(), total);
    assert_eq!(hard.len(), total);
    assert_eq!(terr.len(), total);

    let mut parsed: Vec<Lamp> = Vec::with_capacity(nlamps);
    for i in 0..nlamps {
        let cj = lamps[i * 3].round() as usize;
        let ri = lamps[i * 3 + 1].round() as usize;
        let z = lamps[i * 3 + 2];
        parsed.push(Lamp { ri, cj, z });
    }
    parsed.sort_by(|a, b| a.ri.cmp(&b.ri).then(a.cj.cmp(&b.cj)));

    let mut output = vec![0.0f64; total];

    for lamp in &parsed {
        raycast(
            &mut output,
            m,
            n,
            lamp.ri,
            lamp.cj,
            lamp.z,
            soft,
            hard,
            terr,
            absorb,
            pixw,
            cutoff,
            sensor_ht,
        );
    }

    output
}

// Convert irradiance values to resistance values using a power-law transformation
pub fn irradiance_to_resistance(
    io_raster: &mut [f64],
    m: usize,
    n: usize,
    resmax: f64,
    xmax: f64,
) {
    let total = m * n;
    let mut maxpi = 0.0f64;
    for v in io_raster.iter().take(total) {
        if v.is_finite() && *v > maxpi {
            maxpi = *v;
        }
    }
    if maxpi <= 0.0 {
        return;
    }
    for v in io_raster.iter_mut().take(total) {
        if v.is_finite() && *v > 0.0 {
            *v = (*v / maxpi).powf(xmax) * resmax;
        }
    }
}

// Compute the combined irradiance from multiple sources and squash the values to a specified range
//
// # Arguments
// * lamp: a 2D array of f64 values representing the irradiance from lamps
// * road: a 2D array of f64 values representing the irradiance from roads
// * river: a 2D array of f64 values representing the irradiance from rivers
// * landscape: a 2D array of f64 values representing the irradiance from landscapes
// * linear: a 2D array of f64 values representing the irradiance from linear features
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_irradiance_inverse_square_falloff() {
        let m = 11;
        let n = 11;
        let size = m * n;
        let soft = vec![0.0f64; size];
        let hard = vec![0.0f64; size];
        let terr = vec![0.0f64; size];
        let lamps = vec![5.0, 5.0, 10.0];

        let result = irradiance_run(&lamps, &soft, &hard, &terr, m, n, 1.0, 100.0, 0.0, 0.5);

        let adjacent = result[5 * n + 6];
        assert!(
            adjacent > 0.0,
            "adjacent to lamp should receive irradiance, got {}",
            adjacent
        );

        let top_left = result[0 * n + 0];
        assert!(
            top_left < adjacent,
            "far field should receive less than near field"
        );

        let mut raster = result.clone();
        irradiance_to_resistance(&mut raster, m, n, 100.0, 1.0);
        let max_irr = result.iter().cloned().fold(0.0f64, f64::max);
        let max_idx = result.iter().position(|&v| (v - max_irr).abs() < 1e-12).unwrap_or(0);
        assert!((raster[max_idx] - 100.0).abs() < 0.001, "cell with max irradiance should get resmax");
        assert!(raster.iter().all(|&v| v >= 0.0));
    }

    #[test]
    fn test_irradiance_hard_shadow() {
        let m = 5;
        let n = 5;
        let size = m * n;
        let mut hard = vec![0.0f64; size];
        hard[2 * n + 3] = 10.0;
        let soft = vec![0.0f64; size];
        let terr = vec![0.0f64; size];
        let lamps = vec![2.0, 2.0, 10.0];

        let result = irradiance_run(&lamps, &soft, &hard, &terr, m, n, 1.0, 100.0, 0.0, 0.5);

        let behind_wall = result[2 * n + 4];
        assert!(
            behind_wall <= 0.001,
            "shadow behind hard surface, got {}",
            behind_wall
        );

        let open_path = result[0 * n + 0];
        assert!(
            open_path > 0.0,
            "open path should get irradiance"
        );
    }
}
