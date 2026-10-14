#!/usr/bin/env python3
"""Plot analytical solver payloads emitted by instrumentation-profile.

Usage:
    cargo run --profile release-prof --features bin,instrumentation-profile \
        --bin instrumentation-profile -- 500 --solver mg-lowmem --ground shunt > profile.json
    python3 scripts/plot_instrumentation_profile.py profile.json
"""
import json
import sys
from collections import defaultdict

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

SOLVER_COLORS = {'jacobi': '#444444', 'mg': '#e66101', 'mg-lowmem': '#2c7fb8'}
SOLVER_LABELS = {'jacobi': 'Jacobi CG', 'mg': 'CGMG', 'mg-lowmem': 'MG low-memory'}


def load(filename=None):
    if filename:
        with open(filename) as stream:
            return [json.load(stream)]
    return [json.load(sys.stdin)]


def total_level_bytes(level):
    return sum(level[key] for key in (
        'laplacian_bytes', 'prolongation_p_bytes', 'cholesky_factor_bytes',
        'workspace_e_bytes', 'workspace_d_bytes', 'workspace_d_prime_bytes')) + sum(
            level.get(key, 0) for key in ('smoother_diag_bytes', 'operator_matvec_scratch_bytes'))


def plot_hierarchy(rows, out_prefix):
    groups = defaultdict(list)
    for record in rows:
        if record.get('profile') and record['profile'].get('hierarchy'):
            groups[(record['solver'], record['ground'])].append(record)

    for (solver, ground), records in sorted(groups.items()):
        best = max(records, key=lambda record: record['resolution'])
        levels = best['profile']['hierarchy']
        indices = [level['level'] for level in levels]
        components = [
            ('Laplacian', '#444444', [level['laplacian_bytes'] for level in levels]),
            ('Workspace', '#2c7fb8',
             [total_level_bytes(level) - level['laplacian_bytes'] for level in levels]),
        ]
        fig, ax = plt.subplots(figsize=(7, 6))
        bottom = [0.0] * len(levels)
        for label, color, values in components:
            heights = [value / 1e6 for value in values]
            ax.bar(indices, heights, bottom=bottom, label=label, color=color)
            bottom = [a + b for a, b in zip(bottom, heights)]
        if any(bottom):
            ax.set_ylim(0, max(bottom) * 1.25)
        ax.set_xlabel('Level')
        ax.set_ylabel('Memory')
        ax.set_xticks(indices)
        ax.grid(True, alpha=0.3)
        ax.set_axisbelow(True)
        ax.legend(loc='upper left')
        fig.tight_layout()
        output = f'{out_prefix}_{solver}_{ground}.png'
        fig.savefig(output, dpi=300)
        print(f'Saved {output}')
        plt.close(fig)


if __name__ == '__main__':
    infile = sys.argv[1] if len(sys.argv) > 1 else None
    rows = load(infile)
    if not rows:
        sys.exit('no rows')
    base = infile.rsplit('.', 1)[0] if infile else 'instrumentation_profile'
    plot_hierarchy(rows, base)
