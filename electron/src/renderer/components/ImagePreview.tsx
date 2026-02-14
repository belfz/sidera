import React, { useRef, useEffect, useState, useCallback } from 'react';
import { useAppStore } from '../store';

const ImagePreview: React.FC = () => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const selectedImageId = useAppStore((s) => s.selectedImageId);
  const resultImageId = useAppStore((s) => s.resultImageId);
  const stretch = useAppStore((s) => s.stretch);
  const lights = useAppStore((s) => s.lights);
  const darks = useAppStore((s) => s.darks);
  const flats = useAppStore((s) => s.flats);
  const biases = useAppStore((s) => s.biases);

  const [zoom, setZoom] = useState(1);
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const [isDragging, setIsDragging] = useState(false);
  const [dragStart, setDragStart] = useState({ x: 0, y: 0 });

  const setPreviewStoreId = useAppStore((s) => s.setPreviewStoreId);

  // Track which preview image we currently have loaded in the store
  const [loadedPreviewId, setLoadedPreviewId] = useState<string | null>(null);
  const prevPreviewIdRef = useRef<string | null>(null);

  // Determine what to display:
  // - If there's a pipeline result, show that (it's already in the store).
  // - Otherwise, if a file is selected, load it for preview.
  const hasResult = !!resultImageId;
  const displayId = hasResult ? resultImageId : selectedImageId;

  // Find the file path for the selected image (for on-demand loading)
  const allFrames = [...lights, ...darks, ...flats, ...biases];
  const selectedFile = allFrames.find((f) => f.id === selectedImageId);

  // Load and render preview
  useEffect(() => {
    if (!canvasRef.current) return;

    let cancelled = false;

    const loadAndRender = async () => {
      try {
        let storeImageId: string | null = null;

        if (hasResult && resultImageId) {
          // Pipeline result is already in the store
          storeImageId = resultImageId;
        } else if (selectedFile) {
          // Need to load this file into the store for preview
          // Release previous preview if different
          if (prevPreviewIdRef.current && prevPreviewIdRef.current !== loadedPreviewId) {
            try { await window.astro.releaseImage(prevPreviewIdRef.current); } catch {}
          }

          const id = await window.astro.loadPreview(selectedFile.path);
          if (cancelled) {
            // We navigated away — release immediately
            try { await window.astro.releaseImage(id); } catch {}
            return;
          }
          storeImageId = id;
          setLoadedPreviewId(id);
          setPreviewStoreId(id);
          prevPreviewIdRef.current = id;
        }

        if (!storeImageId || !canvasRef.current || cancelled) return;

        const info = await window.astro.getImageInfo(storeImageId);
        if (!info || !canvasRef.current || cancelled) return;

        const preview = await window.astro.getPreview(storeImageId, stretch);
        if (!preview || !canvasRef.current || cancelled) return;

        const canvas = canvasRef.current;
        const ctx = canvas.getContext('2d');
        if (!ctx) return;

        canvas.width = info.width;
        canvas.height = info.height;

        const imageData = new ImageData(
          new Uint8ClampedArray(preview),
          info.width,
          info.height,
        );
        ctx.putImageData(imageData, 0, 0);
      } catch (err) {
        console.error('Failed to load preview:', err);
      }
    };

    loadAndRender();

    return () => {
      cancelled = true;
    };
  }, [displayId, stretch, hasResult, resultImageId, selectedFile]);

  // Clean up preview when component unmounts or selection changes away
  useEffect(() => {
    return () => {
      if (prevPreviewIdRef.current) {
        window.astro.releaseImage(prevPreviewIdRef.current).catch(() => {});
      }
    };
  }, []);

  // Handle mouse wheel zoom
  const handleWheel = useCallback(
    (e: React.WheelEvent) => {
      e.preventDefault();
      const delta = e.deltaY > 0 ? 0.9 : 1.1;
      setZoom((z) => Math.max(0.1, Math.min(20, z * delta)));
    },
    [],
  );

  // Handle drag to pan
  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      setIsDragging(true);
      setDragStart({ x: e.clientX - offset.x, y: e.clientY - offset.y });
    },
    [offset],
  );

  const handleMouseMove = useCallback(
    (e: React.MouseEvent) => {
      if (!isDragging) return;
      setOffset({
        x: e.clientX - dragStart.x,
        y: e.clientY - dragStart.y,
      });
    },
    [isDragging, dragStart],
  );

  const handleMouseUp = useCallback(() => {
    setIsDragging(false);
  }, []);

  // Fit to window
  const handleFitToWindow = useCallback(() => {
    setZoom(1);
    setOffset({ x: 0, y: 0 });
  }, []);

  return (
    <div className="h-full flex flex-col">
      {/* Toolbar */}
      <div className="h-9 flex items-center justify-between px-3 border-b border-astro-border bg-astro-surface">
        <div className="flex items-center gap-2">
          <button className="btn-icon" onClick={handleFitToWindow} title="Fit to window">
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 8V4m0 0h4M4 4l5 5m11-1V4m0 0h-4m4 0l-5 5M4 16v4m0 0h4m-4 0l5-5m11 5l-5-5m5 5v-4m0 4h-4" />
            </svg>
          </button>
          <button className="btn-icon" onClick={() => setZoom((z) => z * 1.25)} title="Zoom in">
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0zM10 7v3m0 0v3m0-3h3m-3 0H7" />
            </svg>
          </button>
          <button className="btn-icon" onClick={() => setZoom((z) => z * 0.8)} title="Zoom out">
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0zM13 10H7" />
            </svg>
          </button>
        </div>

        <span className="text-xs text-astro-text-dim">
          {Math.round(zoom * 100)}%
        </span>
      </div>

      {/* Canvas area */}
      <div
        ref={containerRef}
        className="flex-1 overflow-hidden bg-astro-bg cursor-grab active:cursor-grabbing"
        onWheel={handleWheel}
        onMouseDown={handleMouseDown}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        onMouseLeave={handleMouseUp}
      >
        {displayId ? (
          <div
            className="w-full h-full flex items-center justify-center"
            style={{
              transform: `translate(${offset.x}px, ${offset.y}px) scale(${zoom})`,
              transformOrigin: 'center',
            }}
          >
            <canvas
              ref={canvasRef}
              className="max-w-none"
              style={{ imageRendering: zoom > 2 ? 'pixelated' : 'auto' }}
            />
          </div>
        ) : (
          <div className="w-full h-full flex items-center justify-center">
            <div className="text-center">
              <svg
                className="w-16 h-16 mx-auto mb-4 text-astro-border"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1}
                  d="M4 16l4.586-4.586a2 2 0 012.828 0L16 16m-2-2l1.586-1.586a2 2 0 012.828 0L20 14m-6-6h.01M6 20h12a2 2 0 002-2V6a2 2 0 00-2-2H6a2 2 0 00-2 2v12a2 2 0 002 2z"
                />
              </svg>
              <p className="text-astro-text-dim text-sm">
                Import light frames to get started
              </p>
              <p className="text-astro-text-dim text-xs mt-1">
                Use the panel on the left to add your FITS files
              </p>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};

export default ImagePreview;
