import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import { load } from './lib/wasm';

const root = ReactDOM.createRoot(document.getElementById('root'));

load()
  .then(() => {
    root.render(
      <React.StrictMode>
        <App />
      </React.StrictMode>
    );
  })
  .catch(err => {
    root.render(
      <div style={{ color: '#f44', padding: 20, font: '13px monospace' }}>
        <p>Failed to load WASM module.</p>
        <pre>{err.message}</pre>
      </div>
    );
  });
