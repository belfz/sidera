import { app, BrowserWindow, ipcMain, dialog } from "electron";
import { spawn, ChildProcess } from "child_process";
import * as path from "path";
import * as fs from "fs";
import * as os from "os";

// ─── CLI process management ──────────────────────────────────────────────────

let cliProcess: ChildProcess | null = null;
let requestId = 0;
const pendingRequests = new Map<
  string,
  { resolve: (data: any) => void; reject: (err: Error) => void }
>();
let stdoutBuffer = "";
let isQuitting = false;

let mainWindow: BrowserWindow | null = null;

// Temp directory for preview RGBA files
const tempDir = path.join(os.tmpdir(), "astro-viber");

function getCliBinaryPath(): string {
  if (process.env.NODE_ENV === "development" || !app.isPackaged) {
    return path.join(__dirname, "../../../target/release/astro-cli");
  }
  const ext = process.platform === "win32" ? ".exe" : "";
  return path.join(process.resourcesPath, `astro-cli${ext}`);
}

function startCliProcess(): void {
  const binPath = getCliBinaryPath();
  console.log("[astro-viber] Starting CLI process:", binPath);

  try {
    cliProcess = spawn(binPath, [], {
      stdio: ["pipe", "pipe", "pipe"],
      env: { ...process.env, RUST_LOG: "info" },
    });
  } catch (err: any) {
    console.error("[astro-viber] Failed to spawn CLI process:", err?.message);
    return;
  }

  cliProcess.stdout!.on("data", (data: Buffer) => {
    stdoutBuffer += data.toString();
    const lines = stdoutBuffer.split("\n");
    stdoutBuffer = lines.pop() || "";

    for (const line of lines) {
      if (!line.trim()) continue;
      try {
        const msg = JSON.parse(line);

        if (msg.progress) {
          mainWindow?.webContents.send("cli:progress", {
            id: msg.id,
            stage: msg.progress.stage,
            percent: msg.progress.percent,
          });
          continue;
        }

        if (msg.id) {
          const pending = pendingRequests.get(msg.id);
          if (pending) {
            pendingRequests.delete(msg.id);
            if (msg.ok) {
              pending.resolve(msg.data);
            } else {
              pending.reject(new Error(msg.error || "Unknown CLI error"));
            }
          }
        }
      } catch {
        console.error("[cli stdout] Failed to parse:", line);
      }
    }
  });

  cliProcess.stderr!.on("data", (data: Buffer) => {
    const text = data.toString();
    const lines = text.split("\n").filter((l) => l.trim().length > 0);
    for (const line of lines) {
      mainWindow?.webContents.send("cli:log", line);
    }
  });

  cliProcess.on("exit", (code, signal) => {
    console.error(
      `[astro-viber] CLI process exited (code=${code}, signal=${signal})`,
    );

    for (const [, pending] of pendingRequests) {
      pending.reject(
        new Error(`CLI process exited unexpectedly (code ${code})`),
      );
    }
    pendingRequests.clear();
    cliProcess = null;

    if (!isQuitting) {
      mainWindow?.webContents.send(
        "cli:log",
        `[system] Processing engine exited (code ${code}). Restarting...`,
      );
      setTimeout(startCliProcess, 1000);
    }
  });

  cliProcess.on("error", (err) => {
    console.error("[astro-viber] CLI process error:", err.message);
    mainWindow?.webContents.send(
      "cli:log",
      `[system] Processing engine error: ${err.message}`,
    );
  });
}

function sendCommand(
  cmd: string,
  params: Record<string, any> = {},
): Promise<any> {
  return new Promise((resolve, reject) => {
    if (!cliProcess || !cliProcess.stdin || cliProcess.killed) {
      reject(new Error("Processing engine not running"));
      return;
    }

    const id = `req_${++requestId}`;
    pendingRequests.set(id, { resolve, reject });

    const request = JSON.stringify({ id, cmd, ...params }) + "\n";
    cliProcess.stdin.write(request, (err) => {
      if (err) {
        pendingRequests.delete(id);
        reject(new Error(`Failed to send command: ${err.message}`));
      }
    });
  });
}

// ─── Window creation ─────────────────────────────────────────────────────────

function createWindow(): void {
  mainWindow = new BrowserWindow({
    width: 1400,
    height: 900,
    minWidth: 1000,
    minHeight: 700,
    backgroundColor: "#0a0e17",
    titleBarStyle: "hiddenInset",
    trafficLightPosition: { x: 16, y: 16 },
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
  });

  if (process.env.NODE_ENV === "development" || !app.isPackaged) {
    mainWindow.loadURL("http://localhost:5173");
  } else {
    mainWindow.loadFile(path.join(__dirname, "../renderer/index.html"));
  }

  mainWindow.on("closed", () => {
    mainWindow = null;
  });
}

// ─── App lifecycle ───────────────────────────────────────────────────────────

app.whenReady().then(() => {
  fs.mkdirSync(tempDir, { recursive: true });

  // ─── Register IPC handlers ─────────────────────────────────────────

  ipcMain.handle(
    "dialog:open-files",
    async (_event, options: { title?: string; filters?: any[] }) => {
      if (!mainWindow) return { canceled: true, filePaths: [] };
      return await dialog.showOpenDialog(mainWindow, {
        title: options.title || "Select FITS files",
        filters: options.filters || [
          { name: "FITS Files", extensions: ["fits", "fit", "fts"] },
          { name: "All Files", extensions: ["*"] },
        ],
        properties: ["openFile", "multiSelections"],
      });
    },
  );

  ipcMain.handle(
    "dialog:save-file",
    async (
      _event,
      options: { title?: string; defaultPath?: string; filters?: any[] },
    ) => {
      if (!mainWindow) return { canceled: true, filePath: "" };
      return await dialog.showSaveDialog(mainWindow, {
        title: options.title || "Save image",
        defaultPath: options.defaultPath,
        filters: options.filters || [
          { name: "FITS", extensions: ["fits"] },
          { name: "TIFF", extensions: ["tiff", "tif"] },
          { name: "PNG", extensions: ["png"] },
        ],
      });
    },
  );

  // ─── CLI bridge IPC handlers ─────────────────────────────────────────

  ipcMain.handle("native:load-fits", async (_event, filePath: string) => {
    return await sendCommand("info", { path: filePath });
  });

  ipcMain.handle("native:load-preview", async (_event, filePath: string) => {
    const result = await sendCommand("load", { path: filePath });
    return result.imageId;
  });

  ipcMain.handle("native:release-image", async (_event, imageId: string) => {
    try {
      await sendCommand("release", { imageId });
    } catch {
      /* ignore */
    }
  });

  ipcMain.handle("native:release-all-images", async () => {
    try {
      await sendCommand("releaseAll");
    } catch {
      /* ignore */
    }
  });

  ipcMain.handle("native:get-image-info", async (_event, imageId: string) => {
    return await sendCommand("imageInfo", { imageId });
  });

  ipcMain.handle(
    "native:get-preview",
    async (_event, imageId: string, stretch?: any) => {
      const outputPath = path.join(
        tempDir,
        `preview_${Date.now()}_${Math.random().toString(36).slice(2)}.rgba`,
      );
      try {
        await sendCommand("preview", {
          imageId,
          outputPath,
          stretch: stretch || null,
        });
        const buffer = await fs.promises.readFile(outputPath);
        fs.promises.unlink(outputPath).catch(() => {});
        return buffer;
      } catch (err) {
        fs.promises.unlink(outputPath).catch(() => {});
        throw err;
      }
    },
  );

  ipcMain.handle(
    "native:get-histogram",
    async (_event, imageId: string, channel: number, bins: number) => {
      try {
        return await sendCommand("histogram", { imageId, channel, bins });
      } catch {
        return { bins: new Array(bins).fill(0), min: 0, max: 65535, channel };
      }
    },
  );

  ipcMain.handle("native:get-auto-stretch", async (_event, imageId: string) => {
    try {
      return await sendCommand("autoStretch", { imageId });
    } catch {
      return { shadows: 0.0, midtones: 0.25, highlights: 1.0 };
    }
  });

  ipcMain.handle(
    "native:save-image",
    async (
      _event,
      imageId: string,
      filePath: string,
      format: string,
      stretch?: any,
    ) => {
      return await sendCommand("save", {
        imageId,
        outputPath: filePath,
        format,
        stretch: stretch || null,
      });
    },
  );

  ipcMain.handle(
    "native:run-pipeline",
    async (
      _event,
      config: {
        lightPaths: string[];
        darkPaths: string[];
        flatPaths: string[];
        biasPaths: string[];
        bayerPattern?: string;
        stackingConfig: { method: string; kappa?: number; iterations?: number };
      },
    ) => {
      console.log(
        `[pipeline] Starting: ${config.lightPaths.length} lights, ${config.darkPaths.length} darks`,
      );

      const result = await sendCommand("pipeline", {
        lightPaths: config.lightPaths,
        darkPaths: config.darkPaths,
        flatPaths: config.flatPaths,
        biasPaths: config.biasPaths,
        bayerPattern: config.bayerPattern || null,
        stackMethod: config.stackingConfig.method,
        kappa: config.stackingConfig.kappa ?? 3.0,
        iterations: config.stackingConfig.iterations ?? 5,
      });

      console.log(`[pipeline] Complete. Result ID: ${result.imageId}`);
      return result.imageId;
    },
  );

  // ─── Start CLI engine and create window ─────────────────────────────

  startCliProcess();
  createWindow();

  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });
});

app.on("before-quit", () => {
  isQuitting = true;
});

app.on("window-all-closed", () => {
  if (cliProcess) {
    cliProcess.kill();
    cliProcess = null;
  }
  if (process.platform !== "darwin") {
    app.quit();
  }
});
