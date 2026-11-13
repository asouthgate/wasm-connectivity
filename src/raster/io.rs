use crate::linalg::real::{Real, scalar};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

pub struct AscGrid<F: Real = f64> {
    pub data: Vec<F>,
    pub nrows: usize,
    pub ncols: usize,
    pub xllcorner: f64,
    pub yllcorner: f64,
    pub cellsize: f64,
    pub nodata: F,
    pub ymax: f64,
}

pub fn parse_asc<P: AsRef<Path>>(path: P) -> AscGrid { parse_asc_typed::<f64, P>(path) }

pub fn parse_asc_typed<F: Real, P: AsRef<Path>>(path: P) -> AscGrid<F> {
    let text = fs::read_to_string(path.as_ref())
        .unwrap_or_else(|e| panic!("cannot read {}: {}", path.as_ref().display(), e));

    let mut ncols = 0usize;
    let mut nrows = 0usize;
    let mut xllcorner = 0.0f64;
    let mut yllcorner = 0.0f64;
    let mut cellsize = 0.0f64;
    let mut nodata = scalar::<F>(-9999.0);
    let mut data = Vec::new();

    for (i, line) in BufReader::new(text.as_bytes()).lines().enumerate() {
        let line = line.unwrap_or_default();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if i < 6 {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 2 {
                let val = parts[1].parse::<f64>().unwrap_or(0.0);
                match parts[0].to_lowercase().as_str() {
                    "ncols" => ncols = val as usize,
                    "nrows" => nrows = val as usize,
                    "xllcorner" | "xllcenter" => xllcorner = val,
                    "yllcorner" | "yllcenter" => yllcorner = val,
                    "cellsize" => cellsize = val,
                    "nodata_value" => nodata = scalar::<F>(val),
                    _ => {}
                }
            }
        } else {
            for token in trimmed.split_whitespace() {
                data.push(token.parse::<F>().unwrap_or(nodata));
            }
        }
    }

    let ymax = yllcorner + nrows as f64 * cellsize;
    AscGrid { data, nrows, ncols, xllcorner, yllcorner, cellsize, nodata, ymax }
}

pub fn write_asc<F: Real, P: AsRef<Path>>(
    path: P, data: &[F], nrows: usize, ncols: usize,
    xllcorner: f64, yllcorner: f64, cellsize: f64, nodata: F,
) {
    let mut f = fs::File::create(path.as_ref())
        .unwrap_or_else(|e| panic!("cannot create {}: {}", path.as_ref().display(), e));
    writeln!(f, "ncols {}", ncols).unwrap();
    writeln!(f, "nrows {}", nrows).unwrap();
    writeln!(f, "xllcorner {:.6}", xllcorner).unwrap();
    writeln!(f, "yllcorner {:.6}", yllcorner).unwrap();
    writeln!(f, "cellsize {:.6}", cellsize).unwrap();
    writeln!(f, "NODATA_value {}", nodata).unwrap();
    for row in 0..nrows {
        let mut line = String::new();
        for col in 0..ncols {
            if col > 0 {
                line.push(' ');
            }
            line.push_str(&format!("{:.16e}", data[row * ncols + col]));
        }
        writeln!(f, "{}", line).unwrap();
    }
}

pub fn asc_to_png<P: AsRef<Path>, Q: AsRef<Path>>(asc_path: P, png_path: Q) {
    let grid = parse_asc(&asc_path);
    let n = grid.data.len();

    let mut min_val = f64::MAX;
    let mut max_val = f64::MIN;
    for &v in &grid.data {
        if v.is_finite() && v != grid.nodata {
            min_val = min_val.min(v);
            max_val = max_val.max(v);
        }
    }
    if max_val <= min_val {
        max_val = min_val + 1.0;
    }

    let scale = 255.0 / (max_val - min_val);
    let mut pixels = vec![0u8; n];
    for i in 0..n {
        let v = grid.data[i];
        if v.is_finite() && v != grid.nodata {
            let scaled = ((v - min_val) * scale).round() as u32;
            pixels[i] = scaled.min(255) as u8;
        }
    }

    let mut img = image::GrayImage::new(grid.ncols as u32, grid.nrows as u32);
    for row in 0..grid.nrows {
        for col in 0..grid.ncols {
            img.put_pixel(col as u32, row as u32, image::Luma([pixels[row * grid.ncols + col]]));
        }
    }
    img.save(&png_path).unwrap();
    eprintln!("wrote {}", png_path.as_ref().display());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asc_export_preserves_small_currents_and_f64_precision() {
        let path = std::env::temp_dir().join(format!(
            "lmConnect-asc-{}-{}.asc",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        ));
        let values = [0.0, 5.3454610306142845e-43, 1e-300,
                      366.321540637836, -9999.0, f64::MIN_POSITIVE];
        write_asc(&path, &values, 2, 3, 10.0, 20.0, 1.0, -9999.0);
        let grid = parse_asc(&path);
        fs::remove_file(&path).unwrap();
        assert_eq!(grid.data, values);
        assert_eq!((grid.nrows, grid.ncols), (2, 3));
        assert_eq!((grid.xllcorner, grid.yllcorner, grid.cellsize, grid.nodata),
                   (10.0, 20.0, 1.0, -9999.0));
    }
}

#[derive(Clone, Debug)]
pub struct RasterGrid {
    pub ncols: usize,
    pub nrows: usize,
    pub affine: [f64; 6],
    pub crs: Option<gdal::spatial_ref::SpatialRef>,
}

#[derive(Debug)]
pub struct Raster<F: Real = f64> {
    pub grid: RasterGrid,
    /// Missing cells are normalised to NaN, independently of the file's sentinel.
    pub data: Vec<F>,
}

type RasterResult<T> = Result<T, Box<dyn std::error::Error>>;

fn raster_error(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into())
}

fn format(path: &Path) -> RasterResult<&'static str> {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("tif" | "tiff") => Ok("GTiff"),
        Some("asc") => Ok("AAIGrid"),
        _ => Err(raster_error(format!("{}: expected .tif, .tiff, or .asc", path.display())).into()),
    }
}

impl RasterGrid {
    fn validate(&self) -> RasterResult<()> {
        if self.nrows == 0 || self.ncols == 0 || self.nrows.checked_mul(self.ncols).is_none() {
            return Err(
                raster_error("raster dimensions must be positive and fit in memory").into(),
            );
        }
        let a = self.affine;
        if !a.iter().all(|v| v.is_finite())
            || a[1] <= 0.0
            || a[5] >= 0.0
            || a[2] != 0.0
            || a[4] != 0.0
            || (a[1] + a[5]).abs() > a[1] * 1e-9
        {
            return Err(raster_error("raster requires a north-up grid with square pixels").into());
        }
        if let Some(crs) = &self.crs {
            if !crs.is_projected() || (crs.linear_units() - 1.0).abs() > 1e-9 {
                return Err(
                    raster_error("raster requires a projected CRS with metre units").into(),
                );
            }
        }
        Ok(())
    }
}

pub fn require_aligned(
    reference: &RasterGrid,
    raster: &RasterGrid,
    path: &Path,
) -> RasterResult<()> {
    if reference.ncols != raster.ncols
        || reference.nrows != raster.nrows
        || reference.crs != raster.crs
        || reference
            .affine
            .iter()
            .zip(raster.affine)
            .any(|(&a, b)| (a - b).abs() > reference.affine[1] * 1e-9)
    {
        return Err(raster_error(format!(
            "{} is not aligned with the reference grid and CRS (including missing CRS)",
            path.display()
        ))
        .into());
    }
    Ok(())
}

pub fn read_raster(path: &Path) -> RasterResult<Raster> {
    read_raster_typed::<f64>(path)
}

pub fn read_raster_typed<F: Real + gdal::raster::GdalType>(path: &Path) -> RasterResult<Raster<F>> {
    let driver = format(path)?;
    let dataset = gdal::Dataset::open(path)
        .map_err(|e| raster_error(format!("cannot open raster {}: {e}", path.display())))?;
    if dataset.driver().short_name() != driver {
        return Err(raster_error(format!(
            "{}: file contents do not match its raster extension",
            path.display()
        ))
        .into());
    }
    if dataset.raster_count() != 1 {
        return Err(raster_error(format!(
            "{} must have exactly one raster band",
            path.display()
        ))
        .into());
    }
    let (ncols, nrows) = dataset.raster_size();
    let crs = if dataset.projection().is_empty() {
        None
    } else {
        Some(dataset.spatial_ref()?)
    };
    if driver == "GTiff" && crs.is_none() {
        return Err(raster_error(format!("{}: GeoTIFF has no CRS", path.display())).into());
    }
    let grid = RasterGrid {
        ncols,
        nrows,
        affine: dataset.geo_transform()?,
        crs,
    };
    grid.validate()
        .map_err(|e| raster_error(format!("{}: {e}", path.display())))?;
    let band = dataset.rasterband(1)?;
    let nodata = band.no_data_value().map(scalar::<F>);
    let mut data = if driver == "AAIGrid" {
        // GDAL may infer Float32 for ASCII grids. Parse the cells directly to
        // retain the requested precision, using GDAL for the grid metadata.
        let text = fs::read_to_string(path)?;
        let mut values = Vec::with_capacity(ncols * nrows);
        for line in text.lines() {
            let mut tokens = line.split_whitespace();
            let Some(first) = tokens.next() else {
                continue;
            };
            if matches!(
                first.to_ascii_lowercase().as_str(),
                "ncols"
                    | "nrows"
                    | "xllcorner"
                    | "yllcorner"
                    | "xllcenter"
                    | "yllcenter"
                    | "cellsize"
                    | "nodata_value"
            ) {
                continue;
            }
            for token in std::iter::once(first).chain(tokens) {
                values.push(token.parse::<F>().map_err(|_| {
                    raster_error(format!("{}: invalid raster cell {token}", path.display()))
                })?);
            }
        }
        if values.len() != nrows * ncols {
            return Err(raster_error(format!("{}: incomplete raster data", path.display())).into());
        }
        values
    } else {
        band.read_as::<F>((0, 0), (ncols, nrows), (ncols, nrows), None)?
            .data()
            .to_vec()
    };
    for value in &mut data {
        if !value.is_finite() || nodata == Some(*value) {
            *value = F::nan();
        }
    }
    Ok(Raster { grid, data })
}

/// Select the file format from the extension and preserve the array's precision.
pub fn write_raster<F: Real + gdal::raster::GdalType>(
    path: &Path,
    grid: &RasterGrid,
    values: &[F],
) -> RasterResult<()> {
    use gdal::{
        raster::{Buffer, RasterCreationOptions},
        DriverManager,
    };
    let driver = format(path)?;
    grid.validate()?;
    if values.len() != grid.nrows * grid.ncols {
        return Err(raster_error("raster data length does not match its grid").into());
    }
    if driver == "GTiff" && grid.crs.is_none() {
        return Err(raster_error("GeoTIFF output requires a CRS; use georeferenced inputs or explicitly select .asc for legacy grids").into());
    }
    let make_dataset = |driver_name: &str, filename: &Path| -> RasterResult<gdal::Dataset> {
        let mut dataset = DriverManager::get_driver_by_name(driver_name)?
            .create_with_band_type::<F, _>(filename, grid.ncols, grid.nrows, 1)?;
        dataset.set_geo_transform(&grid.affine)?;
        if let Some(crs) = &grid.crs {
            dataset.set_spatial_ref(crs)?;
        }
        Ok(dataset)
    };
    if driver == "GTiff" {
        let mut dataset = make_dataset("GTiff", path)?;
        let mut band = dataset.rasterband(1)?;
        band.set_no_data_value(Some(f64::NAN))?;
        band.write(
            (0, 0),
            (grid.ncols, grid.nrows),
            &mut Buffer::new((grid.ncols, grid.nrows), values.to_vec()),
        )?;
        dataset.flush_cache()?;
    } else {
        // AAIGrid has no Create implementation; create it from an in-memory band.
        let dataset = make_dataset("MEM", Path::new(""))?;
        let mut band = dataset.rasterband(1)?;
        band.set_no_data_value(Some(-9999.0))?;
        let values: Vec<F> = values
            .iter()
            .map(|&v| if v.is_finite() { v } else { scalar(-9999.0) })
            .collect();
        band.write(
            (0, 0),
            (grid.ncols, grid.nrows),
            &mut Buffer::new((grid.ncols, grid.nrows), values),
        )?;
        let options = RasterCreationOptions::from_iter(["SIGNIFICANT_DIGITS=17"]);
        dataset
            .create_copy(
                &DriverManager::get_driver_by_name("AAIGrid")?,
                path,
                &options,
            )?
            .flush_cache()?;
    }
    Ok(())
}

#[cfg(test)]
mod raster_file_tests {
    use super::*;
    use gdal::{raster::Buffer, spatial_ref::SpatialRef, DriverManager};

    fn path(extension: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "lmc-raster-{}-{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            extension
        ))
    }
    fn grid() -> RasterGrid {
        RasterGrid {
            nrows: 2,
            ncols: 3,
            affine: [100.125, 2.0, 0.0, 204.25, 0.0, -2.0],
            crs: Some(SpatialRef::from_epsg(27700).unwrap()),
        }
    }
    #[test]
    fn geotiff_and_explicit_ascii_preserve_values_grid_and_missing_cells() {
        let values = [
            0.0,
            1e-300,
            366.321540637836,
            f64::NAN,
            1.0,
            5.3454610306142845e-43,
        ];
        for extension in ["tif", "asc"] {
            let path = path(extension);
            write_raster(&path, &grid(), &values).unwrap();
            let actual = read_raster(&path).unwrap();
            assert_eq!(actual.grid.affine, grid().affine);
            assert_eq!(actual.grid.crs, grid().crs);
            assert_eq!((actual.grid.nrows, actual.grid.ncols), (2, 3));
            for (&actual, &expected) in actual.data.iter().zip(&values) {
                assert!(
                    actual == expected || actual.is_nan() && expected.is_nan(),
                    "{extension}: {actual} != {expected}"
                );
            }
            if extension == "tif" {
                assert_eq!(
                    gdal::Dataset::open(&path)
                        .unwrap()
                        .rasterband(1)
                        .unwrap()
                        .band_type(),
                    gdal::raster::GdalDataType::Float64
                );
            }
            fs::remove_file(&path).unwrap();
            if extension == "asc" {
                fs::remove_file(path.with_extension("prj")).unwrap();
            }
        }
        let path = path("tif");
        let values: [f32; 6] = [0.0, 1.0, f32::MIN_POSITIVE, f32::NAN, 3.0, 4.0];
        write_raster(&path, &grid(), &values).unwrap();
        let actual = read_raster_typed::<f32>(&path).unwrap();
        assert_eq!(actual.data[2], f32::MIN_POSITIVE);
        assert!(actual.data[3].is_nan());
        assert_eq!(
            gdal::Dataset::open(&path)
                .unwrap()
                .rasterband(1)
                .unwrap()
                .band_type(),
            gdal::raster::GdalDataType::Float32
        );
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn file_nodata_is_normalised_and_does_not_affect_alignment() {
        let mut grids = Vec::new();
        for nodata in [-7.0, 9999.0] {
            let path = path("tif");
            let mut ds = DriverManager::get_driver_by_name("GTiff")
                .unwrap()
                .create_with_band_type::<f64, _>(&path, 3, 2, 1)
                .unwrap();
            ds.set_geo_transform(&grid().affine).unwrap();
            ds.set_spatial_ref(grid().crs.as_ref().unwrap()).unwrap();
            let mut band = ds.rasterband(1).unwrap();
            band.set_no_data_value(Some(nodata)).unwrap();
            band.write(
                (0, 0),
                (3, 2),
                &mut Buffer::new((3, 2), vec![1.0, nodata, 2.0, 3.0, 4.0, 5.0]),
            )
            .unwrap();
            ds.flush_cache().unwrap();
            drop(ds);
            let raster = read_raster(&path).unwrap();
            assert!(raster.data[1].is_nan());
            grids.push(raster.grid);
            fs::remove_file(&path).unwrap();
        }
        require_aligned(&grids[0], &grids[1], Path::new("other.tif")).unwrap();
        let mut shifted = grid();
        shifted.affine[0] += 1.0;
        assert!(require_aligned(&grid(), &shifted, Path::new("shifted.tif")).is_err());
        let mut other_crs = grid();
        other_crs.crs = Some(SpatialRef::from_epsg(32630).unwrap());
        assert!(require_aligned(&grid(), &other_crs, Path::new("other.tif")).is_err());
        other_crs.crs = None;
        assert!(require_aligned(&grid(), &other_crs, Path::new("unknown.asc")).is_err());
    }
    #[test]
    fn invalid_files_and_output_dimensions_fail_explicitly() {
        let missing = path("tif");
        assert!(read_raster(&missing)
            .unwrap_err()
            .to_string()
            .contains(missing.to_str().unwrap()));
        assert!(read_raster(Path::new("unknown.xyz")).is_err());
        assert!(write_raster(&path("tif"), &grid(), &[1.0]).is_err());
        let malformed = path("asc");
        fs::write(
            &malformed,
            "ncols 3\nnrows 2\nxllcorner 0\nyllcorner 0\ncellsize 1\nNODATA_value -9999\n1 2\n",
        )
        .unwrap();
        assert!(read_raster(&malformed).is_err());
        fs::remove_file(malformed).unwrap();
    }
}
