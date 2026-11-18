import { parseAsc } from 'lmConnect';

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

export function exampleDefaults(model) {
    return {
        terrainRes: model.nodes.terrain.parameters.scale,
        roadRes: model.nodes.road_resistance.parameters.scale,
        riverRes: model.nodes.river_resistance.parameters.scale,
        buildRes: model.nodes.building_resistance.parameters.scale,
        roadWidth: 3,
        riverWidth: 4,
    };
}

export function exampleModel(template, settings) {
    const model = structuredClone(template);
    model.nodes.terrain.parameters.scale = settings.terrainRes;
    model.nodes.road_resistance.parameters.scale = settings.roadRes;
    model.nodes.river_resistance.parameters.scale = settings.riverRes;
    model.nodes.building_resistance.parameters.scale = settings.buildRes;
    return JSON.stringify(model);
}

export function exampleFeatureMasks(geojson, grid, settings, rasterize) {
    const collection = JSON.parse(geojson);
    const widths = { roads: settings.roadWidth, rivers: settings.riverWidth, buildings: 0 };
    return Object.entries(widths).map(([name, buffer]) => {
        const features = collection.features
            .filter(feature => feature.properties?.layer === name)
            .map(({ geometry }) => ({ type: 'Feature', geometry, properties: { value: 1 } }));
        return [name, rasterize({ type: 'FeatureCollection', features }, { ...grid, buffer })];
    });
}
