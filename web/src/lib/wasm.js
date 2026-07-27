import {initModule, downsample_raster} from './wasm_connect.js';
import wasmUrl from './wasm_connect_bg.wasm?url';

let ready = false;

export async function load() {
  if (!ready) {
    await initModule(wasmUrl);
    ready = true;
  }
}

// export for consistent interface
export { downsample_raster as downsampleRaster } from './wasm_connect.js';

const _worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });
let _reqId = 0;
const _pending = new Map();

_worker.onmessage = (e) => {
  const { id, result, error } = e.data;
  const cb = _pending.get(id);
  if (cb) { _pending.delete(id); if (error) cb.reject(error); else cb.resolve(result); }
};

function _callWorker(fn, args) {
  return new Promise((resolve, reject) => {
    const id = ++_reqId;
    _pending.set(id, { resolve, reject });
    _worker.postMessage({ id, fn, args });
  });
}

export async function runGeospatialPipelineCachedMgAsync(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter = 50_000, tol = 1e-6, useDirichletGround = false) {
  return _callWorker('run_geospatial_pipeline_cached_mg', [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol, useDirichletGround]);
}

export async function resetCacheAsync() {
  return _callWorker('reset_cache', []);
}

export function benchmarkJacobi(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, useDirichletGround = false) {
  return _callWorker('benchmark_jacobi', [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, useDirichletGround]);
}
export function benchmarkGmg(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, useDirichletGround = false) {
  return _callWorker('benchmark_gmg', [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, useDirichletGround]);
}
export function benchmarkAlcouffe(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, useDirichletGround = false) {
  return _callWorker('benchmark_alcouffe', [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, useDirichletGround]);
}
