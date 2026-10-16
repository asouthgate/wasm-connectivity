#!/usr/bin/env python3
"""Report resistance min/max/ratio, Geary's C, and source/ground coverage."""

from pathlib import Path
import sys

import numpy as np
import rasterio


SCENARIOS = ("dsm", "linear_res", "point_lights")
INPUTS = Path(__file__).resolve().parent / "scenarios"


def load(path):
    if not path.is_file():
        raise ValueError(f"Missing {path}; run make build first")
    with rasterio.Env(AAIGRID_DATATYPE="Float64"), rasterio.open(path) as src:
        return src.read(1, masked=True).astype(np.float64)


def positive(data):
    return ~np.ma.getmaskarray(data) & np.isfinite(data.data) & (data.data > 0)


def metrics(resistance, source, ground):
    if resistance.shape != source.shape or resistance.shape != ground.shape:
        raise ValueError("Raster dimensions must match")
    valid = positive(resistance)
    count = np.count_nonzero(valid)
    if not count:
        raise ValueError("No conductive cells")
    values = resistance.data[valid]
    minimum, maximum = values.min(), values.max()
    contrast = maximum / minimum
    raw = np.zeros(resistance.shape)
    raw[valid] = values
    sum_squared_deviations = np.sum((values - values.mean()) ** 2)
    squared_difference = 0.0
    edges = 0
    for axis in (0, 1):
        pairs = (valid[:-1, :] & valid[1:, :]) if axis == 0 else (valid[:, :-1] & valid[:, 1:])
        differences = np.diff(raw, axis=axis)[pairs]
        squared_difference += np.sum(differences ** 2)
        edges += differences.size
    # Geary (1954), doi:10.2307/2986645. Binary symmetric four-neighbour
    # weights; each undirected edge is counted once in this expression.
    geary_c = ((count - 1) * squared_difference / (2 * edges * sum_squared_deviations)
               if edges and sum_squared_deviations > 0 else np.nan)
    source_pct = 100 * np.count_nonzero(positive(source) & valid) / count
    ground_pct = 100 * np.count_nonzero(positive(ground) & valid) / count
    return minimum, maximum, contrast, geary_c, source_pct, ground_pct


def main():
    rows = []
    for scenario in SCENARIOS:
        directory = INPUTS / scenario
        try:
            values = metrics(*(load(directory / f"{name}.asc")
                               for name in ("resistance", "source", "ground")))
        except (ValueError, rasterio.errors.RasterioError) as error:
            sys.exit(f"{scenario}: {error}")
        rows.append((scenario, values))
    geary_label = "Geary's C"
    print(f"{'Scenario':<16} {'Min':>12} {'Max':>12} {'Ratio (max/min)':>16} {geary_label:>14} {'Source (%)':>12} {'Ground (%)':>12}")
    for scenario, (minimum, maximum, contrast, geary_c, source_pct, ground_pct) in rows:
        geary_text = f"{geary_c:.6g}" if np.isfinite(geary_c) else "n/a"
        print(f"{scenario:<16} {minimum:>12.6g} {maximum:>12.6g} {contrast:>16.6g} {geary_text:>14} {source_pct:>12.6g} {ground_pct:>12.6g}")


if __name__ == "__main__":
    main()
