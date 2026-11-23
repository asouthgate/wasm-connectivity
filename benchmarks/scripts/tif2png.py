import sys
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
from matplotlib import pyplot as plt
from matplotlib.colors import LogNorm
from compare_rasters import read_current, rescale_current


def render_current(current, output_path):
    figure, axes = plt.subplots(figsize=(10, 8))
    axes.imshow(1 + 99 * current, cmap="inferno", norm=LogNorm(vmin=1, vmax=100))
    axes.set_axis_off()
    figure.savefig(output_path, dpi=300, bbox_inches="tight")
    plt.close(figure)


def main(circuitscape_path, rust_path, output_directory):
    circuitscape = rescale_current(read_current(circuitscape_path))
    rust = rescale_current(read_current(rust_path))
    output_directory = Path(output_directory)
    render_current(circuitscape, output_directory / "circuitscape_current.png")
    render_current(rust, output_directory / "rust_low_memory_current.png")


if __name__ == "__main__":
    main(*sys.argv[1:])
