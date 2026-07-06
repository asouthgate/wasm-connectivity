import { useRef, useEffect, useState } from 'react';
import { renderMap, renderPoints, VIRIDIS_GRADIENT, PLASMA_GRADIENT } from '../lib/render';

export default function MapView({ type, data, meta, logScale, onToggleScale }) {
  const canvasRef = useRef(null);
  const [range, setRange] = useState({ min: 0, max: 1 });
  const isCurVolt = type === 'cur' || type === 'volt';

  useEffect(() => {
    if (!data || !meta) return;
    const canvas = canvasRef.current;
    if (type === 'points') {
      renderPoints(canvas, data, meta.nrows, meta.ncols, meta.nodata);
    } else {
      const r = renderMap(canvas, data, meta.nrows, meta.ncols, meta.nodata || 0, logScale, 400, isCurVolt);
      if (r) setRange(r);
    }
  }, [data, meta, type, logScale, isCurVolt]);

  const labels = { res:'Resistance', points:'Focal Points', cur:'Current', src:'Source', gnd:'Ground', volt:'Voltage' };
  const dim = meta ? `${meta.ncols}×${meta.nrows}` : '';
  const fmt = (v) => Math.abs(v) < 0.01 ? v.toExponential(1) : Math.abs(v) >= 1000 ? v.toFixed(0) : v.toPrecision(3);

  const legendStyle = { flex: 1, height: 14, borderRadius: 3, border: '1px solid #444' };

  return (
    <div style={{ border: '1px solid #333', display: 'flex', flexDirection: 'column', maxWidth: '100%' }}>
      <div style={{ fontSize: '.65em', padding: '3px 6px', background: '#1a1a1a', borderBottom: '1px solid #333', color: '#888', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <span>{labels[type] || type} {dim && <span style={{ color: '#555' }}>{dim}</span>}</span>
        {onToggleScale && (
          <div style={{ display: 'flex', gap: '2px' }}>
            {['log', 'lin'].map(s => (
              <span key={s} onClick={() => onToggleScale(s)}
                style={{ padding: '1px 4px', border: '1px solid #333', cursor: 'pointer', background: logScale === (s === 'log') ? '#238636' : 'transparent', color: logScale === (s === 'log') ? '#fff' : '#888', fontSize: '.85em' }}>
                {s}
              </span>
            ))}
          </div>
        )}
      </div>
      <canvas ref={canvasRef} style={{ display: 'block', imageRendering: 'pixelated', width: '100%', height: 'auto' }} />
      {type !== 'points' && (
        <div style={{ display: 'flex', gap: 6, alignItems: 'center', fontSize: '.65em', padding: '3px 6px', color: '#999', background: '#121212' }}>
          <span style={{ minWidth: '4em', textAlign: 'right' }}>{fmt(range.min)}</span>
          <div style={{ ...legendStyle, background: isCurVolt ? PLASMA_GRADIENT : VIRIDIS_GRADIENT }} />
          <span style={{ minWidth: '4em' }}>{fmt(range.max)}</span>
        </div>
      )}
    </div>
  );
}
