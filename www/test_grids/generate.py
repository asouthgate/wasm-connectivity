#!/usr/bin/env python3
"""Generate test raster grids for wasm-connect with interesting connectivity patterns."""
import os, math, random

OUT = os.path.dirname(__file__) or "."

def write_asc(filename, ncols, nrows, data, nodata=-9999):
    path = os.path.join(OUT, filename)
    with open(path, 'w') as f:
        f.write(f"ncols {ncols}\n")
        f.write(f"nrows {nrows}\n")
        f.write("xllcorner 0.0\n")
        f.write("yllcorner 0.0\n")
        f.write("cellsize 1.0\n")
        f.write(f"NODATA_value {nodata}\n")
        for row in range(nrows):
            f.write(" ".join(f"{data[row*ncols + col]:.6g}" for col in range(ncols)) + "\n")
    size_kb = os.path.getsize(path) / 1024
    print(f"  wrote {filename} ({size_kb:.0f} KB)")

def write_pts(filename, ncols, nrows, points, nodata=-9999):
    """points: list of (id, row, col) tuples."""
    data = [nodata] * (nrows * ncols)
    for pid, r, c in points:
        data[r * ncols + c] = pid
    write_asc(filename, ncols, nrows, data, nodata)

def gen_uniform(size):
    print(f"\n=== uniform_{size} ===")
    n = size * size
    data = [1.0] * n
    write_asc(f"uniform_{size}_res.asc", size, size, data)
    points = [(1, 0, 0), (2, size-1, size-1)]
    write_pts(f"uniform_{size}_pts.asc", size, size, points)

def gen_two_paths(size):
    print(f"\n=== two_paths_{size} ===")
    data = [50.0] * (size * size)
    half = size // 2
    corr_w = max(3, size // 40)
    for row in range(size):
        # Left corridor (narrow low-resistance path)
        for col in range(corr_w, half - corr_w):
            data[row * size + col] = 1.0
        # Right corridor
        for col in range(half + corr_w, size - corr_w):
            data[row * size + col] = 1.0
        # High barrier in middle
        for col in range(half - corr_w, half + corr_w):
            data[row * size + col] = 200.0
    # Bridge at top quarter and bottom quarter
    for col in range(half - corr_w, half + corr_w):
        for row in range(0, size // 8):
            data[row * size + col] = 1.0
        for row in range(7 * size // 8, size):
            data[row * size + col] = 1.0
    write_asc(f"two_paths_{size}_res.asc", size, size, data)
    points = [(1, size // 4, corr_w // 2), (2, 3 * size // 4, size - corr_w // 2)]
    write_pts(f"two_paths_{size}_pts.asc", size, size, points)

def gen_fragmented(size):
    print(f"\n=== fragmented_{size} ===")
    random.seed(42)
    data = [1.0] * (size * size)
    n_holes = size // 4
    hole_r = size // 12
    for _ in range(n_holes):
        cx = random.randint(hole_r, size - 1 - hole_r)
        cy = random.randint(hole_r, size - 1 - hole_r)
        for r in range(max(0, cy - hole_r), min(size, cy + hole_r)):
            for c in range(max(0, cx - hole_r), min(size, cx + hole_r)):
                dist = math.sqrt((r - cy)**2 + (c - cx)**2)
                if dist < hole_r:
                    data[r * size + c] = -9999
    write_asc(f"fragmented_{size}_res.asc", size, size, data)
    # Find conductive points near opposite corners
    points = [(1, 10, 10), (2, size - 11, size - 11)]
    write_pts(f"fragmented_{size}_pts.asc", size, size, points)

def gen_gradient(size):
    print(f"\n=== gradient_{size} ===")
    data = [0.0] * (size * size)
    for row in range(size):
        for col in range(size):
            # Low resistance at top-left, high resistance at bottom-right
            t = (row + col) / (2.0 * size)
            data[row * size + col] = 0.5 + 99.5 * t
    write_asc(f"gradient_{size}_res.asc", size, size, data)
    points = [(1, 0, 0), (2, size-1, size-1)]
    write_pts(f"gradient_{size}_pts.asc", size, size, points)

def gen_bridge(size):
    print(f"\n=== bridge_{size} ===")
    data = [1.0] * (size * size)
    barrier_start = size // 3
    barrier_end = 2 * size // 3
    barrier_width = 2
    bridge_row = size // 2
    bridge_width = max(2, size // 30)
    for row in range(size):
        for col in range(size):
            # Vertical high-resistance barrier
            if barrier_start <= row <= barrier_end and abs(col - size // 2) <= barrier_width:
                rr = min(row - barrier_start, barrier_end - row)
                if not (rr < bridge_width and abs(col - size // 2) <= barrier_width):
                    data[row * size + col] = 500.0
    # Nodata at extreme left/right to force through barrier
    for row in range(size):
        for col in range(0, size // 6):
            data[row * size + col] = -9999
        for col in range(5 * size // 6, size):
            data[row * size + col] = -9999
    write_asc(f"bridge_{size}_res.asc", size, size, data)
    points = [(1, size // 2, size // 12), (2, size // 2, 11 * size // 12)]
    write_pts(f"bridge_{size}_pts.asc", size, size, points)

def gen_rand_terrain(size):
    print(f"\n=== rand_terrain_{size} ===")
    random.seed(123)
    data = [0.0] * (size * size)
    # Generate several gaussian blobs of low resistance
    centers = [(size//2, size//2), (size//4, size//4), (3*size//4, 3*size//4),
               (size//4, 3*size//4), (3*size//4, size//4), (size//2, size//4),
               (size//4, size//2), (3*size//4, size//2), (size//2, 3*size//4)]
    sigmas = [size/6, size/10, size/10, size/10, size/10, size/12, size/12, size/12, size/12]
    for row in range(size):
        for col in range(size):
            r = 10.0 + 90.0 * random.random()
            for (cy, cx), sigma in zip(centers, sigmas):
                d = math.sqrt((row - cy)**2 + (col - cx)**2)
                r -= 8.0 * math.exp(-0.5 * (d/sigma)**2)
            data[row * size + col] = max(0.5, r)
    write_asc(f"rand_terrain_{size}_res.asc", size, size, data)
    points = [(1, size//4, size//4), (2, 3*size//4, 3*size//4),
              (3, size//4, 3*size//4), (4, 3*size//4, size//4)]
    write_pts(f"rand_terrain_{size}_pts.asc", size, size, points)

def gen_islands(size):
    print(f"\n=== islands_{size} ===")
    data = [-9999.0] * (size * size)
    random.seed(77)
    n_islands = 15
    for i in range(n_islands):
        cx = random.randint(0, size - 1)
        cy = random.randint(0, size - 1)
        radius = random.randint(size // 20, size // 8)
        for r in range(max(0, cy - radius), min(size, cy + radius)):
            for c in range(max(0, cx - radius), min(size, cx + radius)):
                if math.sqrt((r - cy)**2 + (c - cx)**2) < radius * random.uniform(0.8, 1.2):
                    data[r * size + c] = 1.0 + random.uniform(0, 2) 
    write_asc(f"islands_{size}_res.asc", size, size, data)
    write_pts(f"islands_{size}_pts.asc", size, size, [])

# Generate all sizes
for size in [100, 150, 200]:
    gen_uniform(size)
    gen_two_paths(size)
    gen_fragmented(size)
    gen_gradient(size)
    gen_bridge(size)
    gen_rand_terrain(size)

gen_islands(150)
gen_islands(200)

# 300x300 stress tests (fewer patterns, bigger files)
gen_uniform(300)
gen_two_paths(300)
gen_gradient(300)

print("\nDone!")
