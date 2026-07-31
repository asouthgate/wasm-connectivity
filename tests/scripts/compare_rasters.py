#!/usr/bin/env python3
"""Compare two rasters (asc or tif): correlation, scale, and difference stats.

Usage:
    python tests/scripts/compare_rasters.py <raster_a> <raster_b>

Example:
    python tests/scripts/compare_rasters.py results_curmap.tif tests/output/current_map_mg_alcouffe_neumann.asc
"""
import sys
import numpy as np
import rasterio
from scipy import stats


def load(path):
    with rasterio.open(path) as src:
        a = src.read(1).astype(np.float64).ravel()
        nodata = src.nodata
    if nodata is not None:
        a = np.where(a == nodata, np.nan, a)
    return a


def main():
    if len(sys.argv) != 3:
        sys.exit(f"usage: {sys.argv[0]} <raster_a> <raster_b>")

    a = load(sys.argv[1])
    b = load(sys.argv[2])
    if a.shape != b.shape:
        sys.exit(f"shape mismatch: {a.size} vs {b.size} pixels")

    mask = np.isfinite(a) & np.isfinite(b)
    a, b = a[mask], b[mask]
    print(f"Matched pixels: {mask.sum()} (excluded nodata: {(~mask).sum()})")

    diff = a - b
    print(f"\n{'':14s}{'A':>14s}{'B':>14s}")
    print(f"{'min':14s}{a.min():>14.6f}{b.min():>14.6f}")
    print(f"{'max':14s}{a.max():>14.6f}{b.max():>14.6f}")
    print(f"{'mean':14s}{a.mean():>14.6f}{b.mean():>14.6f}")

    print(f"\nDifference (A - B):")
    print(f"  max abs diff:  {np.abs(diff).max():.6f}")
    print(f"  mean abs diff: {np.abs(diff).mean():.6f}")
    print(f"  RMS diff:      {np.sqrt((diff ** 2).mean()):.6f}")

    pearson = np.corrcoef(a, b)[0, 1]
    spearman = stats.spearmanr(a, b)[0]
    # Best-fit scale A ≈ s·B through the origin
    scale = np.sum(a * b) / np.sum(b * b)
    print(f"\nCorrelation:")
    print(f"  Pearson:        {pearson:.6f}")
    print(f"  Spearman rank:  {spearman:.6f}")
    print(f"  Best-fit scale: {scale:.4f}  (A ≈ scale · B)")

    pos = (a > 0) & (b > 0)
    if pos.sum() > 10:
        la, lb = np.log1p(a[pos]), np.log1p(b[pos])
        log_corr = np.corrcoef(la, lb)[0, 1]
        ratio = a[pos] / b[pos]
        print(f"  Log-corr:       {log_corr:.6f}  (on {pos.sum()} positive pixels)")
        print(f"\nRatio A/B percentiles (positive pixels):")
        for p in (10, 25, 50, 75, 90):
            print(f"  p{p:<2d}: {np.percentile(ratio, p):.4f}")


if __name__ == "__main__":
    main()
