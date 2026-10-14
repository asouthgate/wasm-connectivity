import { useRef, useEffect, useState } from 'react';
import { renderMap, VIRIDIS_GRADIENT, PLASMA_GRADIENT } from '../render';

export default function MapView({ type, data, meta, logScale, onToggleScale, preview = false, maxSide = 400, label, showHeader = true }) {
  const canvasRef = useRef(null);
  const [range, setRange] = useState({ min: 0, max: 1 });
  const isCurVolt = type === 'cur' || type === 'volt';

  useEffect(() => {
    if (!data || !meta) return;
    const canvas = canvasRef.current;
    const r = renderMap(canvas, data, meta.nrows, meta.ncols, meta.nodata ?? 0, logScale, maxSide, isCurVolt);
    if (r) setRange(r);
  }, [data, meta, type, logScale, isCurVolt, maxSide]);

  const labels = { res:'Resistance', points:'Focal Points', cur:'Current', src:'Source', gnd:'Ground', volt:'Voltage' };
  const dim = meta ? `${meta.ncols}×${meta.nrows}` : '';
  const fmt = (v) => v === 0 ? '0.0' : Math.abs(v) < 0.01 || Math.abs(v) >= 1000 ? v.toExponential(1) : v.toPrecision(3);

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
      <canvas ref={canvasRef} className="map-canvas" />
      <div className="map-legend">
        <span className="map-legend-label">{fmt(range.min)}</span>
        <div className="map-legend-gradient" style={{ '--legend-bg': isCurVolt ? PLASMA_GRADIENT : VIRIDIS_GRADIENT }} />
        <span className="map-legend-label">{fmt(range.max)}</span>
      </div>
    </div>
  );
}
