import {
  initSync,
  __reset as _reset,
  run_geospatial_pipeline_cached_mg as _geoCmg,
  reset_cache as _resetCache,
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

self.onmessage = async (e) => {
  const { id, fn, args } = e.data;

  try {
    if (fn === 'reset_cache') {
      await getCompiledModule();
      freshInstance();
      _resetCache();
      self.postMessage({ id, result: { ok: true } });
      return;
    }

    if (fn === 'run_geospatial_pipeline_cached_mg') {
      await getCompiledModule();
      freshInstance();
      const [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol, useDirichletGround] = args;
      const r = _geoCmg(baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol, !!useDirichletGround);
      self.postMessage({ id, result: r });
      return;
    }

    self.postMessage({ id, error: `unknown fn: ${fn}` });
  } catch (err) {
    console.error('worker error:', fn, err);
    self.postMessage({ id, error: err.message || String(err) });
  }
};
