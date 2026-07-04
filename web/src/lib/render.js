const POINT_COLORS = [
  [255, 50, 50], [50, 200, 50], [50, 100, 255],
  [255, 200, 50], [255, 50, 255], [50, 255, 255],
];

export function heat(t) {
  t = Math.max(0, Math.min(1, t));
  let r, g, b;
  if (t < 0.125) { const s = t / 0.125; r = 0; g = 0; b = 128 + s * 127; }
  else if (t < 0.375) { const s = (t - 0.125) / 0.25; r = 0; g = s * 255; b = 255; }
  else if (t < 0.625) { const s = (t - 0.375) / 0.25; r = 0; g = 255; b = (1 - s) * 255; }
  else if (t < 0.875) { const s = (t - 0.625) / 0.25; r = s * 255; g = (1 - s) * 255; b = 0; }
  else { const s = (t - 0.875) / 0.125; r = 255; g = (1 - s) * 128; b = 0; }
  return [r, g, b];
}

export function renderMap(canvas, data, nrows, ncols, nodata, logScale, maxSide = 400) {
  const scale = Math.max(1, Math.floor(maxSide / Math.max(nrows, ncols)));
  const w = ncols * scale, h = nrows * scale;
  if (canvas.width !== w) canvas.width = w;
  if (canvas.height !== h) canvas.height = h;

  const ctx = canvas.getContext('2d');
  const img = ctx.createImageData(w, h);

  let minV = Infinity, maxV = -Infinity;
  for (const v of data) {
    if (v === nodata || isNaN(v)) continue;
    const t = logScale && v > 0 ? Math.log10(v) : v;
    if (t < minV) minV = t;
    if (t > maxV) maxV = t;
  }
  if (!isFinite(minV)) minV = 0;
  if (!isFinite(maxV)) maxV = 1;
  if (maxV <= minV) maxV = minV + 1;

  const min = minV, max = maxV;
  for (let r = 0; r < nrows; r++) {
    for (let c = 0; c < ncols; c++) {
      const v = data[r * ncols + c];
      let rr = 17, gg = 17, bb = 17;
      if (v !== nodata && !isNaN(v)) {
        const t = logScale && v > 0 ? Math.log10(v) : v;
        const s = (t - min) / (max - min);
        [rr, gg, bb] = heat(s);
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
}

export function renderPoints(canvas, data, nrows, ncols, nodata, maxSide = 400) {
  const scale = Math.max(1, Math.floor(maxSide / Math.max(nrows, ncols)));
  const w = ncols * scale, h = nrows * scale;
  if (canvas.width !== w) canvas.width = w;
  if (canvas.height !== h) canvas.height = h;

  const ctx = canvas.getContext('2d');
  const img = ctx.createImageData(w, h);

  for (let r = 0; r < nrows; r++) {
    for (let c = 0; c < ncols; c++) {
      const v = data[r * ncols + c];
      let rr = 17, gg = 17, bb = 17;
      if (v !== nodata && v > 0 && !isNaN(v)) {
        const col = POINT_COLORS[(v - 1) % POINT_COLORS.length];
        rr = col[0]; gg = col[1]; bb = col[2];
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
}
