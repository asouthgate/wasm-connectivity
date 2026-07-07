import { Spinner } from './Spinner';

export function StatusBar({ status, loading, showSpinner = true, error = false }) {
  return (
    <div className="status" style={{ display: 'flex', alignItems: 'center', color: error ? '#f44' : undefined }}>
      {loading && showSpinner && <Spinner />}{status}
    </div>
  );
}
