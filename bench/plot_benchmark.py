#!/usr/bin/env python3
"""Plot benchmark time/memory trajectories for three solvers.

CSV schema (emitted by web/src/pages/Benchmark.jsx):
    resolution,repeat,run,prep_time_s,prep_mem_mb,conn_time_s,conn_mem_mb,total_iters

run is one of: jacobi, gmg, alcouffe.
Per row: total_time_s = prep_time_s + conn_time_s.
For each (resolution, run) we average across repeats, then draw one
trajectory per solver of mean total time vs resolution. A single
memory trajectory (jacobi peak conn_mem_mb) is plotted on a
secondary axis.
"""
import sys
import csv
from collections import defaultdict
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

RUNS = ('jacobi', 'gmg', 'alcouffe')
RUN_LABELS = {
    'jacobi':  'Jacobi CG μ (s)',
    'gmg':     'GMG CG μ (s)',
    'alcouffe':'Alcouffe CG μ (s)',
}
RUN_LINESTYLES = {
    'jacobi':  '-',
    'gmg':     '--',
    'alcouffe':':',
}
RUN_COLORS = {
    'jacobi':  '#444',
    'gmg':     '#666',
    'alcouffe':'#888',
}
MEM_COLOR = '#aaa'


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

    by_key = defaultdict(list)
    mem_by_res = defaultdict(list)
    for r in rows:
        try:
            res = int(r['resolution'])
            run = r.get('run', '')
            prep = float(r.get('prep_time_s', 0) or 0)
            conn = float(r.get('conn_time_s', 0) or 0)
            total = prep + conn
            by_key[(res, run)].append(total)
            if run == 'jacobi':
                mem_by_res[res].append(float(r.get('conn_mem_mb', 0) or 0))
        except (ValueError, KeyError):
            continue

    res_all = sorted({res for (res, _) in by_key})
    if not res_all:
        print('no data rows with valid runs')
        return

    mean_t = {r: [sum(by_key[(res, r)]) / len(by_key[(res, r)])
                   if by_key.get((res, r)) else float('nan')
                   for res in res_all] for r in RUNS}
    mem_mean = [sum(mem_by_res[r]) / len(mem_by_res[r]) if mem_by_res.get(r) else float('nan')
                for r in res_all]

    fig, ax1 = plt.subplots(figsize=(9, 5.5))
    ax2 = ax1.twinx()

    for run in RUNS:
        xs, ys = [], []
        for res in res_all:
            pts = by_key.get((res, run))
            for y in pts or []:
                xs.append(res); ys.append(y)
        ax1.scatter(xs, ys, c=RUN_COLORS[run], marker='x', alpha=0.55,
                    zorder=2)

    for run in RUNS:
        ax1.plot(res_all, mean_t[run], RUN_LINESTYLES[run], color=RUN_COLORS[run], lw=2,
                 ms=6, label=RUN_LABELS[run], zorder=3)

    ax2.scatter(res_all, mem_mean, c=MEM_COLOR, marker='s', alpha=0.7,
                zorder=2, label='jacobi peak memory (MB)')
    ax2.plot(res_all, mem_mean, '--', color=MEM_COLOR, lw=2, ms=7,
             label='jacobi peak memory μ (MB)', zorder=3)

    ax1.set_xlabel('Resolution (pixels)')
    ax1.set_ylabel('Total time (s)')
    ax2.set_ylabel('Peak memory (MB)')
    ax1.tick_params(axis='y')
    ax2.tick_params(axis='y')
    ax1.grid(True, alpha=0.3)

    all_t = [y for vals in mean_t.values() for y in vals if y == y]
    all_mem = [m for m in mem_mean if m == m]
    if all_t:
        ax1.set_ylim(0, max(all_t) * 1.25)
    if all_mem:
        ax2.set_ylim(0, max(all_mem) * 1.25)

    res2str = lambda x: f"{x}x{x}"
    ax1.set_xticks(res_all)
    ax1.set_xticklabels([res2str(r) for r in res_all])

    handles = [plt.Line2D([], [], color=RUN_COLORS[r], marker='o', ls=RUN_LINESTYLES[r], lw=2,
                          label=RUN_LABELS[r]) for r in RUNS]
    handles += [plt.Line2D([], [], color=MEM_COLOR, marker='s', ls='--', lw=2,
                           label='jacobi peak mem (MB)')]
    ax1.legend(handles=handles, loc='upper left')

    plt.tight_layout()
    plt.savefig(out_path, dpi=120)
    print(f'Saved {out_path}')


if __name__ == '__main__':
    infile = sys.argv[1] if len(sys.argv) > 1 else None
    out = infile.rsplit('.', 1)[0] + '.png' if infile else 'benchmark.png'
    plot(infile, out)
