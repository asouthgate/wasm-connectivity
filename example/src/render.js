const VIRIDIS_STOPS = [
  [0,    68,   1,  84],
  [0.25, 59,  82, 139],
  [0.5,  33, 144, 140],
  [0.75, 94, 201,  98],
  [1.0, 253, 231,  37],
];

const PLASMA_STOPS = [
  [0,   13,   8, 135],
  [0.25, 126, 3, 168],
  [0.5, 204, 71, 120],
  [0.75, 248,149, 64],
  [1.0, 240,249, 33],
];

export const N_CLASSES = 500;
const JET_RED = [[0, 0], [0.35, 0], [0.66, 1], [0.89, 1], [1, 0.5]];
const JET_GREEN = [[0, 0], [0.125, 0], [0.375, 1], [0.64, 1], [0.91, 0], [1, 0]];
const JET_BLUE = [[0, 0.5], [0.11, 1], [0.34, 1], [0.65, 0], [1, 0]];

function jetChannel(t, stops) {
  let i = 0;
  while (i < stops.length - 2 && t > stops[i + 1][0]) i++;
  const [t0, v0] = stops[i], [t1, v1] = stops[i + 1];
  return Math.floor(255 * Math.max(0, Math.min(1, v0 + (v1 - v0) * (t - t0) / (t1 - t0))));
}
const JET_COLORS = Array.from({ length: N_CLASSES }, (_, i) => {
  const t = i / (N_CLASSES - 1);
  return [JET_RED, JET_GREEN, JET_BLUE].map(stops => jetChannel(t, stops));
});
export const JET_GRADIENT = `linear-gradient(to right,${JET_COLORS.map(rgb => `rgb(${rgb.join(',')})`).join(',')})`;

export const VIRIDIS_GRADIENT = 'linear-gradient(to right,#440154,#3b528b,#21918c,#5ec962,#fde725)';
export const PLASMA_GRADIENT = 'linear-gradient(to right,#0d0887,#7e03a8,#cc4778,#f89540,#f0f921)';

function colorFromStops(t, stops) {
  t = Math.max(0, Math.min(1, t));
  let i = 0;
  while (i < stops.length - 2 && t > stops[i + 1][0]) i++;
  const [t0, r0, g0, b0] = stops[i];
  const [t1, r1, g1, b1] = stops[i + 1];
  const s = (t - t0) / (t1 - t0);
  return [
    Math.round(r0 + (r1 - r0) * s),
    Math.round(g0 + (g1 - g0) * s),
    Math.round(b0 + (b1 - b0) * s),
  ];
}

export function renderMap(canvas, data, nrows, ncols, nodata, logScale, maxSide = 400, palette = 'viridis', quantileScale = false) {
  const scale = Math.max(1, Math.floor(maxSide / Math.max(nrows, ncols)));
  const w = ncols * scale, h = nrows * scale;
  if (canvas.width !== w) canvas.width = w;
  if (canvas.height !== h) canvas.height = h;

  const ctx = canvas.getContext('2d');
  const img = ctx.createImageData(w, h);

  let minRaw = Infinity, maxRaw = -Infinity;
  for (const v of data) {
    if (v === nodata || !Number.isFinite(v) || ((logScale && v <= 0) || (quantileScale && v < 0))) continue;
    if (v < minRaw) minRaw = v;
    if (v > maxRaw) maxRaw = v;
  }
  if (!isFinite(minRaw)) minRaw = 0;
  if (!isFinite(maxRaw)) maxRaw = 1;
  if (maxRaw <= minRaw && !quantileScale) maxRaw = minRaw + 1;

  const span = maxRaw - minRaw;
  const logMin = minRaw > 0 ? Math.log(minRaw) : 0;
  const logSpan = maxRaw > 0 ? Math.log(maxRaw) - logMin : 1;
  const stops = palette === 'plasma' ? PLASMA_STOPS : VIRIDIS_STOPS;
  let boundaries;
  if (quantileScale) {
    let count = 0;
    for (const v of data) if (v !== nodata && Number.isFinite(v) && v >= 0) count++;
    const sorted = new Float64Array(count);
    let index = 0;
    for (const v of data) if (v !== nodata && Number.isFinite(v) && v >= 0) sorted[index++] = v;
    sorted.sort();
    boundaries = [];
    if (count) for (let i = 0; i <= N_CLASSES; i++) {
      const position = (i === N_CLASSES ? 1 : i * (1 / N_CLASSES)) * (count - 1);
      const lower = Math.floor(position), upper = Math.ceil(position);
      const fraction = position - lower;
      const difference = sorted[upper] - sorted[lower];
      // Match NumPy's linear quantile interpolation, including its rounding.
      const value = fraction < 0.5 ? sorted[lower] + difference * fraction
        : sorted[upper] - difference * (1 - fraction);
      if (boundaries.length === 0 || value !== boundaries[boundaries.length - 1]) boundaries.push(value);
    }
  }
  for (let r = 0; r < nrows; r++) {
    for (let c = 0; c < ncols; c++) {
      const v = data[r * ncols + c];
      let rr = 0, gg = 0, bb = 0;
      if (v !== nodata && Number.isFinite(v) && (!logScale || v > 0) && (!quantileScale || v >= 0)) {
        let s, jetIndex;
        if (quantileScale) {
          let lo = 0, hi = boundaries.length;
          while (lo < hi) {
            const mid = (lo + hi) >>> 1;
            if (v < boundaries[mid]) hi = mid;
            else lo = mid + 1;
          }
          const classes = boundaries.length - 1;
          const bin = Math.max(0, Math.min(classes - 1, lo - 1));
          const colorIndex = classes > 1 ? Math.floor(bin * ((N_CLASSES - 1) / (classes - 1)))
            : classes === 1 ? Math.floor((N_CLASSES - 1) / 2) : 0;
          jetIndex = colorIndex;
          s = colorIndex / (N_CLASSES - 1);
        } else if (logScale && v > 0) {
          s = (Math.log(v) - logMin) / logSpan;
        } else {
          s = (v - minRaw) / span;
        }
        if (s < 0) s = 0;
        else if (s > 1) s = 1;
        [rr, gg, bb] = palette === 'jet' ? JET_COLORS[jetIndex ?? Math.min(N_CLASSES - 1, Math.floor(s * N_CLASSES))] : colorFromStops(s, stops);
      }
      for (let dy = 0; dy < scale; dy++) {
        for (let dx = 0; dx < scale; dx++) {
          const i = ((r * scale + dy) * w + c * scale + dx) * 4;
          img.data[i] = rr; img.data[i + 1] = gg; img.data[i + 2] = bb; img.data[i + 3] = 255;
        }
      }
    }
  }
  ctx.putImageData(img, 0, 0);
  let ticks;
  if (quantileScale && boundaries.length > 1) {
    const indices = [...new Set(Array.from({ length: 4 }, (_, i) => Math.round(i * (boundaries.length - 1) / 3)))];
    ticks = indices.map(index => ({ position: index / (boundaries.length - 1), value: boundaries[index] }));
  } else if (logScale && minRaw > 0) {
    ticks = Array.from({ length: 4 }, (_, i) => ({ position: i / 3,
      value: i === 0 ? minRaw : i === 3 ? maxRaw : Math.exp(logMin + logSpan * i / 3) }));
  }
  return { min: isFinite(minRaw) ? minRaw : 0, max: isFinite(maxRaw) ? maxRaw : 1, ticks };
}
