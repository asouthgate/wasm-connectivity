import {
  initSync,
  __reset as _reset,
  run_geospatial_pipeline_mg as _geoMg,
  run_geospatial_pipeline_mg_stencil as _geoMgStencil,
  rasterize_geojson as _rasterize,
  run_resistance_pipeline_browser as _resistance,
  run_resistance_pipeline_browser_with_lightmap as _resistanceLightmap,
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

function postResistanceResult(id, output) {
  const fields = ['total_res', 'lamp_res', 'road_res', 'river_res', 'landscape_res',
    'linear_res', 'generic_res', 'soft_surf', 'hard_surf'];
  try {
    const result = { nrows: output.nrows, ncols: output.ncols };
    const transfer = [];
    fields.forEach((field, index) => {
      result[field] = output.take_raster(index);
      transfer.push(result[field].buffer);
    });
    self.postMessage({ id, result }, transfer);
  } finally {
    output.free();
  }
}

self.onmessage = async (e) => {
  const { id, fn, args } = e.data;

  try {
    await getCompiledModule();
    freshInstance();

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
      postResistanceResult(id, r);
      return;
    }

    if (fn === 'run_resistance_pipeline_browser_with_lightmap') {
      const [roadBinary, riverBinary, buildingMask, dtm, dsm, genericResistance, lightmap, landscapeConductance, paramsJson] = args;
      const r = _resistanceLightmap(
        asF64(roadBinary), asF64(riverBinary), asF64(buildingMask),
        asF64(dtm), asF64(dsm), asF64(genericResistance),
        asF64(lightmap), asF64(landscapeConductance), paramsJson,
      );
      postResistanceResult(id, r);
      return;
    }

    if (fn === 'run_geospatial_pipeline_mg') {
      const [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol, useDirichletGround] = args;
      const r = _geoMg(
        asF64(baseRaster), nrows, ncols, nodata,
        geojsonStr, layerParamsStr, xmin, ymax, cellsize,
        asF64(srcData), asF64(gndData), maxIter, tol, !!useDirichletGround,
      );
      self.postMessage({ id, result: r });
      return;
    }

    if (fn === 'run_geospatial_pipeline_mg_stencil') {
      const [baseRaster, nrows, ncols, nodata, geojsonStr, layerParamsStr, xmin, ymax, cellsize, srcData, gndData, maxIter, tol, useDirichletGround] = args;
      const r = _geoMgStencil(
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
