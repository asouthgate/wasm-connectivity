#!/usr/bin/env python3
"""Plot elapsed runtime and peak RSS from mixed, f64 low-memory and Circuitscape traces."""

import argparse
from collections import defaultdict
import re
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np


LABELS = {"dsm": "Digital surface model", "linear_res": "Linear resistance", "point_lights": "Point lights"}
MARKER = re.compile(r"^scenario=(\S+) run=\d+[ \t]*$", re.MULTILINE)


def elapsed_seconds(value):
    parts = value.split(":")
    if len(parts) not in (2, 3):
        raise ValueError(f"invalid elapsed time: {value}")
    seconds = 0.0
    for part in parts:
        seconds = seconds * 60 + float(part)
    if not np.isfinite(seconds) or seconds < 0:
        raise ValueError(f"invalid elapsed time: {value}")
    return seconds


def parse_trace(path):
    parts = MARKER.split(Path(path).read_text())
    records = defaultdict(list)
    for scenario, block in zip(parts[1::2], parts[2::2]):
        status = re.search(r"Exit status:\s*(\d+)", block)
        elapsed = re.search(r"Elapsed \(wall clock\) time \(h:mm:ss or m:ss\):\s*(\S+)", block)
        rss = re.search(r"Maximum resident set size \(kbytes\):\s*(\d+)", block)
        if scenario in LABELS and status and status[1] == "0" and elapsed and rss:
            records[scenario].append((elapsed_seconds(elapsed[1]), float(rss[1]) / 1024))
    return {scenario: np.array(samples) for scenario, samples in records.items()}


def plot_comparison(mixed, rust, cs, output):
    fig, axes = plt.subplots(2, 1, figsize=(10, 9), sharex=True)
    scenarios = [scenario for scenario in LABELS
                 if scenario in mixed or scenario in rust or scenario in cs]
    x = np.arange(len(scenarios))
    width = 0.25
    for metric, (ax, ylabel) in enumerate(zip(axes, ("Runtime (s)", "Peak memory (MiB)"))):
        for offset, data, label, color in (
            (-width, cs, "Circuitscape 6.1.0 (f64)", "#e66101"),
            (0, rust, "Low-memory MG (f64)", "#5e3c99"),
            (width, mixed, "Low-memory MG (mixed precision)", "#2c7fb8"),
        ):
            present = [i for i, scenario in enumerate(scenarios) if scenario in data]
            if not present:
                continue
            samples = [data[scenarios[i]][:, metric] for i in present]
            ax.bar(x[present] + offset, [s.mean() for s in samples], width,
                   yerr=[s.std(ddof=1) if len(s) > 1 else 0 for s in samples], capsize=5, label=label,
                   facecolor="white", edgecolor=color, linewidth=2, zorder=3)
        ax.set_ylabel(ylabel)
        ax.set_ylim(bottom=0)
        ax.grid(axis="y", alpha=0.3)
        ax.set_axisbelow(True)
    axes[0].legend()
    axes[1].set_xticks(x, [LABELS[scenario] for scenario in scenarios])
    fig.tight_layout()
    fig.savefig(output, dpi=300)
    plt.close(fig)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mixed_trace")
    parser.add_argument("rust_trace")
    parser.add_argument("cs_trace")
    parser.add_argument("output", nargs="?", default="cs_vs_rust_lowmem.png")
    args = parser.parse_args()
    try:
        mixed = parse_trace(args.mixed_trace)
        cs = parse_trace(args.cs_trace)
        rust = parse_trace(args.rust_trace)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    if not mixed and not cs and not rust:
        parser.error("no completed successful runs found")
    plot_comparison(mixed, rust, cs, args.output)
    print(f"Saved {args.output}")


if __name__ == "__main__":
    main()
