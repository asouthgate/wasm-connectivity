import { BrowserRouter, Routes, Route, NavLink } from 'react-router-dom';
import Example from './pages/Example';
import Benchmark from './pages/Benchmark';
import ScenarioBenchmark from './pages/ScenarioBenchmark';

export default function App() {
  return (
    <BrowserRouter>
      <nav className="row">
        <NavLink to="/" end>example pipeline</NavLink>
        <NavLink to="/benchmark">solver-benchmark</NavLink>
        <NavLink to="/scenario-benchmark">scenario-benchmark</NavLink>
      </nav>
      <Routes>
        <Route path="/" element={<Example />} />
        <Route path="/benchmark" element={<Benchmark />} />
        <Route path="/scenario-benchmark" element={<ScenarioBenchmark />} />
      </Routes>
    </BrowserRouter>
  );
}
