#!/usr/bin/env python3
"""Plot minimum, median, maximum, and current-rank standard deviation."""

import argparse
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.cm import ScalarMappable
from matplotlib.colors import Normalize
import numpy as np



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache", type=Path, required=True,
                        help="Processed plot_data.npz from postprocess_light_samples.py")
    parser.add_argument("--output", type=Path, required=True,
                        help="Output figure path")
    args = parser.parse_args()
    with np.load(args.cache, allow_pickle=False) as cached:
        coefficients = cached["coefficients"]
        indices = cached["indices"].tolist()
        selected = dict(zip(indices, cached["selected"]))
        std = cached["std"]
        sample_count = int(cached["sample_count"])
    print(f"Loaded plotting data from {args.cache}", flush=True)

    order = np.argsort(coefficients, kind="stable")
    representatives = [int(order[0]), int(order[len(order) // 2]), int(order[-1])]
    if any(index not in selected for index in representatives):
        parser.error("Cache lacks the median map; rerun postprocess_light_samples.py to rebuild it")
    for letter, index in zip("abc", representatives):
        print(f"({letter}) Light resistance maximum: {coefficients[index]:.12g}", flush=True)
    norm = Normalize(0, 100)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    fig, axes = plt.subplots(2, 2, figsize=(10, 10), layout="compressed")
    fig.get_layout_engine().set(hspace=0.02)
    current_axes = list(axes.flat)[:3]
    for letter, ax, index in zip("abc", current_axes, representatives):
        ax.imshow(np.ma.masked_invalid(selected[index]), cmap="jet", norm=norm, interpolation="nearest")
        ax.set_title(f"({letter})", loc="left", pad=14)
        fig.colorbar(ScalarMappable(norm=norm, cmap="jet"), ax=ax,
                     label="Percentile", shrink=0.8)
    std_norm = Normalize(0, float(np.nanmax(std)))
    axes[1, 1].imshow(np.ma.masked_invalid(std), cmap="coolwarm", norm=std_norm, interpolation="nearest")
    axes[1, 1].set_title("(d)", loc="left", pad=14)
    fig.colorbar(ScalarMappable(norm=std_norm, cmap="coolwarm"), ax=axes[1, 1],
                 label=r"$\sigma$", shrink=0.8)
    for ax in axes.flat:
        ax.set_axis_off()
    fig.savefig(args.output, dpi=300, bbox_inches="tight")
    plt.close(fig)
    print(f"Saved {args.output}", flush=True)


if __name__ == "__main__":
    main()
