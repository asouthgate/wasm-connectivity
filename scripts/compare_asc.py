#!/usr/bin/env python3
"""Report absolute current-map errors, normalized by maximum Circuitscape current."""

import sys
import numpy as np
import rasterio

def load(path):
    with rasterio.Env(AAIGRID_DATATYPE='Float64'), rasterio.open(path) as src:
        data = src.read(1, masked=True).astype(np.float64)
    valid = ~np.ma.getmaskarray(data) & np.isfinite(data.data)
    return data.data, valid

if len(sys.argv) != 3:
    sys.exit("Usage: compare_asc.py <circuitscape.asc> <rust.asc>")

cs, cs_valid = load(sys.argv[1])
rust, rust_valid = load(sys.argv[2])
if cs.shape != rust.shape:
    sys.exit('Current maps have different dimensions')
if not np.array_equal(cs_valid, rust_valid):
    sys.exit('Current maps have different nodata masks')
if not cs_valid.any():
    sys.exit('Current maps have no valid cells')

error = np.abs(cs[cs_valid] - rust[cs_valid])
maximum = error.max()
mean = error.mean()
scale = np.abs(cs[cs_valid]).max()

def proportional(value):
    return value / scale * 100 if scale > 0 else (0.0 if value == 0 else float('inf'))

print(f'Current-map errors: {sys.argv[1]} vs {sys.argv[2]}')
print(f'  Max absolute error: {maximum:.8g}')
print(f'  Mean absolute error: {mean:.8g}')
print(f'  Normalized max absolute error: {proportional(maximum):.8g}%')
print(f'  Normalized mean absolute error: {proportional(mean):.8g}%')
