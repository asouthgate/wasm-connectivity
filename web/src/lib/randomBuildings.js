// Random square building generator shared by the Geospatial warm-start
// button and the Benchmark "hot" phase. Each call produces n squares,
// each covering approximately `fraction` of the raster width/height in
// pixels, placed uniformly at random inside the raster bounds. Repeated
// calls produce fresh random placements.

function rand(seed) {
  // Mulberry32 — deterministic PRNG so seeds produce reproducible
  // placements, useful for benchmark reproducibility.
  let s = seed >>> 0;
  return function () {
    s = (s + 0x6D2B79F5) | 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/**
 * Build a GeoJSON FeatureCollection string of `n` square polygons,
 * placed at random positions inside the raster bounds.
 *
 * @param {number} n          number of squares
 * @param {number} nrows      raster rows
 * @param {number} ncols      raster cols
 * @param {number} xmin       raster xllcorner (geographic)
 * @param {number} ymax       raster ymax (geographic)
 * @param {number} cellsize   raster cell size (geographic units)
 * @param {number} fraction   if < 1: square side as fraction of min(nrows,ncols);
 *                            if >= 1: absolute pixel count.
 * @param {number} seed       PRNG seed for reproducibility
 * @returns {string}          GeoJSON FeatureCollection string
 */
export function randomSquareBuildings(n, nrows, ncols, xmin, ymax, cellsize, fraction = 0.1, seed = Date.now() & 0xffffffff) {
  const rnd = rand(seed);
  const sidePx = fraction >= 1
    ? Math.min(Math.floor(fraction), Math.min(nrows, ncols))
    : Math.max(1, Math.round(fraction * Math.min(nrows, ncols)));
  const sideGeo = sidePx * cellsize;

  const features = [];
  for (let i = 0; i < n; i++) {
    const colStart = Math.floor(rnd() * (ncols - sidePx));
    const rowStart = Math.floor(rnd() * (nrows - sidePx));
    const ox = xmin + colStart * cellsize;
    const oy = ymax - rowStart * cellsize;
    const x0 = ox, y0 = oy;
    const x1 = ox + sideGeo, y1 = oy;
    const x2 = ox + sideGeo, y2 = oy - sideGeo;
    const x3 = ox, y3 = oy - sideGeo;
    const coords = [[[x0, y0], [x1, y1], [x2, y2], [x3, y3], [x0, y0]]];
    features.push({
      type: 'Feature',
      properties: { layer: 'buildings', _random_warm: true },
      geometry: { type: 'Polygon', coordinates: coords },
    });
  }
  return JSON.stringify({ type: 'FeatureCollection', features });
}

/**
 * Merge two GeoJSON FeatureCollection strings into one. Used to splice
 * the random buildings into the existing feature set.
 */
export function mergeGeoJson(fcStrA, fcStrB) {
  let a = { type: 'FeatureCollection', features: [] };
  let b = { type: 'FeatureCollection', features: [] };
  try { a = JSON.parse(fcStrA || '{}'); } catch (_) {}
  try { b = JSON.parse(fcStrB || '{}'); } catch (_) {}
  if (!Array.isArray(a.features)) a.features = [];
  if (!Array.isArray(b.features)) b.features = [];
  return JSON.stringify({ type: 'FeatureCollection', features: a.features.concat(b.features) });
}