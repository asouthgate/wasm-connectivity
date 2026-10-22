#!/usr/bin/env python3
"""Plot named analytical payloads by algorithm phase."""
import argparse
import json
import sys
from collections import defaultdict

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

MB = 1_000_000
SOLVER_LABELS = {'jacobi': 'Jacobi CG', 'mg': 'Explicit MGCG', 'mg-lowmem': 'Low-memory MGCG'}
COMPONENTS = [
    ('inputs', 'Input rasters', '#7f7f7f'),
    ('rhs', 'Separate RHS', '#bdbdbd'),
    ('grounds', 'Owned ground data', '#525252'),
    ('mapping', 'Cell-to-node map', '#9edae5'),
    ('matrix_values', 'CSR values', '#e66101'),
    ('matrix_indices', 'CSR column indices', '#fdb863'),
    ('matrix_indptr', 'CSR row pointers', '#fee0b6'),
    ('cycle_correction', 'V-cycle correction e', '#5e3c99'),
    ('cycle_rhs', 'V-cycle RHS d', '#998ec3'),
    ('cycle_residual', "V-cycle residual / matvec d′", '#d8daeb'),
    ('implicit_prolonged', 'Implicit operator Px', '#1b9e77'),
    ('implicit_applied', 'Implicit operator L(Px)', '#66c2a5'),
    ('diagonals', 'Inverse diagonals', '#a6d854'),
    ('factor', 'Cholesky factor', '#e7298a'),
    ('cg_voltage', 'CG voltage v', '#084594'),
    ('cg_residual', 'CG residual r', '#2171b5'),
    ('cg_preconditioned', 'CG preconditioned residual e0', '#4292c6'),
    ('cg_direction', 'CG search direction p', '#6baed6'),
    ('cg_matvec', 'CG matrix product Lp', '#bdd7e7'),
    ('outputs', 'Returned maps', '#b15928'),
    ('setup', 'Temporary setup buffers', '#ffff99'),
    ('coarse_solve', 'Temporary coarse solve', '#fb9a99'),
    ('transfers', 'Stored transfer arrays', '#cab2d6'),
]


def load(filename=None):
    with open(filename) if filename else sys.stdin as stream:
        return [json.load(stream)]


def phase_components(phase):
    values = defaultdict(int)
    for obj in phase['objects']:
        values[obj['category']] += obj['bytes']
    if sum(values.values()) != phase['total_bytes']:
        raise ValueError(f"phase total does not match named objects: {phase['name']}")
    unknown = set(values) - {key for key, _, _ in COMPONENTS}
    if unknown:
        raise ValueError(f"unknown memory categories: {', '.join(sorted(unknown))}")
    return values


def stacked(ax, indices, components):
    bottom = [0.0] * len(indices)
    for key, label, color in COMPONENTS:
        values = [entry.get(key, 0) / MB for entry in components]
        if not any(values):
            continue
        ax.bar(indices, values, bottom=bottom, label=label, color=color,
               edgecolor='white', linewidth=0.35, zorder=3)
        bottom = [a + b for a, b in zip(bottom, values)]
    ax.set_ylabel('Size (MB)')
    ax.grid(axis='y', alpha=0.25)
    ax.set_axisbelow(True)
    return max(bottom, default=0)


def plot_hierarchy(rows, out_prefix, comparison=False):
    if comparison:
        records = sorted(rows, key=lambda row: row['solver'] == 'mg-lowmem')
        output = f'{out_prefix}_comparison.png'
    else:
        records = rows
        output = None
    # Preserve a separate output per input profile in single-profile mode.
    batches = [records] if comparison else [[record] for record in records]
    for records in batches:
        fig, axes = plt.subplots(len(records), 1, figsize=(12, 5.5 * len(records)), squeeze=False)
        maximum = 0.0
        for row, record in enumerate(records):
            profile = record['profile']
            title = f"{SOLVER_LABELS.get(record['solver'], record['solver'])} · {record['resolution']} × {record['resolution']} · {record['ground']} · {record.get('precision', record.get('profile', {}).get('precision', 'f64'))}"
            phases = profile.get('phases', [])
            left = axes[row, 0]
            if phases:
                indices = list(range(len(phases)))
                maximum = max(maximum, stacked(left, indices, [phase_components(p) for p in phases]))
                left.set_xticks(indices, [p['name'] for p in phases], rotation=45, ha='right', fontsize=8)
                left.set_xlabel('Algorithm phase')
            else:
                left.text(0.5, 0.5, 'No phase accounting in this older profile', ha='center', transform=left.transAxes)
            left.set_title(f'{title}', fontsize=11)
        for ax in axes.flat:
            ax.set_ylim(0, maximum * 1.08 if maximum else 1)
        handles = {}
        for ax in axes.flat:
            for handle, label in zip(*ax.get_legend_handles_labels()):
                handles.setdefault(label, handle)
        ordered = [(label, handles[label]) for _, label, _ in COMPONENTS if label in handles]
        if ordered:
            fig.legend([h for _, h in ordered], [label for label, _ in ordered],
                       loc='lower center', ncol=5, fontsize=8, frameon=False)
        legend_rows = (len(ordered) + 4) // 5
        fig.tight_layout(rect=(0, (0.035 + 0.025 * legend_rows) / len(records), 1, 1))
        if not comparison:
            record = records[0]
            output = f"{out_prefix}_{record['solver']}_{record['ground']}.png"
        fig.savefig(output, dpi=220)
        print(f'Saved {output}')
        plt.close(fig)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description='Plot detailed memory accounting from one or two instrumentation profiles.')
    parser.add_argument('profiles', nargs='*', help='JSON profiles (default: stdin)')
    args = parser.parse_args()
    if len(args.profiles) > 2:
        parser.error('expected at most two JSON profiles')
    try:
        rows = [record for filename in args.profiles for record in load(filename)] if args.profiles else load()
        if not all(isinstance(record, dict) and record.get('profile') for record in rows):
            parser.error('each input must contain a profile')
        comparison = len(args.profiles) == 2
        if comparison and sum(record.get('solver') == 'mg-lowmem' for record in rows) != 1:
            parser.error('two-profile mode requires exactly one mg-lowmem profile')
        base = args.profiles[0].rsplit('.', 1)[0] if args.profiles else 'instrumentation_profile'
        plot_hierarchy(rows, base, comparison=comparison)
    except (OSError, ValueError, KeyError) as error:
        parser.error(str(error))
