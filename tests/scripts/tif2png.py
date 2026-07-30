import rasterio
import matplotlib.pyplot as plt
import matplotlib.colors as colors
import numpy as np
import sys

# 1. Open the Circuitscape geotiff
with rasterio.open(sys.argv[1]) as src:
    current_data = src.read(1)
    # Mask out any NoData/background values so they don't ruin the statistics
    current_data = np.where(current_data == src.nodata, np.nan, current_data)

# 2. Print raw statistics (ignoring NaNs)
raw_min = np.nanmin(current_data)
raw_max = np.nanmax(current_data)
print(f"Original Min Value: {raw_min}")
print(f"Original Max Value: {raw_max}")

# 3. Rescale data from 1 to 100 (Log scale friendly)
# Using 1 instead of 0 because Log(0) is undefined and breaks LogNorm
scaled_data = 1 + ((current_data - raw_min) / (raw_max - raw_min)) * (100 - 1)

print(f"Rescaled Min Value: {np.nanmin(scaled_data)}")
print(f"Rescaled Max Value: {np.nanmax(scaled_data)}")

# 4. Set up the plot
plt.figure(figsize=(10, 8))

# 5. Apply the color scheme and LogNorm
# Setting vmin=1 and vmax=100 explicitly forces the colorbar bounds
plt.imshow(scaled_data, cmap='inferno', norm=colors.LogNorm(vmin=1, vmax=100))

# 6. Add a colorbar legend to see the new 1-100 scale
# plt.colorbar(label='Current Intensity (Rescaled 1-100)')
plt.axis('off') 

# 7. Save the colored image
ofname = sys.argv[1].split(".")[0] + ".png"
plt.savefig(ofname, dpi=300, bbox_inches='tight')
print("Colored map saved as " + ofname)
