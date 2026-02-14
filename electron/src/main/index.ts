import { app, BrowserWindow, ipcMain, dialog } from 'electron';
import * as path from 'path';

// ─── Load native Rust addon ────────────────────────────────────────────────
let napi: any = null;
try {
  // From electron/dist/main/ -> go up 3 levels to project root -> crates/napi-bridge/
  const napiPath = path.join(__dirname, '../../../crates/napi-bridge/index.js');
  console.log('[astro-viber] Loading native addon from:', napiPath);
  napi = require(napiPath);
  napi.initLogger();
  console.log('[astro-viber] Native addon loaded successfully');
} catch (err) {
  console.error('[astro-viber] Failed to load native addon:', err);
  console.warn('[astro-viber] Running in mock mode — processing will not work');
}

const isDev = process.env.NODE_ENV === 'development' || !app.isPackaged;

let mainWindow: BrowserWindow | null = null;

function createWindow(): void {
  mainWindow = new BrowserWindow({
    width: 1400,
    height: 900,
    minWidth: 1000,
    minHeight: 700,
    backgroundColor: '#0a0e17',
    titleBarStyle: 'hiddenInset',
    trafficLightPosition: { x: 16, y: 16 },
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
  });

  if (isDev) {
    mainWindow.loadURL('http://localhost:5173');
    mainWindow.webContents.openDevTools({ mode: 'right' });
  } else {
    mainWindow.loadFile(path.join(__dirname, '../renderer/index.html'));
  }

  mainWindow.on('closed', () => {
    mainWindow = null;
  });
}

// ─── IPC Handlers ───────────────────────────────────────────────────────────

// File dialog for importing FITS files
ipcMain.handle('dialog:open-files', async (_event, options: {
  title?: string;
  filters?: { name: string; extensions: string[] }[];
}) => {
  if (!mainWindow) return { canceled: true, filePaths: [] };

  const result = await dialog.showOpenDialog(mainWindow, {
    title: options.title || 'Select FITS files',
    filters: options.filters || [
      { name: 'FITS Files', extensions: ['fits', 'fit', 'fts'] },
      { name: 'All Files', extensions: ['*'] },
    ],
    properties: ['openFile', 'multiSelections'],
  });

  return result;
});

// Save dialog
ipcMain.handle('dialog:save-file', async (_event, options: {
  title?: string;
  defaultPath?: string;
  filters?: { name: string; extensions: string[] }[];
}) => {
  if (!mainWindow) return { canceled: true, filePath: '' };

  const result = await dialog.showSaveDialog(mainWindow, {
    title: options.title || 'Save image',
    defaultPath: options.defaultPath,
    filters: options.filters || [
      { name: 'FITS', extensions: ['fits'] },
      { name: 'TIFF', extensions: ['tiff', 'tif'] },
      { name: 'PNG', extensions: ['png'] },
    ],
  });

  return result;
});

// ─── Native bridge IPC handlers ─────────────────────────────────────────────

// Load FITS headers only (no pixel data) — for file list
ipcMain.handle('native:load-fits', async (_event, filePath: string) => {
  if (!napi) throw new Error('Native addon not loaded');
  try {
    return await napi.loadFitsFile(filePath);
  } catch (err: any) {
    console.error(`[native] Failed to load FITS headers: ${filePath}`, err?.message || err);
    throw err;
  }
});

// Load a full file for preview
ipcMain.handle('native:load-preview', async (_event, filePath: string) => {
  if (!napi) throw new Error('Native addon not loaded');
  try {
    return await napi.loadPreview(filePath);
  } catch (err: any) {
    console.error(`[native] Failed to load preview: ${filePath}`, err?.message || err);
    throw err;
  }
});

// Release a stored image to free memory
ipcMain.handle('native:release-image', async (_event, imageId: string) => {
  if (!napi) return;
  try {
    napi.releaseImage(imageId);
  } catch (err: any) {
    console.warn('[native] Failed to release image:', err?.message || err);
  }
});

// Release all stored images
ipcMain.handle('native:release-all-images', async () => {
  if (!napi) return;
  try {
    napi.releaseAllImages();
  } catch (err: any) {
    console.warn('[native] Failed to release images:', err?.message || err);
  }
});

ipcMain.handle('native:get-image-info', async (_event, imageId: string) => {
  if (!napi) throw new Error('Native addon not loaded');
  try {
    return napi.getImageInfo(imageId);
  } catch (err: any) {
    console.error('[native] Failed to get image info:', err?.message || err);
    throw err;
  }
});

ipcMain.handle('native:get-preview', async (_event, imageId: string, stretch?: {
  shadows: number;
  midtones: number;
  highlights: number;
}) => {
  if (!napi) return null;
  try {
    return napi.getPreview(imageId, stretch || undefined);
  } catch (err: any) {
    console.error('[native] Failed to get preview:', err?.message || err);
    return null;
  }
});

ipcMain.handle('native:get-histogram', async (_event, imageId: string, channel: number, bins: number) => {
  if (!napi) return { bins: new Array(bins).fill(0), min: 0, max: 65535, channel };
  try {
    return napi.getHistogram(imageId, channel, bins);
  } catch (err: any) {
    console.error('[native] Failed to get histogram:', err?.message || err);
    return { bins: new Array(bins).fill(0), min: 0, max: 65535, channel };
  }
});

ipcMain.handle('native:get-auto-stretch', async (_event, imageId: string) => {
  if (!napi) return { shadows: 0.0, midtones: 0.25, highlights: 1.0 };
  try {
    return napi.getAutoStretch(imageId);
  } catch (err: any) {
    console.error('[native] Failed to get auto-stretch:', err?.message || err);
    return { shadows: 0.0, midtones: 0.25, highlights: 1.0 };
  }
});

ipcMain.handle('native:save-image', async (_event, imageId: string, filePath: string, format: string, stretch?: {
  shadows: number;
  midtones: number;
  highlights: number;
}) => {
  if (!napi) throw new Error('Native addon not loaded');
  return await napi.saveImage(imageId, filePath, format, stretch || undefined);
});

ipcMain.handle('native:run-pipeline', async (_event, config: {
  lightPaths: string[];
  darkPaths: string[];
  flatPaths: string[];
  biasPaths: string[];
  bayerPattern?: string;
  stackingConfig: { method: string; kappa?: number; iterations?: number };
}) => {
  if (!napi) throw new Error('Native addon not loaded');
  console.log(`[pipeline] Starting: ${config.lightPaths.length} lights, ${config.darkPaths.length} darks, ${config.flatPaths.length} flats, ${config.biasPaths.length} biases`);
  try {
    const resultId = await napi.runPipeline(
      config.lightPaths,
      config.darkPaths,
      config.flatPaths,
      config.biasPaths,
      config.bayerPattern || undefined,
      config.stackingConfig,
    );
    console.log(`[pipeline] Complete. Result ID: ${resultId}`);
    return resultId;
  } catch (err: any) {
    console.error('[pipeline] Failed:', err?.message || err);
    throw err;
  }
});

// ─── App lifecycle ──────────────────────────────────────────────────────────

app.whenReady().then(() => {
  createWindow();

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });
});

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit();
  }
});
