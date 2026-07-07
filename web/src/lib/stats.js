export function downloadCSV(filename, headers, rows) {
  const b = new Blob([headers + rows], { type: 'text/csv' });
  const u = URL.createObjectURL(b);
  const a = document.createElement('a'); a.href = u; a.download = filename; a.click();
  URL.revokeObjectURL(u);
}
