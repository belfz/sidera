import React, { useCallback, useEffect } from 'react';
import { useAppStore } from '../store';

const StackingPanel: React.FC = () => {
  const lights = useAppStore((s) => s.lights);
  const darks = useAppStore((s) => s.darks);
  const flats = useAppStore((s) => s.flats);
  const biases = useAppStore((s) => s.biases);
  const stackingConfig = useAppStore((s) => s.stackingConfig);
  const bayerPattern = useAppStore((s) => s.bayerPattern);
  const isProcessing = useAppStore((s) => s.isProcessing);
  const resultImageId = useAppStore((s) => s.resultImageId);
  const setStackingConfig = useAppStore((s) => s.setStackingConfig);
  const setBayerPattern = useAppStore((s) => s.setBayerPattern);
  const setProcessing = useAppStore((s) => s.setProcessing);
  const setProgress = useAppStore((s) => s.setProgress);
  const setResultImageId = useAppStore((s) => s.setResultImageId);
  const setError = useAppStore((s) => s.setError);

  // Subscribe to pipeline progress updates from the CLI process
  useEffect(() => {
    const unsubscribe = window.astro.onProgress((data) => {
      setProgress({ stage: data.stage, progress: data.percent });
    });
    return unsubscribe;
  }, [setProgress]);

  const canStack = lights.length > 0 && !isProcessing;

  const handleStack = useCallback(async () => {
    if (!canStack) return;

    setProcessing(true);
    setError(null);
    setProgress({ stage: 'Starting pipeline...', progress: 0 });

    try {
      const resultId = await window.astro.runPipeline({
        lightPaths: lights.map((f) => f.path),
        darkPaths: darks.map((f) => f.path),
        flatPaths: flats.map((f) => f.path),
        biasPaths: biases.map((f) => f.path),
        bayerPattern: bayerPattern || undefined,
        stackingConfig,
      });

      setResultImageId(resultId);
      setProgress({ stage: 'Complete', progress: 1 });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      console.error('Pipeline failed:', err);
    } finally {
      setProcessing(false);
    }
  }, [
    canStack, lights, darks, flats, biases, bayerPattern, stackingConfig,
    setProcessing, setProgress, setResultImageId, setError,
  ]);

  const handleSave = useCallback(async () => {
    const resultId = useAppStore.getState().resultImageId;
    if (!resultId) return;

    try {
      const result = await window.astro.saveFile({
        title: 'Save stacked image',
        defaultPath: 'stacked',
        filters: [
          { name: 'FITS', extensions: ['fits'] },
          { name: 'TIFF', extensions: ['tiff', 'tif'] },
          { name: 'PNG', extensions: ['png'] },
        ],
      });

      if (result.canceled) return;

      const ext = result.filePath.split('.').pop()?.toLowerCase() || 'fits';
      const format = ext === 'tif' || ext === 'tiff' ? 'tiff' : ext;
      const stretch = ext === 'png' ? useAppStore.getState().stretch : undefined;

      await window.astro.saveImage(resultId, result.filePath, format, stretch);
    } catch (err) {
      console.error('Save failed:', err);
    }
  }, []);

  const showSigmaParams =
    stackingConfig.method === 'sigma_clip_mean' ||
    stackingConfig.method === 'sigma_clip_median';

  return (
    <div className="panel m-2 flex-1">
      <div className="panel-header">Stacking</div>

      <div className="p-3 space-y-4">
        {/* Stacking method */}
        <div>
          <label className="text-xs text-astro-text-dim block mb-1">Method</label>
          <select
            className="select-field"
            value={stackingConfig.method}
            onChange={(e) => setStackingConfig({ method: e.target.value as any })}
          >
            <option value="mean">Mean (Average)</option>
            <option value="median">Median</option>
            <option value="sigma_clip_mean">Sigma-Clipped Mean</option>
            <option value="sigma_clip_median">Sigma-Clipped Median</option>
          </select>
        </div>

        {/* Sigma clipping parameters */}
        {showSigmaParams && (
          <>
            <div>
              <label className="text-xs text-astro-text-dim block mb-1">
                Kappa (sigma): {stackingConfig.kappa.toFixed(1)}
              </label>
              <input
                type="range"
                min={1}
                max={5}
                step={0.1}
                value={stackingConfig.kappa}
                onChange={(e) => setStackingConfig({ kappa: parseFloat(e.target.value) })}
                className="slider"
              />
            </div>
            <div>
              <label className="text-xs text-astro-text-dim block mb-1">
                Iterations: {stackingConfig.iterations}
              </label>
              <input
                type="range"
                min={1}
                max={10}
                step={1}
                value={stackingConfig.iterations}
                onChange={(e) => setStackingConfig({ iterations: parseInt(e.target.value) })}
                className="slider"
              />
            </div>
          </>
        )}

        {/* Bayer pattern */}
        <div>
          <label className="text-xs text-astro-text-dim block mb-1">Bayer Pattern</label>
          <select
            className="select-field"
            value={bayerPattern || 'auto'}
            onChange={(e) => setBayerPattern(e.target.value === 'auto' ? null : e.target.value)}
          >
            <option value="auto">Auto-detect</option>
            <option value="RGGB">RGGB</option>
            <option value="BGGR">BGGR</option>
            <option value="GRBG">GRBG</option>
            <option value="GBRG">GBRG</option>
            <option value="none">None (mono)</option>
          </select>
        </div>

        {/* Summary */}
        <div className="bg-astro-bg rounded-md p-3 text-xs space-y-1">
          <div className="flex justify-between">
            <span className="text-astro-text-dim">Lights</span>
            <span>{lights.length} frames</span>
          </div>
          <div className="flex justify-between">
            <span className="text-astro-text-dim">Darks</span>
            <span>{darks.length} frames</span>
          </div>
          <div className="flex justify-between">
            <span className="text-astro-text-dim">Flats</span>
            <span>{flats.length} frames</span>
          </div>
          <div className="flex justify-between">
            <span className="text-astro-text-dim">Biases</span>
            <span>{biases.length} frames</span>
          </div>
        </div>

        {/* Action buttons */}
        <div className="space-y-2">
          <button
            className="btn-primary w-full"
            disabled={!canStack}
            onClick={handleStack}
          >
            {isProcessing ? (
              <span className="flex items-center justify-center gap-2">
                <svg className="animate-spin w-4 h-4" fill="none" viewBox="0 0 24 24">
                  <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                  <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z" />
                </svg>
                Processing...
              </span>
            ) : (
              `Stack ${lights.length} Light${lights.length !== 1 ? 's' : ''}`
            )}
          </button>

          {resultImageId && (
            <button className="btn-secondary w-full" onClick={handleSave}>
              Save Result
            </button>
          )}
        </div>
      </div>
    </div>
  );
};

export default StackingPanel;
