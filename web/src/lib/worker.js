import initModule, {
  run_geospatial_pipeline as _geo,
  solve_point_sources as _pts,
  solve_raster_sources as _rast,
  rasterize_geojson as _rasterize,
  get_memory,
  __reset,
} from './wasm_connect.js';
import wasmUrl from './wasm_connect_bg.wasm?url';

let ready = false;
function mb() { return get_memory().buffer.byteLength / (1024 * 1024); }
async function ensure() { if (!ready) { await initModule(wasmUrl); ready = true; } }

const FN_MAP = {
  run_geospatial_pipeline: _geo,
  solve_point_sources: _pts,
  solve_raster_sources: _rast,
};

self.onmessage = async (e) => {
  const { id, fn, args } = e.data;

  if (fn === 'benchmark') {
    let result;
    try {
      __reset();
      await initModule(wasmUrl);
      ready = true;

      const [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, sourceData, groundData] = args;

      const t0 = performance.now();
      const resJson = _rasterize(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize);
      const t1 = performance.now();
      const memPrep = mb();

      const parsed = JSON.parse(resJson);
      const t2 = performance.now();
      _rast(new Float64Array(parsed.resistance_map), parsed.nrows, parsed.ncols, nodata, sourceData, groundData, 100_000, 1e-6);
      const t3 = performance.now();
      const memConn = mb();

      result = {
        prepTimeMs: t1 - t0,
        connTimeMs: t3 - t2,
        prepMemMb: memPrep,
        connMemMb: memConn,
      };
    } catch (err) {
      result = { error: err.message || String(err) };
    }
    self.postMessage({ id, result });
    return;
  }

  try {
    await ensure();
    const r = FN_MAP[fn](...args);
    self.postMessage({ id, result: r });
  } catch (err) {
    self.postMessage({ id, error: err.message || String(err) });
  }
};
