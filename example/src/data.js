import { parseAsc } from '@wasm-connect/lib';

const GEO_BASE = '/geodata';

async function fetchText(path) {
    const response = await fetch(`${GEO_BASE}/${path}`);
    if (!response.ok) throw new Error(`Could not load ${path}: HTTP ${response.status}`);
    return response.text();
}

export async function loadExampleData(res) {
    const [baseText, srcText, gndText, geoResp] = await Promise.all([
        fetchText(`base_resistance_${res}.asc`),
        fetchText(`source_${res}.asc`),
        fetchText(`ground_${res}.asc`),
        fetchText(`all_features_${res}.geojson`),
    ]);
    const bp = parseAsc(baseText);
    return {
        baseData: bp.data,
        baseMeta: bp.meta,
        srcData: parseAsc(srcText).data,
        gndData: parseAsc(gndText).data,
        geojsonStr: geoResp
    };
}