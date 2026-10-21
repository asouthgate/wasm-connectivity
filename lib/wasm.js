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
export { upsample_raster as upsampleRaster } from './wasm_connect.js';

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

function _callWorker(fn, args, transfer = []) {
  return new Promise((resolve, reject) => {
    const id = ++_reqId;
    _pending.set(id, { resolve, reject });
    try {
      _getWorker().postMessage({ id, fn, args }, transfer);
    } catch (error) {
      _pending.delete(id);
      reject(error);
    }
  });
}

export async function runGeospatialPipelineMgAsync(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter = 50_000, tol = undefined, useDirichletGround = false, precision = 'f64') {
  if (!['f32', 'f64'].includes(precision)) throw new Error('precision must be f32 or f64');
  if (tol === undefined) tol = precision === 'f32' ? 1e-3 : 1e-4;
  const args = [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol, useDirichletGround];
  return _callWorker(precision === 'f32' ? 'run_geospatial_pipeline_mg_f32' : 'run_geospatial_pipeline_mg', args);
}

export async function runGeospatialPipelineMgStencilAsync(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter = 50_000, tol = undefined, useDirichletGround = false, precision = 'f64') {
  if (!['f32', 'f64'].includes(precision)) throw new Error('precision must be f32 or f64');
  if (tol === undefined) tol = precision === 'f32' ? 1e-3 : 1e-4;
  const args = [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol, useDirichletGround];
  return _callWorker(precision === 'f32' ? 'run_geospatial_pipeline_mg_stencil_f32' : 'run_geospatial_pipeline_mg_stencil', args);
}

export async function rasterizeGeojsonAsync(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize) {
  const args = [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize];
  return _callWorker('rasterize_geojson', args);
}

// Copy inputs before transfer to avoid detaching caller-owned buffers.
function _callResistanceWorker(fn, args) {
  const copies = new Map();
  const inputs = args.slice(0, -1).map(input => {
    if (!copies.has(input)) {
      copies.set(input, ArrayBuffer.isView(input) ? input.slice() : new Float64Array(input));
    }
    return copies.get(input);
  });
  return _callWorker(fn, [...inputs, args.at(-1)], [...new Set(inputs.map(input => input.buffer))]);
}

export async function runResistancePipelineBrowserAsync(roadBinary, riverBinary, buildingMask, dtm, dsm, genericResistance, lamps, landscapeConductance, paramsJson) {
  const args = [roadBinary, riverBinary, buildingMask, dtm, dsm, genericResistance, lamps, landscapeConductance, paramsJson];
  return _callResistanceWorker('run_resistance_pipeline_browser', args);
}

export async function runResistancePipelineBrowserWithLightmapAsync(roadBinary, riverBinary, buildingMask, dtm, dsm, genericResistance, lightmap, landscapeConductance, paramsJson) {
  const args = [roadBinary, riverBinary, buildingMask, dtm, dsm, genericResistance, lightmap, landscapeConductance, paramsJson];
  return _callResistanceWorker('run_resistance_pipeline_browser_with_lightmap', args);
}

export async function roostFinderComputeAsync(detectorsCsv, masterCsv, sunsetCsv, gridSize, diffusivity, t0, t1) {
  const args = [detectorsCsv, masterCsv, sunsetCsv, gridSize, diffusivity, t0, t1];
  return _callWorker('roost_finder_compute', args);
}
