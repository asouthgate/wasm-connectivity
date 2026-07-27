import { parseAsc } from '@wasm-connect/lib';

const GEO_BASE = '/geodata'

export async function loadExampleData(res) {
    const [baseText, srcText, gndText, geoResp] = await Promise.all([
        fetch(`${GEO_BASE}/base_resistance_${res}.asc`).then(r => r.text()),
        fetch(`${GEO_BASE}/source_${res}.asc`).then(r => r.text()),
        fetch(`${GEO_BASE}/ground_${res}.asc`).then(r => r.text()),
        fetch(`${GEO_BASE}/all_features_${res}.geojson`).then(r => r.text()),
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