#!/usr/bin/env python3
"""Compare Julia Circuitscape timings (juliabench.out) against the best
solver from benchmark-2026-07-07.csv as a side-by-side bar chart.
"""
import sys
import csv
import re
from collections import defaultdict
import numpy as np
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.ticker import LogLocator, NullFormatter

JULIA_PATH = sys.argv[1]
CSV_PATH = sys.argv[2]
OUT_PATH = sys.argv[3] if len(sys.argv) > 3 else "julia_vs_bench.png"


def parse_julia(path):
    preconds = []
    solves = []
    completes = []
    with open(path) as f:
        text = f.read()
    for m in re.finditer(r"Time taken to construct preconditioner = ([\d.]+)", text):
        preconds.append(float(m.group(1)))
    for m in re.finditer(r"Time taken to solve linear system = ([\d.]+)", text):
        solves.append(float(m.group(1)))
    for m in re.finditer(r"Time taken to complete job = ([\d.]+)", text):
        completes.append(float(m.group(1)))
    return {
        "preconditioner": np.array(preconds),
        "solve": np.array(solves),
        "complete": np.array(completes),
    }


def load_benchmark(path):
    with open(path) as f:
        rows = list(csv.DictReader(f))
    return rows


def find_best_solver(rows):
    solver_totals = defaultdict(list)
    for r in rows:
        run = r["run"]
        prep = float(r["prep_time_s"])
        conn = float(r["conn_time_s"])
        solver_totals[run].append(prep + conn)
    best = None
    best_mean = float("inf")
    for solver, vals in solver_totals.items():
        m = np.mean(vals)
        if m < best_mean:
            best_mean = m
            best = solver
    return best


def extract_solver_runs(rows, solver):
    data = defaultdict(list)
    for r in rows:
        if r["run"] != solver:
            continue
        res = int(r["resolution"])
        prep = float(r["prep_time_s"])
        conn = float(r["conn_time_s"])
        data[res].append((prep, conn, prep + conn))
    return data


def interp_500(data):
    """Linearly interpolate total time for 500x500 from 400 and 600 alcouffe runs."""
    runs_400 = [t[2] for t in data[400]]
    runs_600 = [t[2] for t in data[600]]
    frac = (500 - 400) / (600 - 400)
    interp = [a + frac * (b - a) for a, b in zip(runs_400, runs_600)]
    return np.array(interp)


def main():

    julia = parse_julia(JULIA_PATH)
    rows = load_benchmark(CSV_PATH)
    best = find_best_solver(rows)
    bench_data = extract_solver_runs(rows, best)
    alcouffe_500 = interp_500(bench_data)

    precond_solve = julia["preconditioner"] + julia["solve"]

    print(f"Julia  -- {len(julia['complete'])} runs")
    print(f"  preconditioner+solve mean = {np.mean(precond_solve):.2f}s ± {np.std(precond_solve):.2f}")
    print(f"  complete mean = {np.mean(julia['complete']):.2f}s ± {np.std(julia['complete']):.2f}")
    print(f"Best solver = {best}")
    print(f"  alcouffe 500x500 (interp) mean = {np.mean(alcouffe_500):.2f}s ± {np.std(alcouffe_500):.2f}")

    # Build bar chart
    labels = [
        "Circuitscape\n(precond + solve)",
        "Circuitscape\n(complete)",
        "WASM\n(Alcouffe)",
    ]
    means = [np.mean(precond_solve),
             np.mean(julia["complete"]),
             np.mean(alcouffe_500)]
    stds  = [np.std(precond_solve),
             np.std(julia["complete"]),
             np.std(alcouffe_500)]

    hatches = ["//", "\\\\", "xx"]
    x = np.arange(len(labels))

    fig, ax = plt.subplots(figsize=(8, 6))
    bars = ax.bar(x, means, yerr=stds, capsize=6, color="white", edgecolor="black")
    for bar, hatch in zip(bars, hatches):
        bar.set_hatch(hatch)
    ax.set_xticks(x)
    ax.set_xticklabels(labels)
    ax.set_ylabel("Time (s)")
    ax.set_yscale("log")
    ax.yaxis.set_major_locator(LogLocator(base=10, subs='all', numticks=10))
    ax.yaxis.set_minor_formatter(NullFormatter())
    ax.grid(axis="y", alpha=0.3, which="both")

    plt.tight_layout()
    plt.savefig(OUT_PATH, dpi=300)
    print(f"\nSaved {OUT_PATH}")


if __name__ == "__main__":
    main()
