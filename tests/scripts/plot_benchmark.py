#!/usr/bin/env python3
"""Plot benchmark time and memory trajectories for three solvers.

CSV schema (emitted by web/src/pages/Benchmark.jsx):
    resolution,repeat,run,prep_time_s,prep_mem_mb,conn_time_s,conn_mem_mb,total_iters

run is one of: jacobi, gmg, alcouffe.
Per row: total_time_s = prep_time_s + conn_time_s.
"""
import sys
import csv
from collections import defaultdict
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

RUNS = ('jacobi', 'gmg', 'alcouffe')
RUN_LABELS = {
    'jacobi':  'Jacobi CG',
    'gmg':     'GMG CG',
    'alcouffe':'Alcouffe CG',
}
RUN_LINESTYLES = {
    'jacobi':  '-',
    'gmg':     '--',
    'alcouffe':':',
}
RUN_COLORS = {
    'jacobi':  '#444444',
    'gmg':     '#e66101',  # Distinct color scheme for clarity
    'alcouffe':'#5e3c99',
}


def load(filename=None):
    f = open(filename) if filename else sys.stdin
    rows = list(csv.DictReader(f))
    f.close()
    return rows


def plot(csv_path, out_path):
    rows = load(csv_path)
    if not rows:
        print('no rows')
        return

    # Track times and memory by (resolution, run)
    time_by_key = defaultdict(list)
    mem_by_key = defaultdict(list)
    
    for r in rows:
        try:
            res = int(r['resolution'])
            run = r.get('run', '')
            if run not in RUNS:
                continue
                
            prep_t = float(r.get('prep_time_s', 0) or 0)
            conn_t = float(r.get('conn_time_s', 0) or 0)
            total_t = prep_t + conn_t
            time_by_key[(res, run)].append(total_t)
            
            conn_m = float(r.get('conn_mem_mb', 0) or 0)
            mem_by_key[(res, run)].append(conn_m)
        except (ValueError, KeyError):
            continue

    res_all = sorted({res for (res, _) in time_by_key})
    if not res_all:
        print('no data rows with valid runs')
        return

    # Compute averages for each resolution and run type
    mean_t = {r: [sum(time_by_key[(res, r)]) / len(time_by_key[(res, r)])
                   if time_by_key.get((res, r)) else float('nan')
                   for res in res_all] for r in RUNS}
                   
    mean_m = {r: [sum(mem_by_key[(res, r)]) / len(mem_by_key[(res, r)])
                   if mem_by_key.get((res, r)) else float('nan')
                   for res in res_all] for r in RUNS}

    # Setup side-by-side plots (1 row, 2 columns)
    fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(14, 6))

    for run in RUNS:
        # Scatter individual data points
        xs, ys = [], []
        for res in res_all:
            pts = time_by_key.get((res, run))
            for y in pts or []:
                xs.append(res)
                ys.append(y)
        ax1.scatter(xs, ys, c=RUN_COLORS[run], marker='x', alpha=0.4, zorder=2)
        
        # Plot mean trendlines
        ax1.plot(res_all, mean_t[run], RUN_LINESTYLES[run], color=RUN_COLORS[run], 
                 lw=2, ms=6, label=f'{RUN_LABELS[run]}', zorder=3)

    for run in RUNS:
        # Scatter individual data points
        xs, ys = [], []
        for res in res_all:
            pts = mem_by_key.get((res, run))
            for y in pts or []:
                xs.append(res)
                ys.append(y)
        ax2.scatter(xs, ys, c=RUN_COLORS[run], marker='o', alpha=0.4, zorder=2)
        
        # Plot mean trendlines
        ax2.plot(res_all, mean_m[run], RUN_LINESTYLES[run], color=RUN_COLORS[run], 
                 lw=2, ms=6, label=f'{RUN_LABELS[run]}', zorder=3)

    # Formatter for labels (e.g., 1000x1000)
    res2str = lambda x: f"{x}x{x}"

    # Polish Left Axes (Time)
    ax1.set_xlabel('Resolution (pixels)')
    ax1.set_ylabel('Total time (s)')
    ax1.set_title('Execution Time Breakdown')
    ax1.set_xticks(res_all)
    ax1.set_xticklabels([res2str(r) for r in res_all])
    ax1.grid(True, alpha=0.3)
    
    all_t = [y for vals in mean_t.values() for y in vals if y == y]
    if all_t:
        ax1.set_ylim(0, max(all_t) * 1.25)
    ax1.legend(loc='upper left')

    # Polish Right Axes (Memory)
    ax2.set_xlabel('Resolution (pixels)')
    ax2.set_ylabel('Peak memory (MB)')
    ax2.set_title('Peak Connection Memory Usage')
    ax2.set_xticks(res_all)
    ax2.set_xticklabels([res2str(r) for r in res_all])
    ax2.grid(True, alpha=0.3)
    
    all_m = [y for vals in mean_m.values() for y in vals if y == y]
    if all_m:
        ax2.set_ylim(0, max(all_m) * 1.25)
    ax2.legend(loc='upper left')

    plt.tight_layout()
    plt.savefig(out_path, dpi=120)
    print(f'Saved {out_path}')


if __name__ == '__main__':
    infile = sys.argv[1] if len(sys.argv) > 1 else None
    out = infile.rsplit('.', 1)[0] + '.png' if infile else 'benchmark.png'
    plot(infile, out)