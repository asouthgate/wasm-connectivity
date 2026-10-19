#!/usr/bin/env python3
"""Plot analytical solver payloads emitted by instrumentation-profile.

Usage:
    cargo run --profile release-prof --features bin,instrumentation-profile \
        --bin instrumentation-profile -- 500 --solver mg-lowmem --ground shunt > profile.json
    python3 scripts/plot_instrumentation_profile.py profile.json
"""
import argparse
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


def plot_hierarchy(rows, out_prefix, comparison=False):
    groups = defaultdict(list)
    for record in rows:
        if record.get('profile') and record['profile'].get('hierarchy'):
            groups[(record['solver'], record['ground'])].append(record)

    if comparison:
        fig, axes = plt.subplots(2, 1, figsize=(10, 9), sharex=True, sharey=True)
        groups_to_plot = [((record['solver'], record['ground']), [record])
                          for record in sorted(rows, key=lambda row: row['solver'] == 'mg-lowmem')]
        maximum = 0.0
        all_indices = set()
    else:
        groups_to_plot = sorted(groups.items())

    for panel, ((solver, ground), records) in enumerate(groups_to_plot):
        best = max(records, key=lambda record: record['resolution'])
        levels = best['profile']['hierarchy']
        indices = [level['level'] for level in levels]
        components = [
            ('Laplacian', '#e66101', [level['laplacian_bytes'] for level in levels]),
            ('Workspace', '#5e3c99',
             [total_level_bytes(level) - level['laplacian_bytes'] for level in levels]),
        ]
        if comparison:
            ax = axes[panel]
            all_indices.update(indices)
        else:
            fig, ax = plt.subplots(figsize=(7, 6))
        bottom = [0.0] * len(levels)
        for label, color, values in components:
            heights = [value / 1e6 for value in values]
            ax.bar(indices, heights, bottom=bottom, label=label,
                   facecolor='white', edgecolor=color, linewidth=2, zorder=3)
            bottom = [a + b for a, b in zip(bottom, heights)]
        if comparison:
            maximum = max(maximum, max(bottom, default=0.0))
        elif any(bottom):
            ax.set_ylim(0, max(bottom) * 1.25)
        if not comparison or panel == 1:
            ax.set_xlabel('Level')
        ax.set_ylabel('Memory (MB)' if comparison else 'Memory')
        ax.set_xticks(indices)
        ax.grid(axis='y', alpha=0.3)
        ax.set_axisbelow(True)
        if not comparison or panel == 0:
            ax.legend(loc='upper left')
        if not comparison:
            fig.tight_layout()
            output = f'{out_prefix}_{solver}_{ground}.png'
            fig.savefig(output, dpi=300)
            print(f'Saved {output}')
            plt.close(fig)

    if comparison:
        axes[1].set_xticks(sorted(all_indices))
        axes[0].set_ylim(0, maximum * 1.25 if maximum else 1)
        fig.tight_layout()
        output = f'{out_prefix}_comparison.png'
        fig.savefig(output, dpi=300)
        print(f'Saved {output}')
        plt.close(fig)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description='Plot one or two instrumentation profiles.')
    parser.add_argument('profiles', nargs='*', help='JSON profiles (default: stdin)')
    args = parser.parse_args()
    if len(args.profiles) > 2:
        parser.error('expected at most two JSON profiles')
    infile = args.profiles[0] if args.profiles else None
    try:
        rows = [record for filename in args.profiles for record in load(filename)] if args.profiles else load()
    except (OSError, ValueError) as error:
        parser.error(str(error))
    comparison = len(args.profiles) == 2
    if comparison:
        if not all(isinstance(record, dict) and record.get('profile')
                   and record['profile'].get('hierarchy') for record in rows):
            parser.error('both profiles must contain a non-empty hierarchy')
        if sum(record.get('solver') == 'mg-lowmem' for record in rows) != 1:
            parser.error('two-profile mode requires exactly one mg-lowmem profile')
    if not rows:
        sys.exit('no rows')
    base = infile.rsplit('.', 1)[0] if infile else 'instrumentation_profile'
    plot_hierarchy(rows, base, comparison=comparison)
