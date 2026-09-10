import {
  initSync,
  __reset as _reset,
  run_geospatial_pipeline_cached_mg as _geoCmg,
  reset_cache as _resetCache,
  rasterize_geojson as _rasterize,
  run_resistance_pipeline_browser as _resistance,
  roost_finder_compute as _roostFinder,
} from './wasm_connect.js';
import wasmUrl from './wasm_connect_bg.wasm?url';

let compiledModule = null;

async function getCompiledModule() {
  if (!compiledModule) {
    const resp = await fetch(wasmUrl);
    compiledModule = await WebAssembly.compile(await resp.arrayBuffer());
  }
  return compiledModule;
}

function freshInstance() {
  const m = compiledModule;
  if (!m) throw new Error('no compiled wasm module');
  _reset();
  initSync(m);
}

// wasm-bindgen exports accept Float64Array for f64 slices. A transferred
// Float32Array would be re-interpreted incorrectly, so ensure f64 here.
function asF64(v) {
  if (v instanceof Float64Array) return v;
  if (ArrayBuffer.isView(v)) return new Float64Array(v);
  if (Array.isArray(v)) return new Float64Array(v);
  return new Float64Array(0);
}

self.onmessage = async (e) => {
  const { id, fn, args } = e.data;

  try {
    await getCompiledModule();
    freshInstance();

    if (fn === 'reset_cache') {
      _resetCache();
      self.postMessage({ id, result: { ok: true } });
      return;
    }

    if (fn === 'roost_finder_compute') {
      const [detectorsCsv, masterCsv, sunsetCsv, gridSize, diffusivity, t0, t1] = args;
      const r = _roostFinder(
        detectorsCsv, masterCsv, sunsetCsv, gridSize, diffusivity, t0, t1,
      );
      self.postMessage({ id, result: r });
      return;
    }

    if (fn === 'rasterize_geojson') {
      const [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize] = args;
      const r = _rasterize(
        asF64(baseRaster), nrows, ncols, nodata,
        geojsonStr, layerParamsStr, xmin, ymax, cellsize,
      );
      self.postMessage({ id, result: r });
      return;
    }

    if (fn === 'run_resistance_pipeline_browser') {
      const [roadBinary, riverBinary, buildingMask, dtm, dsm, genericResistance, lamps, landscapeConductance, paramsJson] = args;
      const r = _resistance(
        asF64(roadBinary), asF64(riverBinary), asF64(buildingMask),
        asF64(dtm), asF64(dsm), asF64(genericResistance),
        asF64(lamps), asF64(landscapeConductance), paramsJson,
      );
      self.postMessage({ id, result: r });
      return;
    }

    if (fn === 'run_geospatial_pipeline_cached_mg') {
      const [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol, useDirichletGround] = args;
      const r = _geoCmg(
        asF64(baseRaster), nrows, ncols, nodata,
        geojsonStr, layerParamsStr, xmin, ymax, cellsize,
        asF64(srcData), asF64(gndData), maxIter, tol, !!useDirichletGround,
      );
      self.postMessage({ id, result: r });
      return;
    }

    self.postMessage({ id, error: `unknown fn: ${fn}` });
  } catch (err) {
    console.error('worker error:', fn, err);
    self.postMessage({ id, error: err.message || String(err) });
  }
};
