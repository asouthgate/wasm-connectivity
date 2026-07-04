export function parseAsc(text) {
  const lines = text.split('\n').filter(l => l.trim() !== '');
  const meta = { ncols: 0, nrows: 0, xllcorner: 0, yllcorner: 0, cellsize: 1, nodata: -9999 };
  let dataStart = 0;
  const keyMap = { ncols: 1, nrows: 1, xllcorner: 1, yllcorner: 1, cellsize: 1, nodata_value: 1 };
  for (let i = 0; i < lines.length && i < 10; i++) {
    const parts = lines[i].trim().split(/\s+/);
    const k = parts[0].toLowerCase();
    if (k === 'ncols') { meta.ncols = +parts[1]; dataStart++; }
    else if (k === 'nrows') { meta.nrows = +parts[1]; dataStart++; }
    else if (k === 'xllcorner') { meta.xllcorner = +parts[1]; dataStart++; }
    else if (k === 'yllcorner') { meta.yllcorner = +parts[1]; dataStart++; }
    else if (k === 'cellsize') { meta.cellsize = +parts[1]; dataStart++; }
    else if (k === 'nodata_value') { meta.nodata = +parts[1]; dataStart++; }
    else { break; }
  }
  const vals = [];
  for (let i = dataStart; i < lines.length; i++) {
    lines[i].trim().split(/\s+/).forEach(s => { const n = +s; if (!isNaN(n)) vals.push(n); });
  }
  return { meta, data: new Float64Array(vals) };
}
