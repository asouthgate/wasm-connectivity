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

export function renderMap(canvas, data, nrows, ncols, nodata, logScale, maxSide = 400, plasma = false) {
  const scale = Math.max(1, Math.floor(maxSide / Math.max(nrows, ncols)));
  const w = ncols * scale, h = nrows * scale;
  if (canvas.width !== w) canvas.width = w;
  if (canvas.height !== h) canvas.height = h;

  const ctx = canvas.getContext('2d');
  const img = ctx.createImageData(w, h);

  let minV = Infinity, maxV = -Infinity;
  let minRaw = Infinity, maxRaw = -Infinity;
  for (const v of data) {
    if (v === nodata || isNaN(v)) continue;
    if (v < minRaw) minRaw = v;
    if (v > maxRaw) maxRaw = v;
    const t = logScale && v > 0 ? Math.log10(v) : v;
    if (t < minV) minV = t;
    if (t > maxV) maxV = t;
  }
  if (!isFinite(minV)) minV = 0;
  if (!isFinite(maxV)) maxV = 1;
  if (maxV <= minV) maxV = minV + 1;

  const min = minV, max = maxV;
  const stops = plasma ? PLASMA_STOPS : VIRIDIS_STOPS;
  for (let r = 0; r < nrows; r++) {
    for (let c = 0; c < ncols; c++) {
      const v = data[r * ncols + c];
      let rr = 17, gg = 17, bb = 17;
      if (v !== nodata && !isNaN(v)) {
        const t = logScale && v > 0 ? Math.log10(v) : v;
        const s = (t - min) / (max - min);
        [rr, gg, bb] = colorFromStops(s, stops);
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
  return { min: isFinite(minRaw) ? minRaw : 0, max: isFinite(maxRaw) ? maxRaw : 1 };
}
