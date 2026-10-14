import { BrowserRouter, Routes, Route, Link, useLocation } from 'react-router-dom';
import Example from './pages/Example';
import Benchmark from './pages/Benchmark';
import ScenarioBenchmark from './pages/ScenarioBenchmark';

function Nav() {
  const loc = useLocation();
  const linkClass = (path) => `nav-link${loc.pathname === path ? ' nav-link--active' : ''}`;
  return (
    <div className="nav">
      <Link to="/" className={linkClass('/')}>example pipeline</Link>
      <Link to="/benchmark" className={linkClass('/benchmark')}>solver-benchmark</Link>
      <Link to="/scenario-benchmark" className={linkClass('/scenario-benchmark')}>scenario-benchmark</Link>
    </div>
  );
}

export default function App() {
  return (
    <BrowserRouter>
      <Nav />
      <Routes>
        <Route path="/" element={<Example />} />
        <Route path="/benchmark" element={<Benchmark />} />
        <Route path="/scenario-benchmark" element={<ScenarioBenchmark />} />
      </Routes>
    </BrowserRouter>
  );
}
