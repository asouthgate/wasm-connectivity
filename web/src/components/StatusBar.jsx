import { Spinner } from './Spinner';

export function StatusBar({ status, loading, showSpinner = true }) {
  return (
    <div className="status" style={{ color: status.color, display: 'flex', alignItems: 'center', position: 'relative', zIndex: 91 }}>
      {loading && showSpinner && <Spinner />}{status.text}
    </div>
  );
}
