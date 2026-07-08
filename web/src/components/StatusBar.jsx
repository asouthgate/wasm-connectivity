import { Spinner } from './Spinner';

export function StatusBar({ status, loading, showSpinner = true, error = false }) {
  return (
    <div className={`status status-bar${error ? ' status-bar--error' : ''}`}>
      {loading && showSpinner && <Spinner />}{status}
    </div>
  );
}
