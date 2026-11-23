import argparse
from pathlib import Path

import matplotlib
import numpy as np

matplotlib.use("Agg")
from matplotlib import pyplot as plt
from matplotlib.colors import LogNorm, Normalize
from compare_rasters import read_current, rescale_current


def current_percentiles(current):
    valid = ~np.ma.getmaskarray(current)
    values = current.data[valid]
    if values.size == 0:
        raise ValueError("Current map has no valid cells")

    _, indices, counts = np.unique(values, return_inverse=True, return_counts=True)
    midpoint_ranks = np.cumsum(counts) - counts / 2
    percentiles = np.ma.masked_all(current.shape, dtype=float)
    percentiles[valid] = 100 * midpoint_ranks[indices] / values.size
    return percentiles


def render_current(current, output_path, quantile, colorbar):
    if quantile:
        plot_values = current_percentiles(current)
        normalization = Normalize(vmin=0, vmax=100)
    else:
        plot_values = 1 + 99 * rescale_current(current)
        normalization = LogNorm(vmin=1, vmax=100)

    figure, axes = plt.subplots(figsize=(10, 8))
    image = axes.imshow(plot_values, cmap="jet", norm=normalization)
    if colorbar:
        if quantile:
            color_scale = figure.colorbar(image, ax=axes, ticks=range(0, 101, 20))
            color_scale.set_label("Current percentile", fontsize=24)
        else:
            color_scale = figure.colorbar(image, ax=axes)
        color_scale.ax.tick_params(labelsize=22)
    axes.set_axis_off()
    figure.savefig(output_path, dpi=300, bbox_inches="tight")
    plt.close(figure)


def main():
    parser = argparse.ArgumentParser(description="Render two current rasters as PNGs.")
    parser.add_argument("circuitscape_path")
    parser.add_argument("rust_path")
    parser.add_argument("output_directory", type=Path)
    parser.add_argument("--quantile", action="store_true", help="Color by within-map percentile")
    parser.add_argument("--colorbar", action="store_true", help="Show a colorbar on each panel")
    arguments = parser.parse_args()

    for raster_path, filename in (
        (arguments.circuitscape_path, "circuitscape_current.png"),
        (arguments.rust_path, "rust_low_memory_current.png"),
    ):
        render_current(
            read_current(raster_path),
            arguments.output_directory / filename,
            arguments.quantile,
            arguments.colorbar,
        )


if __name__ == "__main__":
    main()
