import { create } from 'zustand';
import type { FileInfo, FrameType, StackingConfig, StretchConfig, PipelineProgress } from './types';

const MAX_LOG_LINES = 500;

interface AppState {
  // ─── Frame lists ──────────────────────────────────────────────────────
  lights: FileInfo[];
  darks: FileInfo[];
  flats: FileInfo[];
  biases: FileInfo[];

  // ─── Master calibration frame IDs ─────────────────────────────────────
  masterBiasId: string | null;
  masterDarkId: string | null;
  masterFlatId: string | null;

  // ─── Currently selected / displayed image ─────────────────────────────
  selectedImageId: string | null;
  resultImageId: string | null;
  /** Store image ID for the currently loaded preview (set by ImagePreview). */
  previewStoreId: string | null;

  // ─── Stretch parameters ───────────────────────────────────────────────
  stretch: StretchConfig;
  autoStretch: boolean;

  // ─── Stacking configuration ───────────────────────────────────────────
  stackingConfig: StackingConfig;
  bayerPattern: string | null;

  // ─── Pipeline state ───────────────────────────────────────────────────
  isProcessing: boolean;
  progress: PipelineProgress | null;
  error: string | null;

  // ─── Log output ───────────────────────────────────────────────────────
  logLines: string[];

  // ─── Actions ──────────────────────────────────────────────────────────
  addFrames: (frameType: FrameType, files: FileInfo[]) => void;
  removeFrame: (frameType: FrameType, id: string) => void;
  clearFrames: (frameType: FrameType) => void;
  clearAllFrames: () => void;

  setMasterBiasId: (id: string | null) => void;
  setMasterDarkId: (id: string | null) => void;
  setMasterFlatId: (id: string | null) => void;

  setSelectedImageId: (id: string | null) => void;
  setResultImageId: (id: string | null) => void;
  setPreviewStoreId: (id: string | null) => void;

  setStretch: (stretch: StretchConfig) => void;
  setAutoStretch: (auto: boolean) => void;

  setStackingConfig: (config: Partial<StackingConfig>) => void;
  setBayerPattern: (pattern: string | null) => void;

  setProcessing: (processing: boolean) => void;
  setProgress: (progress: PipelineProgress | null) => void;
  setError: (error: string | null) => void;

  addLogLine: (line: string) => void;
  clearLog: () => void;
}

const frameListKey = (type: FrameType): keyof Pick<AppState, 'lights' | 'darks' | 'flats' | 'biases'> => {
  switch (type) {
    case 'Light': return 'lights';
    case 'Dark': return 'darks';
    case 'Flat': return 'flats';
    case 'Bias': return 'biases';
    default: return 'lights';
  }
};

export const useAppStore = create<AppState>((set) => ({
  lights: [],
  darks: [],
  flats: [],
  biases: [],

  masterBiasId: null,
  masterDarkId: null,
  masterFlatId: null,

  selectedImageId: null,
  resultImageId: null,
  previewStoreId: null,

  stretch: { shadows: 0, midtones: 0.25, highlights: 1.0 },
  autoStretch: true,

  stackingConfig: {
    method: 'sigma_clip_mean',
    kappa: 3.0,
    iterations: 5,
  },
  bayerPattern: null,

  isProcessing: false,
  progress: null,
  error: null,

  logLines: [],

  addFrames: (frameType, files) =>
    set((state) => {
      const key = frameListKey(frameType);
      return { [key]: [...state[key], ...files] };
    }),

  removeFrame: (frameType, id) =>
    set((state) => {
      const key = frameListKey(frameType);
      return { [key]: state[key].filter((f) => f.id !== id) };
    }),

  clearFrames: (frameType) =>
    set(() => ({ [frameListKey(frameType)]: [] })),

  clearAllFrames: () =>
    set(() => ({
      lights: [],
      darks: [],
      flats: [],
      biases: [],
      masterBiasId: null,
      masterDarkId: null,
      masterFlatId: null,
      selectedImageId: null,
      resultImageId: null,
      previewStoreId: null,
    })),

  setMasterBiasId: (id) => set({ masterBiasId: id }),
  setMasterDarkId: (id) => set({ masterDarkId: id }),
  setMasterFlatId: (id) => set({ masterFlatId: id }),

  setSelectedImageId: (id) => set({ selectedImageId: id }),
  setResultImageId: (id) => set({ resultImageId: id }),
  setPreviewStoreId: (id) => set({ previewStoreId: id }),

  setStretch: (stretch) => set({ stretch }),
  setAutoStretch: (autoStretch) => set({ autoStretch }),

  setStackingConfig: (config) =>
    set((state) => ({
      stackingConfig: { ...state.stackingConfig, ...config },
    })),

  setBayerPattern: (pattern) => set({ bayerPattern: pattern }),

  setProcessing: (isProcessing) => set({ isProcessing }),
  setProgress: (progress) => set({ progress }),
  setError: (error) => set({ error }),

  addLogLine: (line) =>
    set((state) => {
      const newLines = [...state.logLines, line];
      // Keep only the last MAX_LOG_LINES lines
      if (newLines.length > MAX_LOG_LINES) {
        return { logLines: newLines.slice(-MAX_LOG_LINES) };
      }
      return { logLines: newLines };
    }),

  clearLog: () => set({ logLines: [] }),
}));
