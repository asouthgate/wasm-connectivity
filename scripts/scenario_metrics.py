#!/usr/bin/env python3
"""Report resistance, spatial contrast, and source/ground coverage metrics."""

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


def connected_regions(mask):
    """Label four-neighbour regions using overlapping runs in adjacent rows."""
    labels = np.zeros(mask.shape, dtype=np.int32)
    parents = [0]

    def root(label):
        while parents[label] != label:
            parents[label] = parents[parents[label]]
            label = parents[label]
        return label

    for row, selected in enumerate(mask):
        changes = np.diff(np.concatenate(([False], selected, [False])).astype(np.int8))
        for start, end in zip(np.flatnonzero(changes == 1), np.flatnonzero(changes == -1)):
            above = np.unique(labels[row - 1, start:end]) if row else []
            neighbours = [root(int(label)) for label in above if label]
            if neighbours:
                label = neighbours[0]
                for neighbour in neighbours[1:]:
                    parents[root(neighbour)] = root(label)
            else:
                label = len(parents)
                parents.append(label)
            labels[row, start:end] = label
    roots = np.array([root(label) for label in range(len(parents))], dtype=np.int32)
    return roots[labels]


def region_contrast(raw, valid):
    """Internal/boundary median conductance ratios for the five largest R<Q90 regions."""
    threshold = np.percentile(raw[valid], 90)
    labels = connected_regions(valid & (raw < threshold))
    sizes = np.bincount(labels.ravel())
    regions = np.flatnonzero(sizes[1:]) + 1
    largest = sorted(regions, key=lambda region: (-sizes[region], region))[:5]
    ratios = []
    for region in largest:
        internal, boundary = [], []
        for axis in (0, 1):
            if axis == 0:
                a, b = raw[:-1, :], raw[1:, :]
                la, lb = labels[:-1, :], labels[1:, :]
                pairs = valid[:-1, :] & valid[1:, :]
            else:
                a, b = raw[:, :-1], raw[:, 1:]
                la, lb = labels[:, :-1], labels[:, 1:]
                pairs = valid[:, :-1] & valid[:, 1:]
            inside = pairs & (la == region) & (lb == region)
            border = pairs & ((la == region) != (lb == region))
            internal.append(2.0 / (a[inside] + b[inside]))
            boundary.append(2.0 / (a[border] + b[border]))
        internal, boundary = np.concatenate(internal), np.concatenate(boundary)
        if internal.size and boundary.size:
            ratios.append(np.median(internal) / np.median(boundary))
    return (min(ratios), max(ratios)) if ratios else (np.nan, np.nan)


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
    region_min, region_max = region_contrast(raw, valid)
    return minimum, maximum, contrast, geary_c, source_pct, ground_pct, region_min, region_max


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
    print(f"{'Scenario':<16} {'Min':>12} {'Max':>12} {'Ratio (max/min)':>16} {geary_label:>14} {'Source (%)':>12} {'Ground (%)':>12} {'Region I/B min':>16} {'Region I/B max':>16}")
    for scenario, (minimum, maximum, contrast, geary_c, source_pct, ground_pct, region_min, region_max) in rows:
        geary_text = f"{geary_c:.6g}" if np.isfinite(geary_c) else "n/a"
        region_min_text = f"{region_min:.6g}" if np.isfinite(region_min) else "n/a"
        region_max_text = f"{region_max:.6g}" if np.isfinite(region_max) else "n/a"
        print(f"{scenario:<16} {minimum:>12.6g} {maximum:>12.6g} {contrast:>16.6g} {geary_text:>14} {source_pct:>12.6g} {ground_pct:>12.6g} {region_min_text:>16} {region_max_text:>16}")
    print("Region I/B: median internal / median boundary edge conductance; min/max across the five largest")
    print("four-neighbour regions with R strictly below its valid-pixel 90th percentile (ties excluded).")
    print("Edges to invalid pixels or outside the raster are excluded; regions missing either edge type are skipped.")


if __name__ == "__main__":
    main()
