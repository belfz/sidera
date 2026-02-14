import React, { useMemo } from 'react';
import { useAppStore } from '../store';

const StatusBar: React.FC = () => {
  const isProcessing = useAppStore((s) => s.isProcessing);
  const progress = useAppStore((s) => s.progress);
  const error = useAppStore((s) => s.error);
  const lights = useAppStore((s) => s.lights);
  const darks = useAppStore((s) => s.darks);
  const flats = useAppStore((s) => s.flats);
  const biases = useAppStore((s) => s.biases);
  const selectedImageId = useAppStore((s) => s.selectedImageId);

  const selectedFrame = useMemo(() => {
    const allFrames = [...lights, ...darks, ...flats, ...biases];
    return allFrames.find((f) => f.id === selectedImageId) ?? null;
  }, [lights, darks, flats, biases, selectedImageId]);

  return (
    <div className="h-7 bg-astro-surface border-t border-astro-border flex items-center px-3 text-xs text-astro-text-dim select-none">
      {/* Left: status or error */}
      <div className="flex-1 flex items-center gap-3 min-w-0">
        {error && (
          <span className="text-astro-danger truncate" title={error}>
            Error: {error}
          </span>
        )}

        {isProcessing && progress && (
          <div className="flex items-center gap-2 min-w-0">
            <span className="truncate">{progress.stage}</span>
            <div className="w-24 h-1.5 bg-astro-border rounded-full overflow-hidden">
              <div
                className="h-full bg-astro-accent rounded-full transition-all duration-300"
                style={{ width: `${progress.progress * 100}%` }}
              />
            </div>
          </div>
        )}

        {!isProcessing && !error && (
          <span>
            {lights.length > 0
              ? `${lights.length} light frame${lights.length !== 1 ? 's' : ''} loaded`
              : 'Ready'}
          </span>
        )}
      </div>

      {/* Right: selected frame info */}
      <div className="flex items-center gap-4">
        {selectedFrame && (
          <>
            <span>
              {selectedFrame.width} x {selectedFrame.height}
            </span>
            <span>
              {selectedFrame.channels === 1 ? 'Mono' : 'RGB'}
            </span>
            <span>
              {selectedFrame.bitpix > 0
                ? `${selectedFrame.bitpix}-bit int`
                : `${Math.abs(selectedFrame.bitpix)}-bit float`}
            </span>
          </>
        )}
      </div>
    </div>
  );
};

export default StatusBar;
