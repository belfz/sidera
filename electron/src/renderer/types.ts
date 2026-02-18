/** Type definitions for astro-viber */

export type FrameType = 'Light' | 'Dark' | 'Flat' | 'Bias' | 'Unknown';

export interface FileInfo {
  id: string;
  path: string;
  filename: string;
  width: number;
  height: number;
  channels: number;
  bitpix: number;
  frameType: FrameType;
  exposureTime?: number;
  temperature?: number;
  gain?: number;
  filterName?: string;
  bayerPattern?: string;
}

export interface StretchConfig {
  shadows: number;
  midtones: number;
  highlights: number;
}

export interface HistogramData {
  bins: number[];
  min: number;
  max: number;
  channel: number;
}

export interface StackingConfig {
  method: 'mean' | 'median' | 'sigma_clip_mean' | 'sigma_clip_median';
  kappa: number;
  iterations: number;
}

export interface DetectedStar {
  x: number;
  y: number;
  flux: number;
  hfr: number;
  peak: number;
}

export interface PipelineProgress {
  stage: string;
  progress: number; // 0-1
}

/** The window.astro API exposed by the preload script. */
export interface AstroAPI {
  openFiles: (options?: {
    title?: string;
    filters?: { name: string; extensions: string[] }[];
  }) => Promise<{ canceled: boolean; filePaths: string[] }>;

  saveFile: (options?: {
    title?: string;
    defaultPath?: string;
    filters?: { name: string; extensions: string[] }[];
  }) => Promise<{ canceled: boolean; filePath: string }>;

  /** Load FITS headers only — no pixel data stored. */
  loadFits: (filePath: string) => Promise<FileInfo>;

  /** Load a single file fully for preview. Returns store image ID. */
  loadPreview: (filePath: string) => Promise<string>;

  /** Release a stored image to free memory. */
  releaseImage: (imageId: string) => Promise<void>;

  /** Release all stored images. */
  releaseAllImages: () => Promise<void>;

  getImageInfo: (imageId: string) => Promise<{ width: number; height: number; channels: number }>;
  getPreview: (imageId: string, stretch?: StretchConfig) => Promise<Buffer | null>;
  getHistogram: (imageId: string, channel: number, bins: number) => Promise<HistogramData>;
  getAutoStretch: (imageId: string) => Promise<StretchConfig>;

  saveImage: (imageId: string, filePath: string, format: string, stretch?: StretchConfig) => Promise<void>;

  runPipeline: (config: {
    lightPaths: string[];
    darkPaths: string[];
    flatPaths: string[];
    biasPaths: string[];
    bayerPattern?: string;
    stackingConfig: StackingConfig;
  }) => Promise<string>;

  /** Save the full (untruncated) log history to a file. Returns true if saved. */
  saveLogs: () => Promise<boolean>;

  /** Subscribe to log lines from the processing engine. Returns unsubscribe fn. */
  onLog: (callback: (line: string) => void) => () => void;

  /** Subscribe to pipeline progress updates. Returns unsubscribe fn. */
  onProgress: (callback: (data: { id: string; stage: string; percent: number }) => void) => () => void;
}

declare global {
  interface Window {
    astro: AstroAPI;
  }
}
