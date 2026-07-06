export function stat(arr) {
  if (arr.length === 0) return { n: 0 };
  const s = [...arr].sort((a, b) => a - b);
  const sum = s.reduce((a, b) => a + b, 0);
  const mean = sum / s.length;
  const med = s.length % 2 === 0 ? (s[s.length / 2 - 1] + s[s.length / 2]) / 2 : s[Math.floor(s.length / 2)];
  const v = s.reduce((a, b) => a + (b - mean) ** 2, 0) / s.length;
  return { n: s.length, mean, median: med, min: s[0], max: s[s.length - 1], stddev: Math.sqrt(v) };
}

export function downloadCSV(filename, headers, rows) {
  const b = new Blob([headers + rows], { type: 'text/csv' });
  const u = URL.createObjectURL(b);
  const a = document.createElement('a'); a.href = u; a.download = filename; a.click();
  URL.revokeObjectURL(u);
}
