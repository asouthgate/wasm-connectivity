import rasterio
import matplotlib.pyplot as plt
import matplotlib.colors as colors
import numpy as np
import sys

with rasterio.open(sys.argv[1]) as src:
    current_data = src.read(1)
    current_data = np.where(current_data == src.nodata, np.nan, current_data)

raw_min = np.nanmin(current_data)
raw_max = np.nanmax(current_data)
print(f"Original Min Value: {raw_min}")
print(f"Original Max Value: {raw_max}")

scaled_data = 1 + ((current_data - raw_min) / (raw_max - raw_min)) * (100 - 1)

print(f"Rescaled Min Value: {np.nanmin(scaled_data)}")
print(f"Rescaled Max Value: {np.nanmax(scaled_data)}")

plt.figure(figsize=(10, 8))

plt.imshow(scaled_data, cmap='inferno', norm=colors.LogNorm(vmin=1, vmax=100))

if "--colorbar" in sys.argv:
    cbar = plt.colorbar()
    cbar.ax.tick_params(labelsize=16)
plt.axis('off') 

ofname = sys.argv[1].split(".")[0] + ".png"
plt.savefig(ofname, dpi=300, bbox_inches='tight')
print("Colored map saved as " + ofname)
