import initModule from './wasm_connect.js';
import wasmUrl from './wasm_connect_bg.wasm?url';

let ready = false;

export async function load() {
  if (!ready) {
    await initModule(wasmUrl);
    ready = true;
  }
}

export { downsample_raster as downsampleRaster } from './wasm_connect.js';

let _worker = null;
let _reqId = 0;
const _pending = new Map();

// Lazily construct the module worker on first use. Deferring construction
// keeps importing this module free of browser-only side effects (e.g. in
// Node/jsdom test environments) until a compute call is actually made.
function _getWorker() {
  if (!_worker) {
    _worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });
    _worker.onmessage = (e) => {
      const { id, result, error } = e.data;
      const cb = _pending.get(id);
      if (cb) { _pending.delete(id); if (error) cb.reject(error); else cb.resolve(result); }
    };
    _worker.onerror = (e) => {
      const err = e.message || String(e.error || 'worker error');
      for (const [, cb] of _pending) cb.reject(new Error(err));
      _pending.clear();
    };
  }
  return _worker;
}

function _callWorker(fn, args, transfer) {
  return new Promise((resolve, reject) => {
    const id = ++_reqId;
    _pending.set(id, { resolve, reject });
    _getWorker().postMessage({ id, fn, args }, transfer ?? []);
  });
}

// Collect the ArrayBuffer(s) of any typed arrays among the args so they are
// transferred (moved) rather than structured-cloned — avoids a full copy of
// multi-megabyte raster buffers on the main thread.
function _buffers(...args) {
  const buffers = [];
  for (const a of args) {
    if (ArrayBuffer.isView(a) && a.buffer) buffers.push(a.buffer);
  }
  return buffers;
}

export async function runGeospatialPipelineCachedMgAsync(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter = 50_000, tol = 1e-6, useDirichletGround = false) {
  const args = [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol, useDirichletGround];
  return _callWorker('run_geospatial_pipeline_cached_mg', args, _buffers(baseRaster, srcData, gndData));
}

export async function rasterizeGeojsonAsync(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize) {
  const args = [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize];
  return _callWorker('rasterize_geojson', args, _buffers(baseRaster));
}

export async function runResistancePipelineBrowserAsync(roadBinary, riverBinary, buildingMask, dtm, dsm, genericResistance, lamps, landscapeConductance, paramsJson) {
  const args = [roadBinary, riverBinary, buildingMask, dtm, dsm, genericResistance, lamps, landscapeConductance, paramsJson];
  return _callWorker('run_resistance_pipeline_browser', args, _buffers(roadBinary, riverBinary, buildingMask, dtm, dsm, genericResistance, lamps, landscapeConductance));
}

export async function roostFinderComputeAsync(detectorsCsv, masterCsv, sunsetCsv, minutesAfterSunset, perNight, gridSize, captureRadius, diffusivity, t0, t1, loss) {
  const args = [detectorsCsv, masterCsv, sunsetCsv, minutesAfterSunset, perNight, gridSize, captureRadius, diffusivity, t0, t1, loss];
  return _callWorker('roost_finder_compute', args);
}

export async function resetCacheAsync() {
  return _callWorker('reset_cache', []);
}
