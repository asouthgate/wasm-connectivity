import { BrowserRouter, Routes, Route, Link, useLocation } from 'react-router-dom';
import Example from './pages/Example';
import Benchmark from './pages/Benchmark';

function Nav() {
  const loc = useLocation();
  const active = (path) => loc.pathname === path;
  const link = (path) => ({
    color: active(path) ? '#ccc' : '#888',
    textDecoration: active(path) ? 'underline' : 'none',
    fontSize: '.85em', padding: '2px 8px',
  });
  return (
    <div style={{ display: 'flex', gap: 16, marginBottom: 12 }}>
      <Link to="/example" style={link('/example')}>example</Link>
      <Link to="/benchmark" style={link('/benchmark')}>benchmark</Link>
    </div>
  );
}

export default function App() {
  return (
    <BrowserRouter>
      <Nav />
      <Routes>
        <Route path="/" element={<Example />} />
        <Route path="/example" element={<Example />} />
        <Route path="/benchmark" element={<Benchmark />} />
      </Routes>
    </BrowserRouter>
  );
}
