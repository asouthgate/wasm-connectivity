#!/usr/bin/env python3
"""Plot elapsed runtime and peak RSS from the two scenario comparison traces."""

import argparse
import re
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np


SCENARIOS = ("dsm", "linear_res", "point_lights")
LABELS = ("Digital surface model", "Linear resistance", "Point lights")
MARKER = re.compile(r"^scenario=(\S+) run=(\d+)\s*$", re.MULTILINE)


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
    text = Path(path).read_text()
    markers = list(MARKER.finditer(text))
    records = {}
    for index, marker in enumerate(markers):
        key = (marker[1], int(marker[2]))
        if key[0] not in SCENARIOS or key[1] not in range(1, 6):
            raise ValueError(f"{path}: unexpected run {key}")
        if key in records:
            raise ValueError(f"{path}: duplicate run {key}")
        end = markers[index + 1].start() if index + 1 < len(markers) else len(text)
        block = text[marker.end():end]

        def field(pattern):
            values = re.findall(pattern, block, re.MULTILINE)
            if len(values) != 1:
                raise ValueError(f"{path}: missing or repeated time field in {key}")
            return values[0]

        status = field(r"^\s*Exit status:\s*(\d+)\s*$")
        if status != "0" or "Command terminated by signal" in block:
            raise ValueError(f"{path}: failed run {key}")
        elapsed = elapsed_seconds(field(
            r"^\s*Elapsed \(wall clock\) time \(h:mm:ss or m:ss\):\s*(\S+)\s*$"
        ))
        rss = float(field(r"^\s*Maximum resident set size \(kbytes\):\s*(\d+)\s*$"))
        if rss <= 0:
            raise ValueError(f"{path}: invalid peak RSS in {key}")
        records[key] = (elapsed, rss / 1024)

    expected = {(scenario, run) for scenario in SCENARIOS for run in range(1, 6)}
    missing = expected - records.keys()
    if missing:
        raise ValueError(f"{path}: missing runs {sorted(missing)}")
    return {
        scenario: np.array([records[scenario, run] for run in range(1, 6)])
        for scenario in SCENARIOS
    }


def plot_comparison(cs, rust, output):
    fig, axes = plt.subplots(2, 1, figsize=(10, 9), sharex=True)
    x = np.arange(len(SCENARIOS))
    width = 0.35
    for metric, (ax, ylabel) in enumerate(zip(axes, ("Runtime (s)", "Peak memory (MiB)"))):
        for offset, data, label, color in (
            (-width / 2, cs, "Circuitscape 6.1.0", "#e66101"),
            (width / 2, rust, "Rust low-memory MG", "#5e3c99"),
        ):
            samples = np.array([data[scenario][:, metric] for scenario in SCENARIOS])
            ax.bar(x + offset, samples.mean(axis=1), width,
                   yerr=samples.std(axis=1, ddof=1), capsize=5, label=label,
                   facecolor="white", edgecolor=color, linewidth=2, zorder=3)
        ax.set_ylabel(ylabel)
        ax.set_ylim(bottom=0)
        ax.grid(axis="y", alpha=0.3)
        ax.set_axisbelow(True)
    axes[0].legend()
    axes[1].set_xticks(x, LABELS)
    fig.tight_layout()
    fig.savefig(output, dpi=300)
    plt.close(fig)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cs_trace")
    parser.add_argument("rust_trace")
    parser.add_argument("output", nargs="?", default="cs_vs_rust_lowmem.png")
    args = parser.parse_args()
    try:
        cs = parse_trace(args.cs_trace)
        rust = parse_trace(args.rust_trace)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    plot_comparison(cs, rust, args.output)
    print(f"Saved {args.output}")


if __name__ == "__main__":
    main()
