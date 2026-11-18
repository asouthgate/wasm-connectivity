use crate::linalg::multigrid::MgOptions;
use crate::solve::{
    solve_raster_sources_mg_stencil_with_options, solve_raster_sources_mg_with_options, GroundMode,
};

/// Current solver settings, independent of source and ground arrays.
#[derive(Clone, Copy, Debug)]
pub struct CurrentSolverSettings {
    pub max_iter: usize,
    pub tolerance: f64,
    pub ground_mode: GroundMode,
    pub explicit: bool,
    pub multigrid: MgOptions,
}

impl Default for CurrentSolverSettings {
    fn default() -> Self {
        Self {
            max_iter: 100_000,
            tolerance: 1e-4,
            ground_mode: GroundMode::Shunt,
            explicit: false,
            multigrid: MgOptions::default(),
        }
    }
}

/// Current solver with borrowed source/ground arrays and solver settings.
pub struct CurrentSolver<'a> {
    pub source: &'a [f64],
    pub ground: &'a [f64],
    pub settings: CurrentSolverSettings,
}
impl<'a> CurrentSolver<'a> {
    pub fn new(source: &'a [f64], ground: &'a [f64], settings: CurrentSolverSettings) -> Self {
        Self {
            source,
            ground,
            settings,
        }
    }
    pub fn validate(&self, size: usize) -> Result<(), String> {
        if self.settings.max_iter == 0
            || !self.settings.tolerance.is_finite()
            || self.settings.tolerance <= 0.0
        {
            return Err(
                "current solver requires positive iterations and finite positive tolerance".into(),
            );
        }
        for (name, values) in [("source", self.source), ("ground", self.ground)] {
            if values.len() != size {
                return Err(format!("current {name} dimensions differ from resistance"));
            }
            if !values.iter().any(|v| v.is_finite() && *v > 0.0) {
                return Err(format!("no current {name} cells"));
            }
        }
        Ok(())
    }
    pub fn solve_current(
        &self,
        resistance: &[f64],
        nrows: usize,
        ncols: usize,
    ) -> Result<Vec<f64>, String> {
        let size = nrows
            .checked_mul(ncols)
            .ok_or("current grid dimensions overflow")?;
        self.validate(size)?;
        if resistance.len() != size {
            return Err("current resistance dimensions differ from grid".into());
        }
        let solve = if self.settings.explicit {
            solve_raster_sources_mg_with_options
        } else {
            solve_raster_sources_mg_stencil_with_options
        };
        let mut result = solve(
            resistance,
            nrows,
            ncols,
            -9999.0,
            self.source,
            self.ground,
            self.settings.max_iter,
            self.settings.tolerance,
            false,
            self.settings.ground_mode,
            self.settings.multigrid,
        );
        if let Some(error) = result.error {
            return Err(error);
        }
        result.output.mask_invalid_current(resistance, -9999.0)?;
        Ok(result.output.current_map)
    }
}
