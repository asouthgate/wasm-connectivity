import rasterio
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
import matplotlib.colors as colors
import numpy as np
import sys
from pathlib import Path

if "--log1p" in sys.argv:
    sys.exit("--log1p has been replaced by --quantilestretch")
if "--log" in sys.argv and "--quantilestretch" in sys.argv:
    sys.exit("Use either --log or --quantilestretch, not both")

N_CLASSES = 5
if not isinstance(N_CLASSES, int) or N_CLASSES < 1:
    sys.exit("N_CLASSES must be a positive integer")
cmap = plt.get_cmap('jet', N_CLASSES)

with rasterio.Env(AAIGRID_DATATYPE="Float64"):
    with rasterio.open(sys.argv[1]) as src:
        current_data = src.read(1)
        current_data = np.where(current_data == src.nodata, np.nan, current_data)

raw_min = np.nanmin(current_data)
raw_max = np.nanmax(current_data)
print(f"Original Min Value: {raw_min}")
print(f"Original Max Value: {raw_max}")

plt.figure(figsize=(10, 8))

plot_data = np.ma.masked_invalid(current_data)
if "--log" in sys.argv:
    plot_data = np.ma.masked_less_equal(plot_data, 0)
elif "--quantilestretch" in sys.argv:
    plot_data = np.ma.masked_less(plot_data, 0)
if not plot_data.count():
    sys.exit("No valid positive values for log plotting" if "--log" in sys.argv
             else "No valid nonnegative values for quantile plotting" if "--quantilestretch" in sys.argv
             else "No valid values to plot")
plot_min, plot_max = float(plot_data.min()), float(plot_data.max())
if "--log" in sys.argv:
    norm = colors.LogNorm(vmin=plot_min, vmax=plot_max)
    ticks = np.geomspace(plot_min, plot_max, 5)
elif "--quantilestretch" in sys.argv:
    # Equal-population classes; repeated quantiles collapse into fewer
    # classes so identical values always receive the same colour.
    boundaries = np.unique(np.quantile(plot_data.compressed(), np.linspace(0, 1, N_CLASSES + 1)))
    if len(boundaries) > 1:
        norm = colors.BoundaryNorm(boundaries, ncolors=cmap.N, clip=True)
        ticks = boundaries[np.linspace(0, len(boundaries) - 1, 5).round().astype(int)]
    else:
        norm = colors.Normalize(vmin=plot_min, vmax=plot_max)
        ticks = np.array([plot_min])
else:
    norm = colors.Normalize(vmin=plot_min, vmax=plot_max)
    ticks = np.linspace(plot_min, plot_max, 5)
if plot_min == plot_max:
    ticks = np.array([plot_min])
ticks[0], ticks[-1] = plot_min, plot_max
image = plt.imshow(plot_data, cmap=cmap, norm=norm)

if "--colorbar" in sys.argv:
    cbar = plt.colorbar(image, ticks=np.unique(ticks), format='%.6g')
    cbar.minorticks_off()
    cbar.ax.tick_params(labelsize=16)
plt.axis('off') 

ofname = str(Path(sys.argv[1]).with_suffix('.png'))
plt.savefig(ofname, dpi=300, bbox_inches='tight')
print("Colored map saved as " + ofname)
