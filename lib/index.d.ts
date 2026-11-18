export type RasterInput = Float32Array | Float64Array | number[];
export type Precision = "f32" | "f64" | "mixed";

export function load(): Promise<void>;
export { downsample_raster as downsampleRaster, upsample_raster as upsampleRaster } from "./lmConnect.js";

export interface CurrentSolverOptions {
  maxIter?: number;
  tol?: number;
  useDirichletGround?: boolean;
  precision?: Precision;
  lowMemory?: boolean;
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

export interface VectorFeatureCollection {
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
  features: VectorFeatureCollection, grid: RasterGrid,
): Promise<Float64Array>;

type ResistanceRaster = Float32Array | Float64Array;

export interface ResistanceOutput<Raster extends ResistanceRaster = Float32Array> {
  nrows: number;
  ncols: number;
  output: Raster;
  current?: Raster;
  layers: Record<string, Record<string, Raster>>;
}

/** Inputs use model names: prepared rasters, feature masks for vector_mask, or pixel x/y/height triples. */
export function runResistancePipelineBrowserAsync(
  modelJson: string, inputs: Record<string, RasterInput>,
  grid: { nrows: number; ncols: number; cellsize: number },
  options: ResistanceExecutionOptions & { samples?: 1; outputPrecision: "f64" },
): Promise<ResistanceOutput<Float64Array>>;

export function runResistancePipelineBrowserAsync(
  modelJson: string, inputs: Record<string, RasterInput>,
  grid: { nrows: number; ncols: number; cellsize: number },
  options?: ResistanceExecutionOptions & { samples?: 1; outputPrecision?: "f32" },
): Promise<ResistanceOutput>;

export interface ResultSummaryStatistics {
  mean: Float32Array;
  /** Sample variance, divided by samples - 1. */
  variance: Float32Array;
}

export interface ResistanceExecutionOptions {
  /** Defaults to 1; values above 1 return means and variances. */
  samples?: number;
  seed?: number;
  allLayers?: boolean;
  outputPrecision?: "f32" | "f64";
  currentSolverOptions?: {
    source: RasterInput;
    ground: RasterInput;
    maxIter?: number;
    tol?: number;
    useDirichletGround?: boolean;
  };
}

export interface ResistanceSamplingResult<Raster extends ResistanceRaster = Float32Array> {
  nrows: number;
  ncols: number;
  samples: number;
  resistance: ResultSummaryStatistics<Raster>;
  current?: ResultSummaryStatistics<Raster>;
  layers: Record<string, Record<string, ResultSummaryStatistics<Raster>>>;
}

export function runResistancePipelineBrowserAsync(
  modelJson: string, inputs: Record<string, RasterInput>,
  grid: { nrows: number; ncols: number; cellsize: number },
  options: ResistanceExecutionOptions & { outputPrecision?: "f32" },
): Promise<ResistanceOutput | ResistanceSamplingResult>;

export function runResistancePipelineBrowserAsync(
  modelJson: string, inputs: Record<string, RasterInput>,
  grid: { nrows: number; ncols: number; cellsize: number },
  options: ResistanceExecutionOptions,
): Promise<ResistanceOutput<ResistanceRaster> | ResistanceSamplingResult<ResistanceRaster>>;

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
