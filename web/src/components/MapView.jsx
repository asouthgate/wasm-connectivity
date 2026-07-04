import { useRef, useEffect } from 'react';
import { renderMap, renderPoints } from '../lib/render';

export default function MapView({ type, data, meta, logScale, onToggleScale }) {
  const canvasRef = useRef(null);

  useEffect(() => {
    if (!data || !meta) return;
    const canvas = canvasRef.current;
    if (type === 'points') {
      renderPoints(canvas, data, meta.nrows, meta.ncols, meta.nodata);
    } else {
      renderMap(canvas, data, meta.nrows, meta.ncols, meta.nodata || 0, logScale);
    }
  }, [data, meta, type, logScale]);

  const dim = meta ? `${meta.ncols}×${meta.nrows}` : '';

  return (
    <div style={{ border: '1px solid #333' }}>
      <div style={{ fontSize: '.65em', padding: '3px 6px', background: '#1a1a1a', borderBottom: '1px solid #333', color: '#888', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <span>{type === 'points' ? 'Focal Points' : type === 'res' ? 'Resistance' : 'Current'} {dim && <span style={{ color: '#555' }}>{dim}</span>}</span>
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
      <canvas ref={canvasRef} style={{ display: 'block', imageRendering: 'pixelated', width: '100%' }} />
      {type !== 'points' && (
        <div style={{ display: 'flex', gap: '4px', alignItems: 'center', fontSize: '.6em', padding: '2px 6px', color: '#666' }}>
          <span>lo</span>
          <div style={{ width: '60px', height: '8px', borderRadius: '2px', background: type === 'res' ? 'linear-gradient(to right,#00f,#0ff,#0f0,#ff0,#f00)' : 'linear-gradient(to right,#001,#00f,#0ff,#ff0,#f00)' }} />
          <span>hi</span>
        </div>
      )}
    </div>
  );
}
