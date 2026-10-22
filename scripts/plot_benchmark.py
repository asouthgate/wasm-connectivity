#!/usr/bin/env python3
"""Plot benchmark mean lines with minimum-to-maximum range bars, including mixed-precision CGMG.

CSV schema (emitted by example/src/pages/Benchmark.jsx):
    resolution,repeat,run,prep_time_s,prep_mem_mb,conn_time_s,conn_mem_mb,total_iters

run is one of: jacobi, gmg, gmg-lowmem, gmg-mixedp-lowmem.
Legacy two-solver CSVs also work.
Time uses conn_time_s only, including input transfer and output export.
Rasterization time (prep_time_s) is excluded.
"""
import sys
import csv
from collections import defaultdict
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

RUNS = ('jacobi', 'gmg', 'gmg-lowmem', 'gmg-mixedp-lowmem')
RUN_LABELS = {
    'jacobi':  'Jacobi CG',
    'gmg':     'CGMG',
    'gmg-lowmem': 'CGMG (low-memory)',
    'gmg-mixedp-lowmem': 'CGMG (mixed precision, low-memory)',
}
RUN_LINESTYLES = {
    'jacobi': '-',
    'gmg': '--',
    'gmg-lowmem': ':',
    'gmg-mixedp-lowmem': '-.',
}
RUN_COLORS = {
    'jacobi':  '#444444',
    'gmg':     '#e66101',
    'gmg-lowmem': '#2c7fb8',
    'gmg-mixedp-lowmem': 'purple',
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
                
            conn_t = float(r.get('conn_time_s', 0) or 0)
            time_by_key[(res, run)].append(conn_t)
            
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

    fig, (ax1, ax2) = plt.subplots(2, 1, figsize=(10, 9), sharex=True)

    active_runs = [run for run in RUNS
                   if any((res, run) in time_by_key for res in res_all)]
    for run in active_runs:
        for ax, means, samples in [(ax1, mean_t, time_by_key),
                                    (ax2, mean_m, mem_by_key)]:
            if not any((res, run) in samples for res in res_all):
                continue
            lower, upper = [], []
            for res, mean in zip(res_all, means[run]):
                values = samples.get((res, run))
                lower.append(max(0.0, mean - min(values)) if values else float('nan'))
                upper.append(max(0.0, max(values) - mean) if values else float('nan'))
            ax.errorbar(res_all, means[run], yerr=[lower, upper],
                        color=RUN_COLORS[run], label=RUN_LABELS[run],
                        linestyle=RUN_LINESTYLES[run], linewidth=2, capsize=4, zorder=3)

    # Formatter for labels (e.g., 1000x1000)
    res2str = lambda x: f"{x}x{x}"

    # Upper axes (time)
    ax1.set_ylabel('Time (s)')
    ax1.set_xticks(res_all)
    ax1.set_xticklabels([res2str(r) for r in res_all], rotation=45)
    ax1.grid(True, axis='y', alpha=0.3)
    
    all_t = [y for vals in time_by_key.values() for y in vals if y == y]
    if all_t:
        ax1.set_ylim(0, max(all_t) * 1.25)
    ax1.legend(loc='upper left')

    # Lower axes (memory)
    ax2.set_xlabel('Resolution (pixels)')
    ax2.set_ylabel('WASM linear memory (MiB)')
    ax2.set_xticks(res_all)
    ax2.set_xticklabels([res2str(r) for r in res_all], rotation=45)
    ax2.grid(True, axis='y', alpha=0.3)
    
    all_m = [y for vals in mem_by_key.values() for y in vals if y == y]
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
