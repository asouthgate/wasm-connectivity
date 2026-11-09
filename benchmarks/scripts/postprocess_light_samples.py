#!/usr/bin/env python3
"""Process sampled current maps into per-run percentiles and ensemble plotting arrays."""

import argparse
import csv
from pathlib import Path

import numpy as np
import rasterio


def read_map(path, reference=None):
    with rasterio.Env(AAIGRID_DATATYPE="Float64"), rasterio.open(path) as src:
        grid = (src.shape, src.transform, src.crs)
        if reference is not None and grid != reference:
            raise ValueError(f"Raster does not align: {path}")
        values = src.read(1, masked=True).astype(np.float64).filled(np.nan)
    if np.any(values[np.isfinite(values)] < 0):
        raise ValueError(f"Negative current in {path}")
    return values, grid


def quantile_scale(reference):
    """Continuous percentile mapping; ties receive their midpoint percentile."""
    valid = reference[np.isfinite(reference)]
    unique, counts = np.unique(valid, return_counts=True)
    first = np.cumsum(counts) - counts
    percentiles = ((first + (counts - 1) / 2) / (valid.size - 1) * 100
                   if valid.size > 1 else np.array([50.0]))

    def transform(values):
        return np.ma.masked_invalid(np.interp(values, unique, percentiles))

    return transform


def prepare_data(directory):
    with (directory / "samples.csv").open(newline="") as stream:
        samples = list(csv.DictReader(stream))
    if len(samples) < 3:
        raise ValueError("At least three samples are required")
    coefficients = np.array([float(sample["light_maximum"]) for sample in samples])
    if not np.all(np.isfinite(coefficients) & (coefficients > 0)):
        raise ValueError("Light maxima must be finite and positive")
    missing = [row["current_map"] for row in samples
               if not (directory / row["current_map"]).is_file()]
    if missing:
        raise ValueError(f"{len(missing)} current maps missing; wait for sampling to finish")
    order = np.argsort(coefficients, kind="stable")
    indices = [int(order[0]), int(order[len(order) // 2]), int(order[-1])]
    if len(set(indices)) != 3:
        raise ValueError("Samples do not provide three distinct representatives across the log range")
    selected = {}
    mean = m2 = mask = grid = None
    for count, row in enumerate(samples, 1):
        values, grid = read_map(directory / row["current_map"], grid)
        finite = np.isfinite(values)
        if mask is None:
            mask = finite
            if not mask.any():
                raise ValueError("No valid current cells")
            mean = np.zeros(values.shape)
            m2 = np.zeros(values.shape)
        elif not np.array_equal(mask, finite):
            raise ValueError(f"Valid-cell mask differs in {row['current_map']}")
        # Rank each run independently before accumulating population moments.
        values = quantile_scale(values)(values).filled(np.nan)
        # Incremental updatest to the avg current map
        delta = values[mask] - mean[mask]
        mean[mask] += delta / count
        m2[mask] += delta * (values[mask] - mean[mask])
        if count - 1 in indices:
            selected[count - 1] = values
        print(f"Read {count}/{len(samples)}: {row['current_map']}", flush=True)
    mean[~mask] = np.nan
    std = np.full(mean.shape, np.nan)
    std[mask] = np.sqrt(np.maximum(m2[mask] / len(samples), 0))

    return coefficients, indices, selected, mean, std, len(samples)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--output", type=Path, help="Defaults to DIRECTORY/plot_data.npz")
    args = parser.parse_args()
    coefficients, indices, selected, mean, std, sample_count = prepare_data(args.directory)
    output = args.output if args.output is not None else args.directory / "plot_data.npz"
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("wb") as stream:
        np.savez(stream, coefficients=coefficients, indices=indices,
                 selected=np.stack([selected[i] for i in indices]),
                 mean=mean, std=std, sample_count=sample_count)
    print(f"Saved plotting data to {output}", flush=True)


if __name__ == "__main__":
    main()
