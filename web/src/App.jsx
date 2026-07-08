import { BrowserRouter, Routes, Route, Link, useLocation } from 'react-router-dom';
import Example from './pages/Example';
import Benchmark from './pages/Benchmark';

function Nav() {
  const loc = useLocation();
  const linkClass = (path) => `nav-link${loc.pathname === path ? ' nav-link--active' : ''}`;
  return (
    <div className="nav">
      <Link to="/example" className={linkClass('/example')}>example</Link>
      <Link to="/benchmark" className={linkClass('/benchmark')}>benchmark</Link>
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
