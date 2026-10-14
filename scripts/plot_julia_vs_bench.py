#!/usr/bin/env python3
"""Compare Julia Circuitscape timings against the best WASM alcouffe solver.
Usage: plot_julia_vs_bench.py <juliabench.out> <benchmark.csv> <new_4t.out> <new_1t.out> [out.png]
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

JULIA_OLD_PATH = sys.argv[1]
CSV_PATH = sys.argv[2]
NEW_4T_PATH = sys.argv[3]
NEW_1T_PATH = sys.argv[4]
OUT_PATH = sys.argv[5] if len(sys.argv) > 5 else "julia_vs_bench.png"


def parse_julia_old(path):
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


def parse_julia_new(path):
    complete_jobs = []
    wall_clocks = []
    with open(path) as f:
        text = f.read()
    for m in re.finditer(r"complete job\s+\d+\s+([\d.]+)s", text):
        complete_jobs.append(float(m.group(1)))
    for m in re.finditer(
        r"Elapsed \(wall clock\) time \(h:mm:ss or m:ss\): (\d+):([\d.]+)", text
    ):
        minutes, seconds = float(m.group(1)), float(m.group(2))
        wall_clocks.append(minutes * 60 + seconds)
    return {
        "complete_job": np.array(complete_jobs),
        "wall_clock": np.array(wall_clocks),
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


CB_COLORS = [
    "#d1961f",  # grey
    "#444444", 
    "#444444",
    "#e66101",
    "#e66101", 
    "#5e3c99",  
    "#5e3c99",  
]


def main():
    julia_old = parse_julia_old(JULIA_OLD_PATH)
    julia_4t = parse_julia_new(NEW_4T_PATH)
    julia_1t = parse_julia_new(NEW_1T_PATH)

    rows = load_benchmark(CSV_PATH)
    best = find_best_solver(rows)
    bench_data = extract_solver_runs(rows, best)
    alcouffe_500 = interp_500(bench_data)

    precond_solve = julia_old["preconditioner"] + julia_old["solve"]

    print(f"Julia 5.11.2 -- {len(julia_old['complete'])} runs")
    print(f"  precond+solve mean = {np.mean(precond_solve):.2f}s ± {np.std(precond_solve):.2f}")
    print(f"  complete mean = {np.mean(julia_old['complete']):.2f}s ± {np.std(julia_old['complete']):.2f}")
    print(f"Julia 5.17.1 (4t) -- {len(julia_4t['complete_job'])} runs")
    print(f"  complete job mean = {np.mean(julia_4t['complete_job']):.2f}s ± {np.std(julia_4t['complete_job']):.2f}")
    print(f"  wall clock mean = {np.mean(julia_4t['wall_clock']):.2f}s ± {np.std(julia_4t['wall_clock']):.2f}")
    print(f"Julia 5.17.1 (1t) -- {len(julia_1t['complete_job'])} runs")
    print(f"  complete job mean = {np.mean(julia_1t['complete_job']):.2f}s ± {np.std(julia_1t['complete_job']):.2f}")
    print(f"  wall clock mean = {np.mean(julia_1t['wall_clock']):.2f}s ± {np.std(julia_1t['wall_clock']):.2f}")
    print(f"Best solver = {best}")
    print(f"  alcouffe 500x500 (interp) mean = {np.mean(alcouffe_500):.2f}s ± {np.std(alcouffe_500):.2f}")

    labels = [
        "WASM",
        "Circuitscape 5.11.2\nprecond + solve",
        "Circuitscape 5.11.2\ncomplete job",
        "Circuitscape 5.17.1 (4t)\ncomplete job",
        "Circuitscape 5.17.1 (4t)\nwall clock",
        "Circuitscape 5.17.1 (1t)\ncomplete job",
        "Circuitscape 5.17.1 (1t)\nwall clock",
    ]
    means = [
        np.mean(alcouffe_500),
        np.mean(precond_solve),
        np.mean(julia_old["complete"]),
        np.mean(julia_4t["complete_job"]),
        np.mean(julia_4t["wall_clock"]),
        np.mean(julia_1t["complete_job"]),
        np.mean(julia_1t["wall_clock"]),
    ]
    stds = [
        np.std(alcouffe_500),
        np.std(precond_solve),
        np.std(julia_old["complete"]),
        np.std(julia_4t["complete_job"]),
        np.std(julia_4t["wall_clock"]),
        np.std(julia_1t["complete_job"]),
        np.std(julia_1t["wall_clock"]),
    ]

    colors = [CB_COLORS[i % len(CB_COLORS)] for i in range(len(labels))]
    x = np.arange(len(labels))

    fig, ax = plt.subplots(figsize=(12, 7))
    bars = ax.bar(x, means, yerr=stds, capsize=6, facecolor='white', edgecolor="black", linewidth=2.0, zorder=3)
    for bar, col in zip(bars, CB_COLORS): 
#        bar.set_hatch("xxx")
        bar.set_edgecolor(col)   # Sets BOTH hatch line color and border color
    ax.set_xticks(x)
    ax.set_xticklabels(labels, fontsize=13, rotation=15, ha="right")
    ax.set_ylabel("Time (s)", fontsize=14)
    ax.set_yscale("log")
    ax.tick_params(axis='x', labelsize=13, labelrotation=45)
    ax.yaxis.set_major_locator(LogLocator(base=10, subs='all', numticks=10))
    ax.yaxis.set_minor_formatter(NullFormatter())
    ax.grid(axis="y", alpha=0.3, which="both")

    plt.tight_layout()
    plt.savefig(OUT_PATH, dpi=300)
    print(f"\nSaved {OUT_PATH}")


if __name__ == "__main__":
    main()
