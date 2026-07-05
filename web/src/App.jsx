import { BrowserRouter, Routes, Route, Link, useLocation } from 'react-router-dom';
import Solver from './pages/Solver';
import Raster from './pages/Raster';
import Experiment from './pages/Experiment';

function Nav() {
  const loc = useLocation();
  const linkStyle = (path) => ({
    color: loc.pathname === path ? '#58a6ff' : '#888',
    textDecoration: 'none', fontSize: '.85em', padding: '2px 8px',
    borderBottom: loc.pathname === path ? '1px solid #58a6ff' : '1px solid transparent',
  });
  return (
    <div style={{ display: 'flex', gap: 16, marginBottom: 12 }}>
      <Link to="/" style={linkStyle('/')}>pairwise</Link>
      <Link to="/raster" style={linkStyle('/raster')}>raster</Link>
      <Link to="/experiment" style={linkStyle('/experiment')}>experiment</Link>
    </div>
  );
}

export default function App() {
  return (
    <BrowserRouter>
      <Nav />
      <Routes>
        <Route path="/" element={<Solver />} />
        <Route path="/raster" element={<Raster />} />
        <Route path="/experiment" element={<Experiment />} />
      </Routes>
    </BrowserRouter>
  );
}
