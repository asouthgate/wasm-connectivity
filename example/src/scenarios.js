export const SCENARIOS = [
  { id: 'dsm', name: 'Digital surface model', description: 'DSM < 20 sources -> DSM > 100 ground' },
  { id: 'linear_res', name: 'Linear resistance', description: 'Resistance = 1/(1 + L) - DSM > 110 source strengths - hard surface > 1 ground', inputs: ['dsm', 'hard_surf'] },
  { id: 'point_lights', name: 'Point lights', description: 'Sources: hard surface > 0, strength = hard-surface value; ground: DSM > 125', inputs: ['hard_surf', 'dsm'] },
];

export function scenarioResistance(id, data, nodata) {
  if (id === 'linear_res') {
    for (let i = 0; i < data.length; i++) {
      if (Number.isFinite(data[i]) && data[i] >= 0 && data[i] !== nodata) {
        data[i] = 1 / (1 + data[i]);
      }
    }
  }
  return data;
}

export function scenarioMasks(id, resistance, nrows, ncols, nodata, rasters = {}) {
  for (const name of SCENARIOS.find(s => s.id === id)?.inputs || []) {
    if (rasters[name]?.data.length !== resistance.length) throw new Error(`Missing or incomplete ${name} raster`);
  }
  const source = new Float64Array(resistance.length);
  const ground = new Float64Array(resistance.length);
  const positiveValue = (raster, i) => {
    const value = raster.data[i];
    return Number.isFinite(value) && value > 0 && value !== raster.meta.nodata ? value : 0;
  };
  for (let row = 0; row < nrows; row++) {
    for (let col = 0; col < ncols; col++) {
      const i = row * ncols + col;
      if (!Number.isFinite(resistance[i]) || resistance[i] <= 0 || resistance[i] === nodata) continue;
      if (id === 'dsm') {
        source[i] = +(resistance[i] < 20);
        ground[i] = +(resistance[i] > 100);
      } else if (id === 'linear_res') {
        const dsm = positiveValue(rasters.dsm, i);
        source[i] = dsm > 110 ? dsm : 0;
        ground[i] = +(positiveValue(rasters.hard_surf, i) > 1);
      } else if (id === 'point_lights') {
        source[i] = positiveValue(rasters.hard_surf, i);
        ground[i] = +(positiveValue(rasters.dsm, i) > 125);
      } else {
        throw new Error(`Unknown scenario: ${id}`);
      }
    }
  }
  if (!source.some(v => v > 0) || !ground.some(v => v > 0)) throw new Error('Scenario has no conductive source or ground cells');
  return { source, ground };
}
