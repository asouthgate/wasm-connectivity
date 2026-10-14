export function StatusBar({ status, loading, error = false }) {
  return <p className={error ? 'status error' : 'status'} aria-busy={loading}>
    {loading && <span className="spinner" role="status" aria-label="Working" />} {status}
  </p>;
}
