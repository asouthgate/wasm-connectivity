use geo::Point;

// Geospatial transformation struct to handle coordinate
//  conversions between geographic and pixel space.
pub struct GeoTransform {
    pub xmin: f64,
    pub ymax: f64,
    pub cellsize: f64,
}

impl GeoTransform {
    pub(super) fn is_valid(&self) -> bool {
        self.cellsize.is_finite() && self.cellsize > 0.0
    }

    // Converts geographic coordinates (x, y) to pixel coordinates (col, row).
    pub(super) fn geo_to_pixel(&self, x: f64, y: f64) -> (isize, isize) {
        if !self.is_valid() {
            return (0, 0);
        }
        let col = ((x - self.xmin) / self.cellsize) as isize;
        let row = ((self.ymax - y) / self.cellsize) as isize;
        (col, row)
    }

    // Converts pixel coordinates (col, row) back to geographic coordinates (x, y).
    pub(super) fn pixel_to_geo(&self, col: usize, row: usize) -> Point<f64> {
        Point::new(
            self.xmin + (col as f64 + 0.5) * self.cellsize,
            self.ymax - (row as f64 + 0.5) * self.cellsize,
        )
    }

    // Clips the given geographic bounds to ensure they fit within the raster dimensions.
    pub(super) fn clip_bounds(&self, col_min: isize, row_min: isize, col_max: isize, row_max: isize, nrows: usize, ncols: usize) -> (usize, usize, usize, usize) {
        let col_start = col_min.max(0) as usize;
        let col_end = (col_max.saturating_add(1)).min(ncols as isize).max(0) as usize;
        let row_start = row_min.max(0) as usize;
        let row_end = (row_max.saturating_add(1)).min(nrows as isize).max(0) as usize;
        (col_start, col_end, row_start, row_end)
    }
}
