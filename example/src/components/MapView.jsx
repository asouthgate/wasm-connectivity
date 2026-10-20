import { useRef, useEffect, useState } from 'react';
import { renderMap, VIRIDIS_GRADIENT, PLASMA_GRADIENT, JET_GRADIENT } from '../render';

export default function MapView({ type, data, meta, image, logScale, quantileScale = false, onToggleScale, preview = false, maxSide = 400, label, showHeader = true }) {
  const canvasRef = useRef(null);
  const [range, setRange] = useState({ min: 0, max: 1 });
  const palette = (type === 'cur' || quantileScale) ? 'jet' : type === 'volt' ? 'plasma' : 'viridis';

  useEffect(() => {
    if (!data || !meta) return;
    const canvas = canvasRef.current;
    const rendered = preview ? document.createElement('canvas') : canvas;
    const r = renderMap(rendered, data, meta.nrows, meta.ncols, meta.nodata ?? 0, logScale, maxSide, palette, quantileScale);
    if (preview) {
      const scale = Math.min(1, maxSide / Math.max(rendered.width, rendered.height));
      canvas.width = Math.max(1, Math.round(rendered.width * scale));
      canvas.height = Math.max(1, Math.round(rendered.height * scale));
      const ctx = canvas.getContext('2d');
      ctx.imageSmoothingEnabled = true;
      ctx.imageSmoothingQuality = 'high';
      ctx.drawImage(rendered, 0, 0, canvas.width, canvas.height);
    }
    if (r) setRange(r);
  }, [data, meta, type, logScale, quantileScale, palette, maxSide, preview]);

  const labels = { res:'Resistance', points:'Focal Points', cur:'Current', src:'Source', gnd:'Ground', volt:'Voltage' };
  const dim = meta ? `${meta.ncols}×${meta.nrows}` : '';
  const fmt = (v) => v === 0 ? '0.0' : Math.abs(v) < 0.01 || Math.abs(v) >= 1000 ? v.toExponential(1) : v.toPrecision(3);
  const displayRange = image || range;

  return (
    <div className="map-view">
      {showHeader && <div className="map-header">
        <span>{label || labels[type] || type}{preview ? ' preview' : ''} {dim && dim}</span>
        {onToggleScale && (
          <select aria-label={`${label || labels[type] || type} scale`} value={logScale ? 'log' : 'lin'}
            onChange={e => onToggleScale(e.target.value)}>
            <option value="log">log</option>
            <option value="lin">lin</option>
          </select>
        )}
      </div>}
      {image ? <img src={image.src} alt={label || labels[type] || type} className="map-canvas" /> :
        <canvas ref={canvasRef} className="map-canvas" />}
      <div className="map-legend">
        <span className="map-legend-label">{fmt(displayRange.min)}</span>
        <div className="map-legend-gradient" style={{ '--legend-bg': palette === 'jet' ? JET_GRADIENT : palette === 'plasma' ? PLASMA_GRADIENT : VIRIDIS_GRADIENT }} />
        <span className="map-legend-label">{fmt(displayRange.max)}</span>
      </div>
    </div>
  );
}
