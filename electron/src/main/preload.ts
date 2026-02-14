import { contextBridge, ipcRenderer } from 'electron';

/**
 * Expose a safe API to the renderer process via contextBridge.
 * The renderer can access these methods via `window.astro`.
 */
contextBridge.exposeInMainWorld('astro', {
  // ─── File dialogs ───────────────────────────────────────────────────────
  openFiles: (options?: {
    title?: string;
    filters?: { name: string; extensions: string[] }[];
  }) => ipcRenderer.invoke('dialog:open-files', options || {}),

  saveFile: (options?: {
    title?: string;
    defaultPath?: string;
    filters?: { name: string; extensions: string[] }[];
  }) => ipcRenderer.invoke('dialog:save-file', options || {}),

  // ─── Native bridge ─────────────────────────────────────────────────────
  /** Load only FITS headers (no pixel data) for the file list. */
  loadFits: (filePath: string) =>
    ipcRenderer.invoke('native:load-fits', filePath),

  /** Load a single file fully for preview purposes. */
  loadPreview: (filePath: string) =>
    ipcRenderer.invoke('native:load-preview', filePath),

  /** Release a stored image to free memory. */
  releaseImage: (imageId: string) =>
    ipcRenderer.invoke('native:release-image', imageId),

  /** Release all stored images. */
  releaseAllImages: () =>
    ipcRenderer.invoke('native:release-all-images'),

  getImageInfo: (imageId: string) =>
    ipcRenderer.invoke('native:get-image-info', imageId),

  getPreview: (imageId: string, stretch?: {
    shadows: number;
    midtones: number;
    highlights: number;
  }) => ipcRenderer.invoke('native:get-preview', imageId, stretch),

  getHistogram: (imageId: string, channel: number, bins: number) =>
    ipcRenderer.invoke('native:get-histogram', imageId, channel, bins),

  getAutoStretch: (imageId: string) =>
    ipcRenderer.invoke('native:get-auto-stretch', imageId),

  saveImage: (
    imageId: string,
    filePath: string,
    format: string,
    stretch?: { shadows: number; midtones: number; highlights: number },
  ) => ipcRenderer.invoke('native:save-image', imageId, filePath, format, stretch),

  runPipeline: (config: {
    lightPaths: string[];
    darkPaths: string[];
    flatPaths: string[];
    biasPaths: string[];
    bayerPattern?: string;
    stackingConfig: { method: string; kappa?: number; iterations?: number };
  }) => ipcRenderer.invoke('native:run-pipeline', config),
});
