#!/usr/bin/env python3
"""Plot Chudleigh benchmark summaries and saved 2000 x 2000 CG traces."""
import argparse
from collections import defaultdict
import csv
import math
from pathlib import Path
import re
from statistics import mean

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.ticker import MaxNLocator

RUN_STYLES = {
    "jacobi": ("Jacobi CG", "#444444", "-", "s"),
    "gmg": ("Explicit f64", "#e66101", "-", "x"),
    "gmg-lowmem": ("Low-memory f64", "#008450", "--", "o"),
    "gmg-lowmem-f32": ("Low-memory f32", "purple", ":", "v"),
    "gmg-mixedp-lowmem": ("Low-memory mixed", "purple", "-.", "^"),
}
TRACE_MODES = (
    ("gmg", "f64", 1e-4),
    ("gmg-lowmem", "f64", 1e-4),
    ("gmg-mixedp-lowmem", "mixed", 1e-3),
)
METRICS = (
    ("conn_time_s", "Time (s)"),
    ("conn_mem_mb", "WASM linear memory (MiB)"),
    ("total_iters", "CG iterations"),
)
CG_SAMPLE = re.compile(
    r"^PROFILE\s+(\d+)\s+CG\s+(CG initial|CG update|final post-sweep \d+/\d+)\s+"
    r"\S+\s+(\S+)", re.IGNORECASE,
)


def read_trace(lines, precision):
    # Mixed precision starts with x = 0 on this nonzero right-hand side.
    residuals = [1.0] if precision == "mixed" else []
    final_sample = None
    problems = []
    for line in lines:
        match = CG_SAMPLE.match(line)
        if match is None:
            if not line.startswith("PROFILE") and line.strip():
                problems.append(line.strip())
            continue
        iteration, phase, residual = match.groups()
        iteration = int(iteration)
        residual = float(residual)
        if not math.isfinite(residual) or residual < 0:
            raise ValueError(f"Invalid residual at CG iteration {iteration}")
        if phase.lower().startswith("final post-sweep"):
            sweep, sweep_count = map(int, phase.split()[-1].split("/"))
            if sweep == sweep_count:
                final_sample = (iteration, residual)
        else:
            if iteration != len(residuals):
                raise ValueError(f"Unexpected CG iteration {iteration}")
            residuals.append(residual)
    if len(residuals) < 2 or final_sample is None or final_sample[0] != len(residuals) - 1:
        raise ValueError("Incomplete CG trace" + (f": {'; '.join(problems)}" if problems else ""))
    errors = [line for line in problems if not line.startswith("Warning:")]
    if any(not line.startswith("error: solve did not meet tolerance:") for line in errors):
        raise ValueError("; ".join(errors))
    return residuals, final_sample[1], not errors


def load_benchmarks(csv_path):
    samples = defaultdict(list)
    with csv_path.open(newline="") as source:
        for row in csv.DictReader(source):
            run = row["run"]
            if run in RUN_STYLES:
                values = tuple(float(row[column]) for column, _ in METRICS)
                samples[int(row["resolution"]), run].append(values)
    if not samples:
        raise ValueError(f"No recognized benchmark runs in {csv_path}")
    return samples


def plot_resolution_metrics(axes, samples):
    resolutions = sorted({resolution for resolution, _ in samples})
    for index, (axis, (_, ylabel)) in enumerate(zip(axes, METRICS)):
        for run, (label, color, linestyle, marker) in RUN_STYLES.items():
            run_values = [
                [row[index] for row in samples.get((resolution, run), ())]
                for resolution in resolutions
            ]
            if not any(run_values):
                continue
            averages = [mean(values) if values else math.nan for values in run_values]
            lower = [average - min(values) if values else math.nan
                     for values, average in zip(run_values, averages)]
            upper = [max(values) - average if values else math.nan
                     for values, average in zip(run_values, averages)]
            axis.errorbar(resolutions, averages, yerr=[lower, upper], label=label,
                          color=color, linestyle=linestyle, marker=marker, capsize=4)
        axis.set_xticks(resolutions)
        axis.tick_params(axis="x", labelrotation=45)
        axis.set_xlabel("Raster width (pixels)", fontsize=15, labelpad=12)
        axis.set_ylabel(ylabel, fontsize=15, labelpad=12)
        if index < 2:
            axis.set_ylim(bottom=0)
    axes[2].yaxis.set_major_locator(MaxNLocator(integer=True))
    axes[0].legend(fontsize=12)


def plot_convergence(axis, trace_paths):
    for trace_path, (run, precision, tolerance) in zip(trace_paths, TRACE_MODES):
        label, color, linestyle, marker = RUN_STYLES[run]
        with trace_path.open() as trace:
            residuals, final_residual, converged = read_trace(trace, precision)
        iterations = len(residuals) - 1
        print(f"{label}: {iterations} iterations, final relative residual {final_residual:.6e}, "
              f"tolerance {tolerance:.0e} {'met' if converged else 'missed'}")
        axis.semilogy(residuals, linestyle=linestyle, color=color, label=label,
                      marker=marker, markersize=9 if marker == "o" else 5,
                      markerfacecolor="none" if marker == "o" else color,
                      nonpositive="mask")
        axis.plot(iterations, final_residual, marker="o", linestyle="none",
                  markerfacecolor="none", color=color)
    axis.plot([], [], "ko", markerfacecolor="none", label="After final smoothing")
    axis.set_xlabel("CG iteration", fontsize=15, labelpad=12)
    axis.set_ylabel("Relative residual", fontsize=15, labelpad=12)
    axis.xaxis.set_major_locator(MaxNLocator(integer=True))
    axis.legend(fontsize=12)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("benchmark_csv", type=Path)
    parser.add_argument("explicit_trace", type=Path)
    parser.add_argument("low_memory_trace", type=Path)
    parser.add_argument("mixed_trace", type=Path)
    parser.add_argument("--output", type=Path, required=True, help="Path for the plot")
    args = parser.parse_args()

    fig, axes = plt.subplots(2, 2, figsize=(12.8, 12.8), layout="constrained")
    time_axis, memory_axis, iterations_axis, convergence_axis = axes.flat
    plot_resolution_metrics((time_axis, memory_axis, iterations_axis),
                            load_benchmarks(args.benchmark_csv))
    plot_convergence(convergence_axis, (args.explicit_trace, args.low_memory_trace, args.mixed_trace))
    for panel_label, axis in zip(("a)", "b)", "c)", "d)"), axes.flat):
        axis.set_title(panel_label, loc="left", fontsize=15)
        axis.tick_params(labelsize=12)
        axis.grid(axis="y", alpha=0.3)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(args.output, dpi=120)
    print(f"Saved {args.output}")


if __name__ == "__main__":
    main()
