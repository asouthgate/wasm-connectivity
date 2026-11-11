export type RasterInput = Float32Array | Float64Array | number[];
export type Precision = "f32" | "f64" | "mixed";

export function load(): Promise<void>;
export { downsample_raster as downsampleRaster, upsample_raster as upsampleRaster } from "./lmConnect.js";

export interface CurrentSolverOptions {
  maxIter?: number;
  tol?: number;
  useDirichletGround?: boolean;
  precision?: Precision;
}

export interface CurrentSolveResult {
  output: {
    voltages: number[];
    current_map: number[];
    nrows: number;
    ncols: number;
  };
  total_iters: number;
  error?: string;
}

export function solveCurrentAsync(
  resistance: RasterInput, nrows: number, ncols: number, nodata: number,
  source: RasterInput, ground: RasterInput, options?: CurrentSolverOptions,
): Promise<CurrentSolveResult>;

export interface GeospatialOptions {
  geojson?: Record<string, unknown>;
  layerParams?: Record<string, { resistance: number; width?: number }>;
  maxIter?: number;
  tol?: number;
  useDirichletGround?: boolean;
  precision?: Precision;
}

export function runGeospatialPipelineMgAsync(
  baseRaster: RasterInput, nrows: number, ncols: number, nodata: number,
  xmin: number, ymax: number, cellsize: number,
  srcData: RasterInput, gndData: RasterInput, options?: GeospatialOptions,
): Promise<string>;

export function runGeospatialPipelineMgAsync(
  baseRaster: RasterInput, nrows: number, ncols: number, nodata: number,
  geojsonStr: string, layerParamsStr: string, xmin: number, ymax: number,
  cellsize: number, srcData: RasterInput, gndData: RasterInput,
  maxIter?: number, tol?: number, useDirichletGround?: boolean, precision?: Precision,
): Promise<string>;

export function runGeospatialPipelineMgStencilAsync(
  baseRaster: RasterInput, nrows: number, ncols: number, nodata: number,
  xmin: number, ymax: number, cellsize: number,
  srcData: RasterInput, gndData: RasterInput, options?: GeospatialOptions,
): Promise<string>;

export function runGeospatialPipelineMgStencilAsync(
  baseRaster: RasterInput, nrows: number, ncols: number, nodata: number,
  geojsonStr: string, layerParamsStr: string, xmin: number, ymax: number,
  cellsize: number, srcData: RasterInput, gndData: RasterInput,
  maxIter?: number, tol?: number, useDirichletGround?: boolean, precision?: Precision,
): Promise<string>;

export interface RasterFeatureCollection {
  type: "FeatureCollection";
  features: Array<{
    type: "Feature";
    properties: { value: number; [key: string]: unknown };
    geometry: {
      type: "Polygon" | "MultiPolygon" | "LineString" | "MultiLineString";
      coordinates: unknown;
    };
  }>;
}

export interface RasterGrid {
  nrows: number;
  ncols: number;
  xmin: number;
  ymax: number;
  cellsize: number;
  /** Line buffer radius in grid coordinate units; defaults to zero. */
  buffer?: number;
}

export function rasterizeGeojsonAsync(
  features: RasterFeatureCollection, grid: RasterGrid,
): Promise<Float64Array>;

export interface ResistanceOutput {
  nrows: number;
  ncols: number;
  total_res: Float32Array;
  lamp_res: Float32Array;
  road_res: Float32Array;
  river_res: Float32Array;
  landscape_res: Float32Array;
  linear_res: Float32Array;
  generic_res: Float32Array;
  soft_surf: Float32Array;
  hard_surf: Float32Array;
}

export function runResistancePipelineBrowserAsync(
  roadBinary: RasterInput, riverBinary: RasterInput, buildingHeightsMask: RasterInput,
  dtm: RasterInput, dsm: RasterInput, genericResistance: RasterInput,
  lamps: RasterInput, landscapeConductance: RasterInput, paramsJson: string,
): Promise<ResistanceOutput>;

export function roostFinderComputeAsync(
  detectorsCsv: string, masterCsv: string, sunsetCsv: string,
  gridSize: number, diffusivity: number, t0: number, t1: number,
): Promise<string>;

export function parseAsc(text: string): {
  meta: {
    ncols: number;
    nrows: number;
    xllcorner: number;
    yllcorner: number;
    cellsize: number;
    nodata: number;
  };
  data: Float64Array;
};
