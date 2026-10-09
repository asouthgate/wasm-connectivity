#!/usr/bin/env python3
"""Plot analytical solver payloads emitted by instrumentation-profile.

Input: NDJSON records with resolution, solver, ground, total_iters, and profile.
Produces per-MG-hierarchy figures including level 0, and a scaling comparison
of named solver buffers across Jacobi, explicit MG, and MG with matrix-free
levels 0 and 1. Both MG paths generate transfers without entry arrays. Setup
buffers and allocator overhead are excluded; these totals are not heap peaks.

Usage:
    cargo run --profile release-prof --features bin,instrumentation-profile \
        --bin instrumentation-profile -- 500 --solver all > profile.jsonl
    python3 tests/scripts/plot_instrumentation_profile.py profile.jsonl
"""
import json
import sys
from collections import defaultdict

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

SOLVER_COLORS = {'jacobi': '#444444', 'mg': '#e66101', 'mg-stencil': '#2c7fb8'}
SOLVER_LABELS = {'jacobi': 'Jacobi CG', 'mg': 'CGMG', 'mg-stencil': 'CGMG (matrix-free levels 0–1)'}


def load(filename=None):
    if filename:
        with open(filename) as stream:
            return [json.loads(line) for line in stream if line.strip()]
    return [json.loads(line) for line in sys.stdin if line.strip()]


def total_level_bytes(level):
    return sum(level[key] for key in (
        'laplacian_bytes', 'prolongation_p_bytes', 'cholesky_factor_bytes',
        'workspace_e_bytes', 'workspace_d_bytes', 'workspace_d_prime_bytes')) + sum(
            level.get(key, 0) for key in ('smoother_diag_bytes', 'operator_matvec_scratch_bytes'))


def solver_storage_bytes(profile):
    levels = profile.get('hierarchy', [])
    # MG includes its fine matrix at level 0; Jacobi has no hierarchy.
    matrix_and_workspace = (sum(total_level_bytes(level) for level in levels)
                            if levels else profile['laplacian_bytes'])
    return matrix_and_workspace + sum(profile.get(key, 0) for key in (
        'ground_storage_bytes', 'filled_resistance_bytes', 'cell_to_node_map_bytes',
        'voltage_v_bytes', 'residual_r_bytes', 'preconditioned_residual_e0_bytes',
        'search_direction_p_bytes', 'matvec_lp_bytes', 'source_s_bytes',
        'jacobi_diag_bytes', 'cache_laplacian_bytes', 'cache_cell_to_node_bytes',
        'cache_last_voltages_bytes'))


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
            ('Laplacian $L_l$', '#555555', [level['laplacian_bytes'] for level in levels]),
            ('Prolongation $P_l$', '#e66101', [level['prolongation_p_bytes'] for level in levels]),
            ("Workspace $e_l, d_l, d'_l$", '#f1c40f',
             [sum(level[key] for key in ('workspace_e_bytes', 'workspace_d_bytes',
                                        'workspace_d_prime_bytes')) for level in levels]),
            ('Cholesky factor', '#2c7fb8', [level['cholesky_factor_bytes'] for level in levels]),
            ('Jacobi diagonal', '#8e44ad', [level.get('smoother_diag_bytes', 0) for level in levels]),
            ('Operator scratch', '#16a085', [level.get('operator_matvec_scratch_bytes', 0) for level in levels]),
        ]
        # New profiles generate P geometrically; omit its empty legend entry.
        # Keep the category for older profiles containing stored transfers.
        components = [(label, color, values) for label, color, values in components
                      if any(values)]
        fig, ax = plt.subplots(figsize=(8, 5))
        bottom = [0.0] * len(levels)
        for label, color, values in components:
            heights = [value / 1e6 for value in values]
            ax.bar(indices, heights, bottom=bottom, label=label, color=color)
            bottom = [a + b for a, b in zip(bottom, heights)]
        if any(bottom):
            ax.set_ylim(0, max(bottom) * 1.15)
        ax.set_xlabel('Level $l$ (0 = finest)')
        ax.set_ylabel('Analytical payload (MB)')
        ax.set_title(f"Hierarchy — {SOLVER_LABELS.get(solver, solver)} / {ground} "
                     f"({best['resolution']}×{best['resolution']})")
        ax.set_xticks(indices)
        ax.grid(True, axis='y', alpha=0.3)
        ax.legend()
        fig.tight_layout()
        output = f'{out_prefix}_{solver}_{ground}.png'
        fig.savefig(output, dpi=120)
        print(f'Saved {output}')
        plt.close(fig)


def plot_scaling(rows, out_path):
    # Separate ground modes and average repeated runs at the same resolution.
    groups = defaultdict(lambda: defaultdict(list))
    for record in rows:
        if record.get('profile') and record['solver'] in SOLVER_COLORS:
            groups[(record['solver'], record['ground'])][record['resolution']].append(
                solver_storage_bytes(record['profile']) / 1e6)
    if not groups:
        print('no solver profiles found')
        return
    fig, ax = plt.subplots(figsize=(8, 5))
    for (solver, ground), points in sorted(groups.items()):
        resolutions = sorted(points)
        means = [sum(points[res]) / len(points[res]) for res in resolutions]
        ax.plot(resolutions, means, marker='o', color=SOLVER_COLORS[solver],
                linestyle='--' if ground == 'dirichlet' else '-',
                label=f'{SOLVER_LABELS[solver]} / {ground}')
    ax.set_xlabel('Resolution (pixels per side)')
    ax.set_ylabel('Named solver payloads (MB)')
    ax.set_title('Analytical Solver Storage vs Resolution')
    ax.grid(True, alpha=0.3)
    ax.legend()
    fig.tight_layout()
    fig.savefig(out_path, dpi=120)
    print(f'Saved {out_path}')
    plt.close(fig)


if __name__ == '__main__':
    infile = sys.argv[1] if len(sys.argv) > 1 else None
    rows = load(infile)
    if not rows:
        sys.exit('no rows')
    base = infile.rsplit('.', 1)[0] if infile else 'instrumentation_profile'
    plot_hierarchy(rows, base)
    plot_scaling(rows, base + '_scaling.png')
