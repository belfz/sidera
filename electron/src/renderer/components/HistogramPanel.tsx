import React, { useRef, useEffect, useCallback, useState } from "react";
import { useAppStore } from "../store";
import type { HistogramData } from "../types";

const HIST_WIDTH = 280;
const HIST_HEIGHT = 120;
const HIST_BINS = 256;

const HistogramPanel: React.FC = () => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const {
    resultImageId,
    previewStoreId,
    stretch,
    autoStretch,
    setStretch,
    setAutoStretch,
  } = useAppStore();

  const [histData, setHistData] = useState<HistogramData | null>(null);

  // Use the pipeline result if available, otherwise the loaded preview
  const displayId = resultImageId || previewStoreId;

  // Fetch histogram data
  useEffect(() => {
    if (!displayId) {
      setHistData(null);
      return;
    }

    const fetchHistogram = async () => {
      try {
        const data = await window.astro.getHistogram(displayId, 0, HIST_BINS);
        setHistData(data);
      } catch (err) {
        console.error("Failed to fetch histogram:", err);
      }
    };

    fetchHistogram();
  }, [displayId]);

  // Fetch auto-stretch params
  useEffect(() => {
    if (!displayId || !autoStretch) return;

    const fetchAutoStretch = async () => {
      try {
        const params = await window.astro.getAutoStretch(displayId);
        setStretch(params);
      } catch (err) {
        console.error("Failed to fetch auto-stretch:", err);
      }
    };

    fetchAutoStretch();
  }, [displayId, autoStretch, setStretch]);

  // Render histogram on canvas
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !histData) return;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    canvas.width = HIST_WIDTH * dpr;
    canvas.height = HIST_HEIGHT * dpr;
    ctx.scale(dpr, dpr);

    // Clear
    ctx.fillStyle = "#0a0e17";
    ctx.fillRect(0, 0, HIST_WIDTH, HIST_HEIGHT);

    // Find max bin value (excluding extremes for better visualization)
    const bins = histData.bins;
    const trimmedBins = bins.slice(1, bins.length - 1);
    const maxBin = Math.max(...trimmedBins, 1);

    // Draw histogram bars
    const barWidth = HIST_WIDTH / bins.length;
    ctx.fillStyle = "#339af0";
    ctx.globalAlpha = 0.7;

    for (let i = 0; i < bins.length; i++) {
      const value = bins[i];
      // Use log scale for better visualization
      const height =
        value > 0
          ? (Math.log(value + 1) / Math.log(maxBin + 1)) * HIST_HEIGHT
          : 0;
      const x = i * barWidth;
      ctx.fillRect(x, HIST_HEIGHT - height, barWidth, height);
    }

    ctx.globalAlpha = 1.0;

    // Draw stretch markers
    const drawMarker = (pos: number, color: string) => {
      const x = pos * HIST_WIDTH;
      ctx.strokeStyle = color;
      ctx.lineWidth = 1.5;
      ctx.setLineDash([4, 2]);
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, HIST_HEIGHT);
      ctx.stroke();
      ctx.setLineDash([]);
    };

    drawMarker(stretch.shadows, "#ff6b6b"); // Red for shadows
    drawMarker(stretch.midtones, "#51cf66"); // Green for midtones
    drawMarker(stretch.highlights, "#ffa502"); // Yellow for highlights
  }, [histData, stretch]);

  const handleStretchChange = useCallback(
    (key: "shadows" | "midtones" | "highlights", value: number) => {
      setAutoStretch(false);
      setStretch({ ...stretch, [key]: value });
    },
    [stretch, setStretch, setAutoStretch],
  );

  return (
    <div className="panel m-2">
      <div className="panel-header flex items-center justify-between">
        <span>Histogram</span>
        <button
          className={`text-xs px-2 py-0.5 rounded ${
            autoStretch
              ? "bg-astro-accent/20 text-astro-accent"
              : "text-astro-text-dim hover:text-astro-text"
          }`}
          onClick={() => setAutoStretch(!autoStretch)}
          title="Auto-stretch"
        >
          STF
        </button>
      </div>

      <div className="p-3 space-y-3">
        {/* Histogram canvas */}
        <canvas
          ref={canvasRef}
          className="w-full rounded border border-astro-border"
          style={{ width: HIST_WIDTH, height: HIST_HEIGHT }}
        />

        {/* Stretch sliders */}
        <div className="space-y-2">
          <StretchSlider
            label="Shadows"
            value={stretch.shadows}
            color="text-astro-red"
            onChange={(v) => handleStretchChange("shadows", v)}
          />
          <StretchSlider
            label="Midtones"
            value={stretch.midtones}
            color="text-astro-green"
            onChange={(v) => handleStretchChange("midtones", v)}
          />
          <StretchSlider
            label="Highlights"
            value={stretch.highlights}
            color="text-astro-warning"
            onChange={(v) => handleStretchChange("highlights", v)}
          />
        </div>
      </div>
    </div>
  );
};

interface StretchSliderProps {
  label: string;
  value: number;
  color: string;
  onChange: (value: number) => void;
}

const StretchSlider: React.FC<StretchSliderProps> = ({
  label,
  value,
  color,
  onChange,
}) => {
  return (
    <div className="flex items-center gap-2">
      <span className={`text-xs w-16 ${color}`}>{label}</span>
      <input
        type="range"
        min={0}
        max={1}
        step={0.001}
        value={value}
        onChange={(e) => onChange(parseFloat(e.target.value))}
        className="slider flex-1"
      />
      <span className="text-xs text-astro-text-dim w-10 text-right font-mono">
        {value.toFixed(3)}
      </span>
    </div>
  );
};

export default HistogramPanel;
