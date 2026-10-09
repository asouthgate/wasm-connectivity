#!/usr/bin/env python3
"""Plot Laplacian structural nonzeros per multigrid level.

Usage:
    python3 tests/scripts/plot_profile_nnz.py profile.json
    python3 tests/scripts/plot_profile_nnz.py profile.json --output nnz.png

Accepts the NDJSON emitted by instrumentation-profile at any resolution.
Matrices are fixed across CG iterations: these counts describe hierarchy
levels, not iteration history. Stencil counts describe structural entries,
even though the fine matrix is not stored.
"""

import argparse
import json
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.ticker import MaxNLocator, StrMethodFormatter


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile", type=Path, help="Instrumentation NDJSON file")
    parser.add_argument("--output", type=Path, help="Output image (default: PROFILE_nnz.png)")
    args = parser.parse_args()

    try:
        with args.profile.open() as stream:
            records = [json.loads(line) for line in stream if line.strip()]
        records = [record for record in records
                   if (record.get("profile") or {}).get("hierarchy")]
        if not records:
            parser.error("No multigrid hierarchy found; generate a profile with --solver mg or mg-stencil")
        # Extract before plotting so malformed records produce a useful error.
        curves = [(record, sorted(record["profile"]["hierarchy"],
                                  key=lambda level: level["level"])) for record in records]
        for _, levels in curves:
            for level in levels:
                if level["level"] < 0 or level["nnz"] < 0:
                    raise ValueError("level and nnz must be nonnegative")
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.error(str(error))

    fig, ax = plt.subplots(figsize=(8, 5))
    ticks = set()
    for record, levels in curves:
        indices = [level["level"] for level in levels]
        counts = [level["nnz"] for level in levels]
        ticks.update(indices)
        resolution = record["resolution"]
        label = f"{resolution}×{resolution} / {record['solver']} / {record['ground']}"
        ax.plot(indices, counts, marker="o", label=label)
    ax.set_xticks(sorted(ticks))
    ax.set_xlabel("level")
    ax.set_ylabel("nnz")
    ax.yaxis.set_major_locator(MaxNLocator(integer=True))
    ax.yaxis.set_major_formatter(StrMethodFormatter("{x:,.0f}"))
    ax.set_ylim(bottom=0)
    ax.margins(y=0.15)
    ax.grid(True, alpha=0.3)
    ax.legend()
    fig.tight_layout()
    output = args.output or args.profile.with_name(args.profile.stem + "_nnz.png")
    fig.savefig(output, dpi=150)
    plt.close(fig)
    print(f"Saved {output}")


if __name__ == "__main__":
    main()
