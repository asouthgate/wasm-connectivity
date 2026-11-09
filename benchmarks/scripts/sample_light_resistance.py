#!/usr/bin/env python3
"""Run paper scenario 3 with sampled light-resistance maxima using the native Rust solver."""

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import csv
from pathlib import Path
import subprocess
import tempfile

import numpy as np
import rasterio


ROOT = Path(__file__).resolve().parents[2]
OTHER_LAYERS = ("road_res", "river_res", "landscape_res", "linear_res", "generic_res")


def load(path, reference=None):
    with rasterio.Env(AAIGRID_DATATYPE="Float64"), rasterio.open(path) as src:
        grid = (src.shape, src.transform, src.crs)
        if reference is not None:
            # ASC inputs may omit CRS metadata; their dimensions and transform must match.
            if grid[:2] != reference[:2] or (
                grid[2] is not None and reference[2] is not None and grid[2] != reference[2]
            ):
                raise ValueError(f"Raster does not align: {path}")
        values = src.read(1, masked=True).astype(np.float64).filled(np.nan)
    return values, grid


def write_asc(path, values, grid):
    (rows, cols), transform, _ = grid
    if transform.b != 0 or transform.d != 0 or transform.a <= 0 or transform.e != -transform.a:
        raise ValueError("ASC output requires a north-up grid with square pixels")
    with path.open("w") as stream:
        stream.write(
            f"ncols {cols}\nnrows {rows}\nxllcorner {transform.c:.17g}\n"
            f"yllcorner {transform.f + rows * transform.e:.17g}\n"
            f"cellsize {transform.a:.17g}\nNODATA_value -9999\n"
        )
        np.savetxt(stream, np.where(np.isfinite(values), values, -9999), fmt="%.17g")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--layers", type=Path, default=ROOT / "2000x2000-lit")
    parser.add_argument("--scenario", type=Path, default=ROOT / "benchmarks/scenarios/point_lights")
    parser.add_argument("--solver", type=Path, default=ROOT / "target/release/scenario-solve")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=100)
    parser.add_argument("--workers", type=int, default=16)
    parser.add_argument("--minimum", type=float, default=1e3)
    parser.add_argument("--maximum", type=float, default=1e8)
    parser.add_argument("--seed", type=int, default=0)
    args = parser.parse_args()
    if args.samples < 1 or args.workers < 1:
        parser.error("samples and workers must be positive")
    if not (np.isfinite(args.minimum) and np.isfinite(args.maximum)
            and 0 < args.minimum <= args.maximum):
        parser.error("require finite 0 < minimum <= maximum")
    if not args.solver.is_file():
        parser.error("solver missing; build with cargo build --release --features bin --bin scenario-solve")

    light, grid = load(args.layers / "lamp_res.tif")
    finite_light = light[np.isfinite(light)]
    if not finite_light.size or finite_light.min() < 0 or finite_light.max() <= 0:
        raise ValueError("Light resistance must be nonnegative with a positive maximum")
    current_max = float(finite_light.max())
    fixed = np.zeros_like(light)
    for name in OTHER_LAYERS:
        layer, _ = load(args.layers / f"{name}.tif", grid)
        if np.any(layer[np.isfinite(layer)] < 0):
            raise ValueError(f"Negative resistance in {name}")
        fixed += layer
    valid = np.isfinite(fixed) & np.isfinite(light)
    if not valid.any():
        raise ValueError("No valid resistance cells")
    source, _ = load(args.scenario / "source.asc", grid)
    ground, _ = load(args.scenario / "ground.asc", grid)
    for name, values in (("source", source), ("ground", ground)):
        if not np.any(valid & np.isfinite(values) & (values > 0)):
            raise ValueError(f"No valid {name} cells")
    light /= current_max
    fixed[~valid] = np.nan
    coefficients = 10.0 ** np.random.default_rng(args.seed).uniform(
        np.log10(args.minimum), np.log10(args.maximum), args.samples
    )
    # Refuse an existing directory to prevent overwriting an earlier experiment.
    args.output.mkdir(parents=True, exist_ok=False)
    solver = args.solver.resolve()
    with tempfile.TemporaryDirectory(prefix="light-samples-") as temporary:
        temporary = Path(temporary)
        source_path, ground_path = temporary / "source.asc", temporary / "ground.asc"
        write_asc(source_path, source, grid)
        write_asc(ground_path, ground, grid)

        def run(index, coefficient):
            print(f"Starting sample {index}: light maximum {coefficient:.8g}", flush=True)
            resistance = temporary / f"resistance_{index:03d}.asc"
            output = args.output / f"current_{index:03d}.asc"
            write_asc(resistance, fixed + coefficient * light, grid)
            try:
                with (args.output / f"solve_{index:03d}.log").open("w") as log:
                    subprocess.run([
                        str(solver), "--resistance", str(resistance),
                        "--source", str(source_path), "--ground", str(ground_path),
                        "--precision", "f64", "--tol", "1e-4", "--max-iter", "100000",
                        "--l0nu", "2", "--l1nu", "5", "--coarsenu", "25",
                        "--l0omega", "0.95", "--coarseomega", "0.4", "--l0postnu", "40",
                        "--output", str(output),
                    ], stdout=log, stderr=subprocess.STDOUT, check=True)
            except Exception:
                print(f"Failed sample {index}: see solve_{index:03d}.log", flush=True)
                raise
            finally:
                resistance.unlink()
            return index

        with (args.output / "samples.csv").open("w", newline="") as stream:
            writer = csv.writer(stream)
            writer.writerow(["sample", "light_maximum", "original_light_maximum", "seed", "current_map"])
            for index, coefficient in enumerate(coefficients):
                writer.writerow([index, coefficient, current_max, args.seed, f"current_{index:03d}.asc"])
        with ThreadPoolExecutor(max_workers=args.workers) as executor:
            futures = [executor.submit(run, i, value) for i, value in enumerate(coefficients)]
            try:
                for completed, future in enumerate(as_completed(futures), 1):
                    index = future.result()
                    print(f"[{completed}/{args.samples}] Finished sample {index}: light maximum {coefficients[index]:.8g}", flush=True)
            except BaseException:
                for future in futures:
                    future.cancel()
                raise


if __name__ == "__main__":
    main()
