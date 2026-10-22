import { readFile, mkdir, open } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { parseAsc } from '../../lib/parseAsc.js';
import { SCENARIOS, scenarioResistance, scenarioMasks } from '../../example/src/scenarios.js';

const root = new URL('../../', import.meta.url);
const input = new URL('example/public/geodata/scenarios/', root);
const output = new URL('scripts/scenarios/', root);

async function load(name) {
  const raster = parseAsc(await readFile(new URL(`${name}.asc`, input), 'utf8'));
  if (raster.meta.nrows !== 2000 || raster.meta.ncols !== 2000 || raster.data.length !== 4_000_000) {
    throw new Error(`${name} must contain a complete 2000×2000 raster`);
  }
  return raster;
}

async function writeRaster(path, data, meta) {
  const file = await open(path, 'w');
  try {
    await file.write(`ncols ${meta.ncols}\nnrows ${meta.nrows}\nxllcorner ${meta.xllcorner}\nyllcorner ${meta.yllcorner}\ncellsize ${meta.cellsize}\nNODATA_value ${meta.nodata}\n`);
    for (let row = 0; row < meta.nrows; row++) {
      await file.write(`${data.subarray(row * meta.ncols, (row + 1) * meta.ncols).join(' ')}\n`);
    }
  } finally {
    await file.close();
  }
}

for (const scenario of SCENARIOS) {
  const { data, meta } = await load(scenario.id);
  const rasters = {};
  for (const name of scenario.inputs || []) {
    const raster = await load(name);
    for (const key of ['xllcorner', 'yllcorner', 'cellsize']) {
      if (raster.meta[key] !== meta[key]) throw new Error(`${name} does not align with ${scenario.id}`);
    }
    rasters[name] = raster;
  }
  scenarioResistance(scenario.id, data, meta.nodata);
  const { source, ground } = scenarioMasks(scenario.id, data, meta.nrows, meta.ncols, meta.nodata, rasters);
  for (let i = 0; i < data.length; i++) {
    if (!Number.isFinite(data[i]) || data[i] <= 0 || data[i] === meta.nodata) data[i] = meta.nodata;
  }
  const directory = new URL(`${scenario.id}/`, output);
  await mkdir(new URL('output/', directory), { recursive: true });
  for (const [name, values] of [['resistance', data], ['source', source], ['ground', ground]]) {
    await writeRaster(new URL(`${name}.asc`, directory), values, meta);
  }
  console.log(`Prepared ${scenario.id}: ${fileURLToPath(directory)}`);
}
