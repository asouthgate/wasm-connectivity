/* tslint:disable */
/* eslint-disable */

export function downsample_raster(data: Float64Array, nrows: number, ncols: number, nodata: number, target_rows: number, target_cols: number): string;

export function init_panic_hook(): void;

export function rasterize_geojson(base_raster: Float64Array, nrows: number, ncols: number, _nodata: number, geojson_str: string, layer_params_str: string, xmin: number, ymax: number, cellsize: number): string;

export function reset_cache(): void;

export function run_geospatial_pipeline_cached_mg(base_raster: Float64Array, nrows: number, ncols: number, nodata: number, geojson_str: string, layer_params_str: string, xmin: number, ymax: number, cellsize: number, source_data: Float64Array, ground_data: Float64Array, max_iter: number, tol: number, use_dirichlet_ground: boolean): string;

export function run_resistance_pipeline_browser(road_binary: Float64Array, river_binary: Float64Array, building_mask: Float64Array, dtm: Float64Array, dsm: Float64Array, generic_resistance: Float64Array, lamps: Float64Array, landscape_conductance: Float64Array, params_json: string): string;

export function run_resistance_pipeline_wasm(road_binary: Float64Array, river_binary: Float64Array, building_mask: Float64Array, lcm: Float64Array, dtm: Float64Array, dsm: Float64Array, generic_resistance: Float64Array, lamps: Float64Array, params_json: string): string;

export function solve_raster_sources_jacobi_cached(resistance_data: Float64Array, nrows: number, ncols: number, nodata: number, source_data: Float64Array, ground_data: Float64Array, max_iter: number, tol: number, rebuild_laplacian: boolean, use_dirichlet_ground: boolean): string;

export function solve_raster_sources_mg(resistance_data: Float64Array, nrows: number, ncols: number, nodata: number, source_data: Float64Array, ground_data: Float64Array, max_iter: number, tol: number, use_dirichlet_ground: boolean): string;

export function solve_raster_sources_mg_alcouffe(resistance_data: Float64Array, nrows: number, ncols: number, nodata: number, source_data: Float64Array, ground_data: Float64Array, max_iter: number, tol: number, use_dirichlet_ground: boolean): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly downsample_raster: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => [number, number];
    readonly rasterize_geojson: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number) => [number, number];
    readonly run_geospatial_pipeline_cached_mg: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number, s: number) => [number, number];
    readonly run_resistance_pipeline_browser: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number) => [number, number];
    readonly run_resistance_pipeline_wasm: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number) => [number, number];
    readonly solve_raster_sources_jacobi_cached: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number) => [number, number];
    readonly solve_raster_sources_mg: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number) => [number, number];
    readonly solve_raster_sources_mg_alcouffe: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number) => [number, number];
    readonly init_panic_hook: () => void;
    readonly reset_cache: () => void;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
