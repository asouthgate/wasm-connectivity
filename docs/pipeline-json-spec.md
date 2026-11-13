# Resistance pipeline JSON specification

See the [horseshoe-bat model](../example/horseshoe-bat-model.json) for an example of
resistance pipeline parameterisation using bundled data. 

## Model fields

| Field | Meaning |
| --- | --- |
| `name` | Model name |
| `grid` | Optional explicit CRS, bounds and cell size. Required when there are no raster inputs. |
| `inputs` | Named data sources. All declared inputs must exist. |
| `nodes` | Named processing nodes. |
| `output` | A reference to the raster to export as `total_resistance.tif`. |

## Grid

Omit `grid` to infer it from aligned raster inputs. For a vector-only model,
provide it explicitly. Raster models may also provide it to set the output area:

```json
{
  "grid": {
    "crs": "EPSG:27700",
    "bounds": [
      286588,
      79118,
      287088,
      79618
    ],
    "cellsize": 1
  }
}
```

`bounds` are `[xmin, ymin, xmax, ymax]`; `cellsize` is in metres. 

With an explicit grid, raster inputs must have the same CRS and cell size,
although data is not required to overlap and missing data is tolerated.

## Inputs

Input definitions declare catalogue types, not file paths or field mappings.
Each input name creates a required command-line flag with exactly that name. For example, 
the CLI flags `--dtm`, `--dsm` etc. will be required for the following JSON:

```json
{
  "inputs": {
    "dtm": {
      "type": "raster_topography"
    },
    "dsm": {
      "type": "raster_surface"
    },
    "landcover": {
      "type": "raster_ranked"
    },
    "buildings": {
      "type": "vector_occluder"
    },
    "roads": {
      "type": "vector_distance"
    },
    "rivers": {
      "type": "vector_distance"
    },
    "generic": {
      "type": "vector_resistance"
    },
    "lamps": {
      "type": "raycaster_point_source"
    }
  }
}
```

The following input types are currently supported:

| Type | Description |
| --- | --- |
| `raster_topography` | Ground elevation raster (DTM). | 
| `raster_surface` | Surface elevation raster (DSM). | 
| `raster_ranked` | Land-cover classes or ranks. | 
| `vector_distance` | Line or polygon geometry; no attributes required. | 
| `vector_occluder` | Line or polygon geometry with a finite, nonnegative `height` attribute in metres above terrain. | 
| `vector_resistance` | Line or polygon geometry with a finite numeric `resistance` attribute. | 
| `raycaster_point_source` | Point geometry with a finite, nonnegative `height` attribute in metres above terrain. | 

### Raster files

All three raster types require single-band GeoTIFFs (`.tif` or `.tiff`), north-up,
with square pixels and a projected CRS in metre units. Without an explicit `grid`,
at least one raster input is required, and all raster inputs must share dimensions,
transform and CRS. With an explicit `grid`, the grid rules above apply.

### Vector

Supported files are GeoJSON FeatureCollections (`.json` or `.geojson`) and
GeoPackages (`.gpkg`). A GeoPackage must have exactly one layer in the grid CRS.
GeoJSON coordinates must already be in that CRS.

For occluders and resistance features, overlaps take the maximum attribute value.
`vector_distance` rasterises geometry as a binary
mask, and calculates distances to features.

### Light-source files

Raycaster point sources outside the grid are ignored. Points inside feed the irradiance
engine rather than being rasterised as resistance values.

## Nodes and connections

Each node uses `op`, `inputs` and literal `parameters`.
For an operation taking one raster, `inputs` is a reference. For `sum` and
`maximum`, it is an array of references. Operations with several distinct input
roles use an object of named references. Connections use
`"inputs.roads"` for datasets or `"nodes.road_res"` for nodes.
The latter selects a node's single output. For a node with multiple outputs, select one explicitly,
for example `"nodes.surfaces.opaque"`.
The model's `output` uses the same reference format.

```json
{
  "op": "power_response",
  "inputs": "inputs.roads",
  "parameters": {
    "input_range": [0, 200],
    "base_range": [1, 0.5],  // decreases as input increases
    "exponent": 5,
    "scale": 10,
    "offset": 0,
    "above": 0
  }
}
```

The same node can also be written using `input` and `operations`, as below.
Here, `input` identifies the raster passed to `power_response`.

The following operations are supported:

| Operation | Inputs | Output | Description |
| --- | --- | --- | --- |
| `round` | raster | raster | Round to integers. |
| `discrete_map` | raster | raster | Map intervals to values. |
| `distance` | raster | raster | Distance to nonzero cells. |
| `power_response` | raster | raster | Apply a power response curve. |
| `override` | raster, raster | raster | Replace values where a condition matches. |
| `mask_na` | raster, raster[] | raster | Propagate missing cells from other rasters. |
| `normalise` | raster | raster | Divide by the positive maximum. |
| `sum` | raster[] | raster | Add rasters cellwise. |
| `maximum` | raster[] | raster | Take the maximum cellwise. |
| `surface_heights` | raster, raster, raster | raster, raster | Derive opaque and translucent heights. |
| `irradiance` | raycaster_point_source, raster, raster, raster | raster | Raycast light with occlusion and attenuation. |

`raster[]` means a nonempty list of rasters.

Operations taking one raster can be listed in `operations`. Reference arrays
must be nonempty. Parameters are required unless stated otherwise; `inputs`
and `parameters` may be omitted when empty.

## Pipeline operations

Examples below without `inputs` are entries in an `operations` array.
To use one directly as a named node, add an `inputs` reference as above.
These operations can be chained and implicitly take their first arg.

### `round`

```json
{
  "op": "round"
}
```

Rounds each value to the nearest integer.

### `discrete_map`

```json
{
  "op": "discrete_map",
  "parameters": {
    "intervals": {
      "(1,3)": 4,
      "(3,6)": 1,
      "[6,inf)": 2
    },
    "unmatched": 0
  }
}
```

Maps each matching interval to a finite numeric value. It covers classification,
ranking and binary presence masks.

| Notation | Matches |
| --- | --- |
| `[1,3)` | `1 <= x < 3` |
| `(3,5]` | `3 < x <= 5` |
| `[4,4]` | Exactly `4` |
| `(-inf,0)` | Values below zero |
| `[6,inf)` | Values at least six |

Infinite endpoints must use parentheses.
In the example, heights exactly 1 or 3 receive the unmatched value.

`unmatched` must be a finite numeric value, `"nodata"`, or `"error"`. Missing
input cells remain missing regardless of `unmatched`.

### `distance`

```json
{
  "op": "distance",
  "parameters": {
    "units": "metres",
    "when_empty": "infinity"
  }
}
```

Computes Euclidean distance to the nearest finite, nonzero input cell. 
Missing input cells remain missing.

`when_empty` specifies the distance when no features exist: either a finite
nonnegative number in metres, or `"infinity"`.

### `power_response`

```json
{
  "op": "power_response",
  "parameters": {
    "input_range": [
      0,
      200
    ],
    "base_range": [
      1,
      0.5
    ],
    "exponent": 5,
    "scale": 10,
    "offset": 0,
    "above": 0
  }
}
```

For `input_range = [lo, hi]` and `base_range = [start, end]`, values within the
inclusive input range are transformed as:

```text
base = start + (end - start) * (value - lo) / (hi - lo)
result = scale * base^exponent + offset
```

### `override`

```json
{
  "op": "override",
  "inputs": {
    // note the first argument is implicit 
    "where": "inputs.buildings"
  },
  "parameters": {
    "value": 8,
    "interval": "(0,inf)"
  }
}
```

Replaces the current value where the referenced raster matches the interval.

### `mask_na`

```json
{
  "op": "mask_na",
  "inputs": {
    // Implicit first argument
    "mask": [
      "nodes.surfaces.translucent",
      "inputs.landcover"
      // mask_na takes an array of masks
    ]
  }
}
```

Makes the current cell missing wherever any referenced raster cell is nonfinite.
Otherwise its current value is preserved. The reference list must be nonempty.

### `normalise`

```json
{
  "op": "normalise",
  "parameters": {
    "method": "positive_maximum",
    "when_zero": 0
  }
}
```

Divides finite values by the raster's positive maximum.

## Operations with explicit inputs

For the following operations, inputs must be explicit, and operator chaining is
not possible within a node.

### `sum` and `maximum`

```json
{
  "op": "sum",
  "inputs": [
    "nodes.road_res",
    "nodes.river_res",
    "nodes.landscape_res"
  ]
}
```

```json
{
  "op": "maximum",
  "inputs": [
    "nodes.managed_hedge_res",
    "nodes.tree_res"
  ]
}
```

### `surface_heights`

This model excerpt names the node `surfaces`:

```json
{
  "nodes": {
    "surfaces": {
      "op": "surface_heights",
      "inputs": {
        "topography": "inputs.dtm",
        "surface": "inputs.dsm",
        "occluders": "inputs.buildings"
      }
    }
  }
}
```

`topography` and `surface` contain elevations; `occluders` contains heights above
the topography, with zero outside features. The example connects DTM, DSM and
building datasets to those inputs.

The operation produces two rasters. Their names are fixed by the operation;
they are not declared in JSON. Reference them using the node name and output name:

| Output reference | Values |
| --- | --- |
| `nodes.surfaces.translucent` | Surface minus topography outside positive-height occluders; zero at occluders. |
| `nodes.surfaces.opaque` | Positive supplied occluder heights; zero elsewhere. |

Positive occluder heights override surface minus topography, including where the
surface is missing. Missing topography, or missing surface without a positive occluder override,
makes both outputs missing. Use `"nodes.surfaces.translucent"` or `"nodes.surfaces.opaque"` to select an output.
The `irradiance` example below connects these two outputs to its
`translucent_surface` and `opaque_surface` inputs.

### `irradiance`

Each input name identifies a connection on the operation, not a dataset name.

| Input name | Required data | Meaning |
| --- | --- | --- |
| `point_sources` | `raycaster_point_source` input | Source positions and heights. |
| `topography` | `raster_topography` input | Elevation raster used to position sources and trace rays. |
| `translucent_surface` | Height raster in metres above topography | Surface that attenuates rays. Connect the `translucent` output of `surface_heights`, or a `vector_occluder` input. |
| `opaque_surface` | Height raster in metres above topography | Surface that blocks rays. Connect the `opaque` output of `surface_heights`, or a `vector_occluder` input. |

```json
{
  "op": "irradiance",
  "inputs": {
    "topography": "inputs.dtm",
    "translucent_surface": "nodes.surfaces.translucent",
    "opaque_surface": "nodes.surfaces.opaque",
    "point_sources": "inputs.lamps"
  },
  "parameters": {
    "cutoff_metres": 100,
    "sensor_height_metres": 0,
    "absorption": 0.5
  }
}
```

`cutoff_metres` limits the lamp calculation radius; `sensor_height_metres` is the
receiving height above terrain, and `absorption` controls attenuation through
translucent vegetation. All three parameters must be finite and nonnegative.

The result is relative irradiance, including opaque-surface shadowing and translucent-surface
attenuation. Convert it to resistance with subsequent `normalise` and
`power_response` steps. Use `mask_na` to explicitly exclude cells with missing
terrain or surfaces, as the horseshoe-bat example does. No lamps produce zero
irradiance.

## Exporting results

The raster selected by `output` is written as `total_resistance.tif`, regardless
of its model name. It includes georeferencing, CRS and NoData metadata.

Add `--all-layers` to export each other named raster node output as `<name>.tif` and
surface outputs as `<name>.translucent.tif` and `<name>.opaque.tif`. Raw input files and
unnamed intermediate steps are not exported. The selected output is not exported
a second time under its model name. An additional node named `total_resistance`
would conflict with the final filename and is rejected when exporting all layers.

```sh
./target/release/lmc-resistance \
  --model example/horseshoe-bat-model.json \
  --dtm example/data/resistance/dtm.tif \
  --dsm example/data/resistance/dsm.tif \
  --landcover example/data/resistance/lcm.tif \
  --buildings example/data/resistance/buildings.gpkg \
  --roads example/data/resistance/roads.geojson \
  --rivers example/data/resistance/empty.geojson \
  --generic example/data/resistance/empty.geojson \
  --lamps example/data/resistance/empty.geojson \
  --output-dir results/ \
  --all-layers
```
