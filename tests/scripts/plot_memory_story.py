#!/usr/bin/env python3
"""Plot the analytical memory story emitted by the `mem-story` binary.

Input is NDJSON, one JSON object per solve:
    {"resolution": N, "solver": "jacobi"|"mg", "ground": "...",
     "total_iters": K, "story": <MemoryStory>}

Produces two figures:
  1. Hierarchy breakdown: stacked bytes per multigrid level (Laplacian L_l,
     prolongation P_l, workspace e/d/d') for the largest resolution of each
     solver/ground pair.
  2. Scaling: total hierarchy bytes vs resolution across solvers.

Usage:
    cargo run --features bin,memory-story --bin mem-story -- 500 --solver all \
        > mem.jsonl
    python3 tests/scripts/plot_memory_story.py mem.jsonl
"""
import sys
import json
from collections import defaultdict

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

SOLVER_COLORS = {'mg': '#e66101', 'jacobi': '#444444'}


def load(filename=None):
    f = open(filename) if filename else sys.stdin
    rows = [json.loads(line) for line in f if line.strip()]
    f.close()
    return rows


def total_level_bytes(level):
    return (level['laplacian_bytes']
            + level['prolongation_p_bytes']
            + level['cholesky_factor_bytes']
            + level['workspace_e_bytes']
            + level['workspace_d_bytes']
            + level['workspace_d_prime_bytes'])


def plot_hierarchy(rows, out_prefix):
    # Group by (solver, ground), keep the largest-resolution story per group.
    groups = defaultdict(list)
    for r in rows:
        story = r.get('story')
        if not story or not story.get('hierarchy'):
            continue
        groups[(r['solver'], r['ground'])].append(r)

    if not groups:
        print('no hierarchy stories found')
        return

    for (solver, ground), recs in groups.items():
        best = max(recs, key=lambda r: r['resolution'])
        res = best['resolution']
        story = best['story']
        levels = story['hierarchy']
        lv = [lvl['level'] for lvl in levels]
        lap = [lvl['laplacian_bytes'] / 1e6 for lvl in levels]
        prol = [lvl['prolongation_p_bytes'] / 1e6 for lvl in levels]
        work = [(lvl['workspace_e_bytes'] + lvl['workspace_d_bytes']
                 + lvl['workspace_d_prime_bytes']) / 1e6 for lvl in levels]
        chol = [lvl['cholesky_factor_bytes'] / 1e6 for lvl in levels]

        fig, ax = plt.subplots(figsize=(8, 5))
        ax.bar(lv, lap, label='Laplacian $L_l$', color='#555555')
        ax.bar(lv, prol, bottom=lap, label='Prolongation $P_l$',
               color='#e66101')
        ax.bar(lv, work, bottom=[a + b for a, b in zip(lap, prol)],
               label='Workspace $e_l, d_l, d\'_l$', color='#f1c40f')
        ax.bar(lv, chol,
               bottom=[a + b + c for a, b, c in zip(lap, prol, work)],
               label='Cholesky factor', color='#2c7fb8')

        ax.set_xlabel('Level $l$ (0 = finest)')
        ax.set_ylabel('Bytes (MB)')
        ax.set_title(f'Multigrid hierarchy memory — {solver} / {ground} '
                     f'({res}x{res})')
        ax.set_xticks(lv)
        ax.grid(True, axis='y', alpha=0.3)
        ax.legend(loc='upper right')

        plt.tight_layout()
        out = f'{out_prefix}_{solver}_{ground}.png'
        plt.savefig(out, dpi=120)
        print(f'Saved {out}')
        plt.close(fig)


def plot_scaling(rows, out_path):
    reses = sorted({r['resolution'] for r in rows
                    if r.get('story') and r['story'].get('hierarchy')})
    if len(reses) < 2:
        print('single resolution; skipping scaling plot')
        return

    fig, ax = plt.subplots(figsize=(7, 5))
    for solver in ('jacobi', 'mg'):
        pts = []
        for r in rows:
            if r['solver'] != solver:
                continue
            story = r.get('story')
            if not story or not story.get('hierarchy'):
                continue
            total = sum(total_level_bytes(l) for l in story['hierarchy'])
            pts.append((r['resolution'], total / 1e6))
        if not pts:
            continue
        pts.sort()
        xs = [p[0] for p in pts]
        ys = [p[1] for p in pts]
        ax.plot(xs, ys, marker='o', label=solver,
                color=SOLVER_COLORS.get(solver, 'k'))

    ax.set_xlabel('Resolution (pixels)')
    ax.set_ylabel('Total hierarchy bytes (MB)')
    ax.set_title('Multigrid hierarchy memory vs resolution')
    ax.grid(True, alpha=0.3)
    ax.legend()
    plt.tight_layout()
    plt.savefig(out_path, dpi=120)
    print(f'Saved {out_path}')


if __name__ == '__main__':
    infile = sys.argv[1] if len(sys.argv) > 1 else None
    rows = load(infile)
    if not rows:
        print('no rows')
        sys.exit(1)
    base = infile.rsplit('.', 1)[0] if infile else 'memory_story'
    plot_hierarchy(rows, base)
    plot_scaling(rows, base + '_scaling.png')
