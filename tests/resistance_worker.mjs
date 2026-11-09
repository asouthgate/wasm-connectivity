import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { Worker as ThreadWorker } from 'node:worker_threads';

// Run the actual browser worker in Node, adapting only module URLs and messaging.
const loader = `export async function load(url, context, nextLoad) {
  if (url.endsWith('.wasm?url')) return { format: 'module', shortCircuit: true,
    source: 'export default ' + JSON.stringify(url.slice(0, -4)) };
  return nextLoad(url, context);
}`;
const loaderUrl = 'data:text/javascript,' + encodeURIComponent(loader);
register(loaderUrl, import.meta.url);
const bootstrap = `
  import { parentPort, workerData } from 'node:worker_threads';
  import { register } from 'node:module';
  import { readFile } from 'node:fs/promises';
  register(workerData.loaderUrl, workerData.workerUrl);
  globalThis.fetch = async url => new Response(await readFile(new URL(url)));
  globalThis.self = { postMessage(message, transfer = []) {
    parentPort.postMessage({ ...message, transferCount: transfer.length }, transfer);
    if (transfer.some(buffer => buffer.byteLength !== 0)) throw new Error('Output transfer did not detach');
  }};
  await import(workerData.workerUrl);
  parentPort.on('message', data => self.onmessage({ data }));
`;
const instances = [];
globalThis.Worker = class {
  constructor(url) {
    this.thread = new ThreadWorker(new URL('data:text/javascript,' + encodeURIComponent(bootstrap)), {
      workerData: { workerUrl: url.href, loaderUrl },
    });
    this.thread.on('message', data => { this.lastReply = data; this.onmessage?.({ data }); });
    this.thread.on('error', error => this.onerror?.({ message: error.message, error }));
    instances.push(this);
  }
  postMessage(message, transfer) {
    this.lastInputTypes = message.args.slice(0, -1).map(input => input.constructor.name);
    this.thread.postMessage(message, transfer);
    assert.ok(transfer.every(buffer => buffer.byteLength === 0), 'Input copies must transfer');
  }
};
after(async () => { await Promise.all(instances.map(worker => worker.thread.terminate())); });
const { rasterizeGeojsonAsync, runResistancePipelineBrowserAsync, runResistancePipelineBrowserWithLightmapAsync } =
  await import('../lib/wasm.js');
const params = {
  road_buffer: 200, road_resmax: 10, road_xmax: 5, river_buffer: 10, river_resmax: 2000, river_xmax: 4,
  landscape_rankmax: 8, landscape_resmax: 100, landscape_xmax: 5, linear_buffer: 10, linear_rankmax: 4,
  linear_resmax: 22000, linear_xmax: 3, lamp_resmax: 1e8, lamp_xmax: 1, lamp_ext: 100,
  pixw: 1, nrows: 2, ncols: 2,
};
function inputs(Type = Float32Array, genericValue = 5) {
  const zero = new Type(4);
  return [zero, zero, zero, new Type(4).fill(10), new Type(4).fill(12),
    new Type(4).fill(genericValue), new Type(0), new Type(4).fill(4)];
}
function check(result, genericValue = 5) {
  assert.equal(result.nrows, 2);
  assert.equal(result.ncols, 2);
  const expected = { road_res: 0, river_res: 2000, landscape_res: 3.125, linear_res: 9283.25,
    generic_res: genericValue, lamp_res: 0, soft_surf: 2, hard_surf: 0,
    total_res: 2000 + 3.125 + 9283.25 + genericValue };
  assert.equal(Object.keys(result).length, 11);
  for (const [field, value] of Object.entries(expected)) {
    assert.ok(result[field] instanceof Float32Array, field);
    assert.deepEqual([...result[field]], Array(4).fill(Math.fround(value)), field);
  }
  assert.equal(instances[0].lastReply.transferCount, 9);
}

test('transfers f32 copies and all nine results while preserving aliased caller inputs', async () => {
  const data = inputs();
  const snapshots = data.map(array => array.slice());
  check(await runResistancePipelineBrowserAsync(...data, JSON.stringify(params)));
  data.forEach((array, i) => assert.deepEqual(array, snapshots[i]));
  assert.ok(instances[0].lastInputTypes.every(type => type === 'Float32Array'));
});

test('preserves f64 input support and refreshes f32 memory views across repeated runs', async () => {
  for (const value of [11, 17, 23]) {
    const data = inputs(Float64Array, value);
    check(await runResistancePipelineBrowserAsync(...data, JSON.stringify(params)), value);
    assert.equal(data[5][0], value);
    assert.ok(instances[0].lastInputTypes.every(type => type === 'Float64Array'));
  }
});

test('light maps retain f32 output precision and leave the supplied light map usable', async () => {
  const data = inputs();
  const light = new Float32Array([1, 2, 1, 4]);
  const result = await runResistancePipelineBrowserWithLightmapAsync(
    ...data.slice(0, 6), light, data[7], JSON.stringify(params),
  );
  assert.deepEqual([...result.lamp_res], [25e6, 50e6, 25e6, 100e6]);
  assert.deepEqual([...result.total_res], [...result.lamp_res].map(value => Math.fround(value + 11291.375)));
  assert.deepEqual([...light], [1, 2, 1, 4]);
  assert.equal(instances[0].lastReply.transferCount, 9);
});

test('missing terrain preserves NaNs in binary outputs', async () => {
  const data = inputs();
  data[3][0] = NaN;
  const result = await runResistancePipelineBrowserAsync(...data, JSON.stringify(params));
  for (const field of ['total_res', 'lamp_res', 'landscape_res', 'linear_res', 'soft_surf', 'hard_surf']) {
    assert.ok(Number.isNaN(result[field][0]), field);
  }
  assert.equal(result.generic_res[0], 5);
  assert.equal(result.river_res[0], 2000);
});

test('parameter errors reject the request and a later valid request still succeeds', async () => {
  await assert.rejects(runResistancePipelineBrowserAsync(...inputs(), '{'),
    error => String(error).includes('Invalid params JSON'));
  check(await runResistancePipelineBrowserAsync(...inputs(), JSON.stringify(params)));
});


test('rasterizes per-feature values without layer configuration and transfers a typed raster', async () => {
  const grid = { nrows: 8, ncols: 8, xmin: 0, ymax: 8, cellsize: 1 };
  const features = {
    type: 'FeatureCollection',
    features: [
      { type: 'Feature', properties: { value: 3 }, geometry: {
        type: 'Polygon', coordinates: [[[1, 1], [5, 1], [5, 5], [1, 5], [1, 1]]],
      } },
      { type: 'Feature', properties: { value: 10 }, geometry: {
        type: 'Polygon', coordinates: [[[3, 3], [7, 3], [7, 7], [3, 7], [3, 3]]],
      } },
    ],
  };
  const raster = await rasterizeGeojsonAsync(features, grid);
  assert.ok(raster instanceof Float64Array);
  assert.equal(raster.length, 64);
  assert.equal(raster[0], 0);
  assert.equal(raster[6 * 8 + 2], 3);
  assert.equal(raster[4 * 8 + 4], 10);
  assert.equal(features.features[0].properties.value, 3);
  assert.equal(instances[0].lastReply.transferCount, 1);

  const line = { type: 'FeatureCollection', features: [{
    type: 'Feature', properties: { value: 2 },
    geometry: { type: 'LineString', coordinates: [[1, 4], [7, 4]] },
  }] };
  const thin = await rasterizeGeojsonAsync(line, grid);
  const buffered = await rasterizeGeojsonAsync(line, { ...grid, buffer: 2 });
  assert.ok(buffered.filter(v => v === 2).length > thin.filter(v => v === 2).length);
});

test('rasterizer rejects missing values and invalid grids and remains usable', async () => {
  const grid = { nrows: 8, ncols: 8, xmin: 0, ymax: 8, cellsize: 1 };
  const invalid = { type: 'FeatureCollection', features: [{
    type: 'Feature', properties: {}, geometry: {
      type: 'Polygon', coordinates: [[[1, 1], [5, 1], [5, 5], [1, 5], [1, 1]]],
    },
  }] };
  await assert.rejects(rasterizeGeojsonAsync(invalid, grid), /properties.value/);
  await assert.rejects(rasterizeGeojsonAsync(invalid, { ...grid, cellsize: 0 }), /cellsize/);
  await assert.rejects(rasterizeGeojsonAsync(invalid, { ...grid, buffer: -1 }), /buffer/);
  const empty = await rasterizeGeojsonAsync({ type: 'FeatureCollection', features: [] }, grid);
  assert.deepEqual(empty, new Float64Array(64));
});


test('solver-only public wrapper matches the current solve without geospatial inputs', async () => {
  const { solveCurrentAsync, runGeospatialPipelineMgStencilAsync } = await import('../lib/index.js');
  const nrows = 32;
  const ncols = 32;
  const resistance = new Float64Array(nrows * ncols).fill(1);
  const source = new Float64Array(nrows * ncols);
  const ground = new Float64Array(nrows * ncols);
  source[0] = 1;
  ground[ground.length - 1] = 1;
  for (const precision of ['f64', 'f32', 'mixed']) {
    for (const useDirichletGround of [false, true]) {
      const options = { precision, useDirichletGround, maxIter: 100000 };
      const result = await solveCurrentAsync(resistance, nrows, ncols, -9999, source, ground, options);
      assert.ok(!result.error, result.error);
      assert.equal(result.output.nrows, nrows);
      assert.equal(result.output.ncols, ncols);
      assert.ok(result.output.current_map.every(Number.isFinite));
      const geospatial = JSON.parse(await runGeospatialPipelineMgStencilAsync(
        resistance, nrows, ncols, -9999, 0, nrows, 1, source, ground, options,
      ));
      assert.deepEqual(result.output.current_map, geospatial.current_map);
      assert.deepEqual(result.output.voltages, geospatial.voltage_map);
      assert.equal(result.total_iters, geospatial.total_iters);
    }
  }
  const defaultResult = await solveCurrentAsync(resistance, nrows, ncols, -9999, source, ground);
  assert.ok(!defaultResult.error, defaultResult.error);
  assert.equal(source[0], 1);
  assert.equal(source[1], 0);
  assert.equal(ground[ground.length - 1], 1);
  await assert.rejects(solveCurrentAsync(resistance, nrows, ncols, -9999, source, ground,
    { precision: 'invalid' }), /precision/);
});
