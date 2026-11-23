#!/usr/bin/env python3
"""Compare two rasters (asc or tif): correlation, scale, and difference stats.

Usage:
    python benchmarks/scripts/compare_rasters.py <raster_a> <raster_b>

Example:
    python benchmarks/scripts/compare_rasters.py results_curmap.tif tests/output/current_map_mg_alcouffe_shunt.asc
"""
import argparse
import numpy as np
import rasterio


def read_current(path):
    with rasterio.Env(AAIGRID_DATATYPE="Float64"), rasterio.open(path) as raster:
        return np.ma.masked_invalid(raster.read(1, masked=True).astype(np.float64))


def rescale_current(current):
    if current.count() == 0 or current.min() == current.max():
        raise ValueError("Rescaling requires at least two distinct current values")
    minimum = current.min()
    return (current - minimum) / (current.max() - minimum)


def main():
    from scipy import stats

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference")
    parser.add_argument("comparison")
    parser.add_argument("--strict-nodata", action="store_true")
    args = parser.parse_args()

    reference = read_current(args.reference)
    comparison = read_current(args.comparison)
    if reference.shape != comparison.shape:
        parser.error(f"Shape mismatch: {reference.shape} vs {comparison.shape}")
    reference_mask = np.ma.getmaskarray(reference)
    comparison_mask = np.ma.getmaskarray(comparison)
    if args.strict_nodata and not np.array_equal(reference_mask, comparison_mask):
        parser.error("Current maps have different NoData masks")
    mask = ~(reference_mask | comparison_mask)
    if not mask.any():
        parser.error("Current maps have no overlapping valid cells")
    a, b = reference.data[mask], comparison.data[mask]
    print(f"Matched pixels: {mask.sum()} (excluded nodata: {(~mask).sum()})")

    diff = a - b
    print(f"\n{'':14s}{'A':>14s}{'B':>14s}")
    print(f"{'min':14s}{a.min():>14.6f}{b.min():>14.6f}")
    print(f"{'max':14s}{a.max():>14.6f}{b.max():>14.6f}")
    print(f"{'mean':14s}{a.mean():>14.6f}{b.mean():>14.6f}")

    print(f"\nDifference (A - B):")
    maximum_error = np.abs(diff).max()
    mean_error = np.abs(diff).mean()
    print(f"  Max absolute difference: {maximum_error:.8g} raw units")
    print(f"  Mean absolute difference: {mean_error:.8g} raw units")
    reference_peak = np.ma.abs(reference).max()
    for label, error in (("max", maximum_error), ("mean", mean_error)):
        percent = 100 * error / reference_peak if reference_peak else (0.0 if error == 0 else np.inf)
        print(f"  Normalized {label} absolute error: {percent:.8g}%")
    try:
        rescaled_reference = rescale_current(reference)
        rescaled_comparison = rescale_current(comparison)
    except ValueError as error:
        print(f"  Rescaled difference unavailable: {error}")
    else:
        rescaled_error = np.ma.abs(rescaled_reference - rescaled_comparison)
        print(f"  Max absolute difference: {rescaled_error.max():.8g} rescaled units")
        print(f"  Mean absolute difference: {rescaled_error.mean():.8g} rescaled units")
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
