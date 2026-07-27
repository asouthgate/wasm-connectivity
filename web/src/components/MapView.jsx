import { useRef, useEffect, useState } from 'react';
import { renderMap, VIRIDIS_GRADIENT, PLASMA_GRADIENT } from '../lib/render';

export default function MapView({ type, data, meta, logScale, onToggleScale }) {
  const canvasRef = useRef(null);
  const [range, setRange] = useState({ min: 0, max: 1 });
  const isCurVolt = type === 'cur' || type === 'volt';

  useEffect(() => {
    if (!data || !meta) return;
    const canvas = canvasRef.current;
    const r = renderMap(canvas, data, meta.nrows, meta.ncols, meta.nodata || 0, logScale, 400, isCurVolt);
    if (r) setRange(r);
  }, [data, meta, type, logScale, isCurVolt]);

  const labels = { res:'Resistance', points:'Focal Points', cur:'Current', src:'Source', gnd:'Ground', volt:'Voltage' };
  const dim = meta ? `${meta.ncols}×${meta.nrows}` : '';
  const fmt = (v) => Math.abs(v) < 0.01 ? v.toExponential(1) : Math.abs(v) >= 1000 ? v.toFixed(0) : v.toPrecision(3);

  return (
    <div className="map-view">
      <div className="map-header">
        <span>{labels[type] || type} {dim && dim}</span>
        {onToggleScale && (
          <div className="map-scale-toggles">
            {['log', 'lin'].map(s => {
              const isActive = logScale === (s === 'log');
              return (
                <span key={s} onClick={() => onToggleScale(s)}
                  className={`map-scale-btn ${isActive ? 'map-scale-btn--active' : ''}`}>
                  {s}
                </span>
              );
            })}
          </div>
        )}
      </div>
      <canvas ref={canvasRef} className="map-canvas" />
      <div className="map-legend">
        <span className="map-legend-label map-legend-label--right">{fmt(range.min)}</span>
        <div className="map-legend-gradient" style={{ '--legend-bg': isCurVolt ? PLASMA_GRADIENT : VIRIDIS_GRADIENT }} />
        <span className="map-legend-label">{fmt(range.max)}</span>
      </div>
    </div>
  );
}
