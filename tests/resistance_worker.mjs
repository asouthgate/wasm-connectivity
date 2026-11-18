import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import { readFile } from 'node:fs/promises';
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
  import { parentPort, workerData, Worker as ThreadWorker } from 'node:worker_threads';
  import { register } from 'node:module';
  import { readFile } from 'node:fs/promises';
  register(workerData.loaderUrl, workerData.workerUrl);
  globalThis.fetch = async url => new Response(await readFile(new URL(url)));
  globalThis.crossOriginIsolated = true;
  globalThis.Worker = class {
    constructor(url) {
      this.thread = new ThreadWorker(new URL(workerData.bootstrapUrl), {
        workerData: { ...workerData, workerUrl: String(url) },
      });
      this.thread.on('message', data => this.onmessage?.({ data }));
      this.thread.on('error', error => this.onerror?.(error));
    }
    postMessage(message) { this.thread.postMessage(message); }
    terminate() { this.thread.terminate(); }
  };
  globalThis.self = { location: new URL(workerData.workerUrl), postMessage(message, transfer = []) {
    parentPort.postMessage({ ...message, transferCount: transfer.length }, transfer);
    if (transfer.some(buffer => buffer.byteLength !== 0)) throw new Error('Output transfer did not detach');
  }};
  await import(workerData.workerUrl);
  parentPort.on('message', data => self.onmessage({ data }));
`;
const bootstrapUrl = 'data:text/javascript,' + encodeURIComponent(bootstrap);
const instances = [];
globalThis.Worker = class {
  constructor(url) {
    this.thread = new ThreadWorker(new URL(bootstrapUrl), {
      workerData: { workerUrl: url.href, loaderUrl, bootstrapUrl },
    });
    this.thread.on('message', data => { this.lastReply = data; this.onmessage?.({ data }); });
    this.thread.on('error', error => this.onerror?.({ message: error.message, error }));
    instances.push(this);
  }
  terminate() { this.thread.terminate(); }
  postMessage(message, transfer) {
    this.lastInputTypes = message.fn === 'run_resistance_pipeline_browser'
      ? Object.values(message.args[1]).map(input => input.constructor.name) : [];
    this.thread.postMessage(message, transfer);
    assert.ok(transfer.every(buffer => buffer.byteLength === 0), 'Input copies must transfer');
  }
};
after(async () => { await Promise.all(instances.map(worker => worker.thread.terminate())); });
const { rasterizeGeojsonAsync, runResistancePipelineBrowserAsync } =
  await import('../lib/wasm.js');
const model = JSON.parse(await readFile(new URL('../example/horseshoe-bat-model.json', import.meta.url)));
const grid = { nrows: 2, ncols: 2, cellsize: 1 };
function inputs(Type = Float32Array, genericValue = 5) {
  const noFeatures = new Type(4);
  return { roads: noFeatures, rivers: noFeatures, buildings: new Type(4),
    dtm: new Type(4).fill(10), dsm: new Type(4).fill(12),
    generic: new Type(4).fill(genericValue), lamps: new Type(0), landcover: new Type(4).fill(4) };
}
function run(data = inputs(), definition = model, dimensions = grid) {
  return runResistancePipelineBrowserAsync(JSON.stringify(definition), data, dimensions);
}
function check(result, genericValue = 5) {
  assert.equal(result.nrows, 2);
  assert.equal(result.ncols, 2);
  const expected = { road_res: 0, river_res: 2000, landscape_res: 3.125,
    linear_res: 9283.25, lamp_res: 0 };
  for (const [field, value] of Object.entries(expected)) {
    assert.ok(result.layers[field].data instanceof Float32Array, field);
    assert.deepEqual([...result.layers[field].data], Array(4).fill(Math.fround(value)), field);
  }
  assert.deepEqual([...result.layers.surfaces.translucent], [2, 2, 2, 2]);
  assert.deepEqual([...result.layers.surfaces.opaque], [0, 0, 0, 0]);
  assert.deepEqual([...result.output], Array(4).fill(Math.fround(11286.375 + genericValue)));
  assert.ok(!Object.hasOwn(result.layers, "total_res"));
  const rasters = Object.values(result.layers).flatMap(Object.values);
  assert.equal(instances[0].lastReply.transferCount, rasters.length + 1);
}

test('transfers f32 copies and named layers while preserving aliased caller inputs', async () => {
  const data = inputs();
  const snapshots = Object.fromEntries(Object.entries(data).map(([name, array]) => [name, array.slice()]));
  check(await run(data));
  for (const name of Object.keys(data)) assert.deepEqual(data[name], snapshots[name]);
  assert.ok(instances[0].lastInputTypes.every(type => type === 'Float32Array'));
});

test('preserves f64 input support across repeated runs', async () => {
  for (const value of [11, 17, 23]) {
    const data = inputs(Float64Array, value);
    check(await run(data), value);
    assert.equal(data.generic[0], value);
    assert.ok(instances[0].lastInputTypes.every(type => type === 'Float64Array'));
  }
});

test('the browser can return resistance at solver precision', async () => {
  const definition = { name: 'precision', inputs: { values: { type: 'raster_ranked' } },
    nodes: {}, output: 'inputs.values' };
  const values = new Float64Array([1.0000000001, 2, 3, 4]);
  const result = await runResistancePipelineBrowserAsync(JSON.stringify(definition), { values }, grid,
    { allLayers: false, outputPrecision: 'f64' });
  assert.deepEqual(result.output, values);
});

test('precomputed irradiance is expressed by the model instead of a separate execution path', async () => {
  const definition = structuredClone(model);
  delete definition.inputs.lamps;
  delete definition.nodes.irradiance;
  definition.inputs.light = { type: 'raster_ranked' };
  definition.nodes.lamp_res.input = 'inputs.light';
  const data = inputs();
  delete data.lamps;
  data.light = new Float32Array([1, 2, 1, 4]);
  const result = await run(data, definition);
  assert.deepEqual([...result.layers.lamp_res.data], [25e6, 50e6, 25e6, 100e6]);
  assert.deepEqual([...result.output], [...result.layers.lamp_res.data].map(value => Math.fround(value + 11291.375)));
  assert.deepEqual([...data.light], [1, 2, 1, 4]);
});

test('missing terrain preserves NaNs in binary outputs', async () => {
  const data = inputs();
  data.dtm[0] = NaN;
  const result = await run(data);
  assert.ok(Number.isNaN(result.output[0]));
  for (const field of ['lamp_res', 'landscape_res', 'linear_res']) {
    assert.ok(Number.isNaN(result.layers[field].data[0]), field);
  }
  assert.ok(Number.isNaN(result.layers.surfaces.translucent[0]));
  assert.ok(Number.isNaN(result.layers.surfaces.opaque[0]));
  assert.equal(result.layers.river_res.data[0], 2000);
});

test('executes arbitrary models, parameter changes, named ports, and direct input outputs', async () => {
  const definition = { name: 'custom', inputs: { habitat: { type: 'raster_ranked' } },
    nodes: { changed: { op: 'power_response', inputs: 'inputs.habitat', parameters: {
      input_range: [0, 4], base_range: [0, 1], exponent: 1, scale: 8, offset: 1,
    } } }, output: 'nodes.changed' };
  const data = { habitat: new Float64Array([0, 1, 2, 4]) };
  assert.deepEqual([...(await run(data, definition)).output], [1, 3, 5, 9]);
  definition.nodes.changed.parameters.scale = 4;
  assert.deepEqual([...(await run(data, definition)).output], [1, 2, 3, 5]);
  definition.output = 'inputs.habitat';
  assert.deepEqual([...(await run(data, definition)).output], [0, 1, 2, 4]);
  definition.nodes = {};
  assert.deepEqual((await run(data, definition)).layers, {});
  const portModel = structuredClone(model);
  portModel.output = 'nodes.surfaces.translucent';
  assert.deepEqual([...(await run(inputs(), portModel)).output], [2, 2, 2, 2]);
});

test('WASM matches saved native results with lights, distances, masks, and non-unit cells', async () => {
  const nrows = 13, ncols = 17, size = nrows * ncols, cellsize = 2;
  const data = {
    dtm: new Float64Array(size).fill(100),
    dsm: Float64Array.from({ length: size }, (_, i) => 100 + [0, 1, 2, 3, 4, 6, 8][i % 7]),
    buildings: new Float64Array(size),
    landcover: Float64Array.from({ length: size }, (_, i) => i % 23),
    generic: new Float64Array(size).fill(3),
    lamps: new Float64Array([4, 5, 8, 12, 8, 5]),
  };
  data.buildings.set([5, 4, 2], 10);
  data.dsm[10] = NaN;
  data.dtm[11] = NaN;
  data.landcover[12] = NaN;
  data.dtm[13] = NaN;
  data.dsm[14] = NaN;
  data.generic[17] = NaN;
  data.roads = new Float64Array(size);
  data.roads[0] = 1;
  data.roads[size - 1] = 2;
  data.rivers = new Float64Array(size);
  data.rivers[ncols + 2] = 1;
  data.roads[15] = NaN;
  data.rivers[16] = NaN;
  const result = await run(data, model, { nrows, ncols, cellsize });
  const snapshots = JSON.parse(await readFile(new URL('../src/pipeline/model/fixtures/reference-resistance.json', import.meta.url)));
  // Snapshot 5 is the second scenario at two-metre resolution.
  for (const [name, expected] of Object.entries(snapshots[5])) {
    const [node, port = 'data'] = name.split('.');
    const actual = node === "total_res" ? result.output : result.layers[node][port];
    for (let i = 0; i < size; i++) {
      if (expected[i] === null) assert.ok(Number.isNaN(actual[i]), `${name}[${i}]`);
      else assert.equal(actual[i], Math.fround(expected[i]), `${name}[${i}]`);
    }
  }
});

test('model and input errors reject the request and later valid requests still succeed', async () => {
  await assert.rejects(runResistancePipelineBrowserAsync('{', inputs(), grid),
    error => String(error).includes('Invalid model JSON'));
  for (const [data, definition, dimensions, message] of [
    [{ ...inputs(), extra: new Float32Array(4) }, model, grid, 'undeclared model input'],
    [{ ...inputs(), roads: new Float32Array(3) }, model, grid, 'dimensions'],
    [{ ...inputs(), lamps: new Float32Array([1, 2]) }, model, grid, 'dimensions'],
    [inputs(), { ...model, output: 'nodes.absent' }, grid, 'unknown node'],
    [inputs(), model, { ...grid, cellsize: 0 }, 'invalid model grid'],
  ]) {
    await assert.rejects(run(data, definition, dimensions), error => String(error).includes(message));
  }
  const missing = inputs();
  delete missing.dtm;
  await assert.rejects(run(missing), error => String(error).includes('missing input: dtm'));
  check(await run());
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


test('current solver wrapper supports each precision and ground mode', async () => {
  const { solveCurrentAsync } = await import('../lib/index.js');
  const nrows = 32;
  const ncols = 32;
  const resistance = new Float64Array(nrows * ncols).fill(1);
  const source = new Float64Array(nrows * ncols);
  const ground = new Float64Array(nrows * ncols);
  source[0] = 1;
  ground[ground.length - 1] = 1;
  for (const precision of ['f64', 'f32', 'mixed']) {
    for (const useDirichletGround of [false, true]) {
      for (const lowMemory of precision === 'mixed' ? [true] : [true, false]) {
        const options = { precision, lowMemory, useDirichletGround, maxIter: 100000 };
        const result = await solveCurrentAsync(resistance, nrows, ncols, -9999, source, ground, options);
        assert.ok(!result.error, result.error);
        assert.equal(result.output.nrows, nrows);
        assert.equal(result.output.ncols, ncols);
        assert.ok(result.output.current_map.every(Number.isFinite));
      }
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


test('building height rasters use zero outside and a one-metre candidate overrides missing DSM', async () => {
  const grid = { nrows: 2, ncols: 2, xmin: 0, ymax: 2, cellsize: 1 };
  const buildings = await rasterizeGeojsonAsync({ type: 'FeatureCollection', features: [{
    type: 'Feature', properties: { value: 1 }, geometry: {
      type: 'Polygon', coordinates: [[[0,1],[1,1],[1,2],[0,2],[0,1]]],
    },
  }] }, grid);
  assert.equal(buildings[0], 1);
  assert.equal(buildings[1], 0);
  const data = inputs();
  data.buildings = buildings;
  data.dsm[0] = NaN;
  const result = await run(data);
  assert.equal(result.layers.surfaces.opaque[0], 1);
  assert.equal(result.layers.surfaces.translucent[0], 0);
  assert.ok(Number.isFinite(result.output[0]));
  assert.equal(result.layers.surfaces.opaque[1], 0);
  assert.equal(result.layers.surfaces.translucent[1], 2);
});

test('sampled browser runs return resistance and current statistics without retaining samples', async () => {
  const { runResistancePipelineBrowserAsync, solveCurrentAsync } = await import('../lib/index.js');
  const definition = { name: 'sampled', inputs: { data: { type: 'raster_ranked' } },
    nodes: { resistance: { op: 'power_response', inputs: 'inputs.data', parameters: {
      input_range: [1, 4], base_range: [1, 4],
      exponent: { sample: { values: [1, 2, 3] } }, scale: 1, offset: 0,
    } } }, output: 'nodes.resistance' };
  const source = new Float64Array([1, 0, 0]);
  const ground = new Float64Array([0, 1, 1]);
  const data = new Float64Array([1, 2, 4]);
  const options = { samples: 3, allLayers: true, currentSolverOptions: { source, ground } };
  const result = await runResistancePipelineBrowserAsync(JSON.stringify(definition), { data },
    { nrows: 1, ncols: 3, cellsize: 1 }, options);
  assert.equal(result.samples, 3);
  assert.deepEqual([...result.resistance.mean], [1, Math.fround(14 / 3), 28]);
  assert.deepEqual([...result.resistance.variance], [0, Math.fround(28 / 3), 1008]);
  assert.ok(!Object.hasOwn(result.layers, "resistance"));
  const wasm = await import('../lib/lmConnect.js');
  const runWithThreads = threads => runResistancePipelineBrowserAsync(
    JSON.stringify(definition), { data }, { nrows: 1, ncols: 3, cellsize: 1 }, { ...options, threads },
  );
  if (typeof wasm.initialize_pipeline_threads === 'function') {
    await assert.rejects(runResistancePipelineBrowserAsync(
      '{', { data }, { nrows: 1, ncols: 3, cellsize: 1 }, { ...options, threads: 2 },
    ), /Invalid model JSON/);
    for (const threadCount of [2, 4, 2]) {
      assert.deepEqual(await runWithThreads(threadCount), result);
    }
  } else {
    await assert.rejects(runWithThreads(2), /Wasm thread support requires/);
  }
  await assert.rejects(runWithThreads(0), /threads must be a positive/);
  const currents = [];
  for (const exponent of [1, 2, 3]) {
    const resistance = Float64Array.from(data, v => v ** exponent);
    const solved = await solveCurrentAsync(resistance, 1, 3, -9999, source, ground, { tol: 1e-4 });
    assert.ok(!solved.error, solved.error);
    currents.push(solved.output.current_map);
  }
  for (let i = 0; i < 3; i++) {
    const mean = currents.reduce((sum, map) => sum + map[i], 0) / 3;
    const variance = currents.reduce((sum, map) => sum + (map[i] - mean) ** 2, 0) / 2;
    assert.equal(result.current.mean[i], Math.fround(mean));
    assert.equal(result.current.variance[i], Math.fround(variance));
  }
  assert.deepEqual([...source], [1, 0, 0]);
  assert.deepEqual([...ground], [0, 1, 1]);
  assert.deepEqual([...data], [1, 2, 4]);
  definition.nodes.resistance.parameters.exponent = { sample: { values: [2] } };
  const single = await runResistancePipelineBrowserAsync(JSON.stringify(definition), { data },
    { nrows: 1, ncols: 3, cellsize: 1 }, { samples: 1, allLayers: false, currentSolverOptions: { source, ground } });
  assert.deepEqual([...single.output], [1, 4, 16]);
  assert.deepEqual(single.current, Float32Array.from(currents[1]));
  assert.deepEqual(single.layers, {});
  await assert.rejects(runResistancePipelineBrowserAsync(JSON.stringify(definition), { data },
    { nrows: 1, ncols: 3, cellsize: 1 }, { samples: 0 }), /positive integer/);
});
