import React, { useCallback } from 'react';
import { useAppStore } from '../store';
import type { FileInfo, FrameType } from '../types';

/** Extract filename from a full path. */
const basename = (path: string) => path.split(/[\\/]/).pop() || path;

const FilePanel: React.FC = () => {
  const { lights, darks, flats, biases, addFrames, removeFrame, clearFrames, setSelectedImageId } =
    useAppStore();

  const handleImport = useCallback(
    async (frameType: FrameType) => {
      try {
        const result = await window.astro.openFiles({
          title: `Import ${frameType} Frames`,
        });
        if (result.canceled || result.filePaths.length === 0) return;

        const files: FileInfo[] = [];
        for (const filePath of result.filePaths) {
          const info = await window.astro.loadFits(filePath);
          files.push({
            ...info,
            filename: basename(filePath),
            frameType,
          });
        }

        addFrames(frameType, files);
      } catch (err) {
        console.error(`Failed to import ${frameType} frames:`, err);
      }
    },
    [addFrames],
  );

  return (
    <div className="flex flex-col h-full">
      <div className="panel-header">Frames</div>

      <div className="flex-1 overflow-y-auto p-2 space-y-2">
        <FrameGroup
          label="Lights"
          type="Light"
          frames={lights}
          badgeClass="badge-light"
          onImport={() => handleImport('Light')}
          onRemove={(id) => removeFrame('Light', id)}
          onClear={() => clearFrames('Light')}
          onSelect={setSelectedImageId}
        />

        <FrameGroup
          label="Darks"
          type="Dark"
          frames={darks}
          badgeClass="badge-dark"
          onImport={() => handleImport('Dark')}
          onRemove={(id) => removeFrame('Dark', id)}
          onClear={() => clearFrames('Dark')}
          onSelect={setSelectedImageId}
        />

        <FrameGroup
          label="Flats"
          type="Flat"
          frames={flats}
          badgeClass="badge-flat"
          onImport={() => handleImport('Flat')}
          onRemove={(id) => removeFrame('Flat', id)}
          onClear={() => clearFrames('Flat')}
          onSelect={setSelectedImageId}
        />

        <FrameGroup
          label="Biases"
          type="Bias"
          frames={biases}
          badgeClass="badge-bias"
          onImport={() => handleImport('Bias')}
          onRemove={(id) => removeFrame('Bias', id)}
          onClear={() => clearFrames('Bias')}
          onSelect={setSelectedImageId}
        />
      </div>
    </div>
  );
};

interface FrameGroupProps {
  label: string;
  type: FrameType;
  frames: FileInfo[];
  badgeClass: string;
  onImport: () => void;
  onRemove: (id: string) => void;
  onClear: () => void;
  onSelect: (id: string) => void;
}

const FrameGroup: React.FC<FrameGroupProps> = ({
  label,
  frames,
  badgeClass,
  onImport,
  onRemove,
  onClear,
  onSelect,
}) => {
  const [expanded, setExpanded] = React.useState(true);

  return (
    <div className="panel">
      {/* Header */}
      <button
        className="w-full flex items-center justify-between px-3 py-2 hover:bg-astro-surface-light transition-colors"
        onClick={() => setExpanded(!expanded)}
      >
        <div className="flex items-center gap-2">
          <svg
            className={`w-3 h-3 text-astro-text-dim transition-transform ${expanded ? 'rotate-90' : ''}`}
            fill="currentColor"
            viewBox="0 0 20 20"
          >
            <path
              fillRule="evenodd"
              d="M7.293 14.707a1 1 0 010-1.414L10.586 10 7.293 6.707a1 1 0 011.414-1.414l4 4a1 1 0 010 1.414l-4 4a1 1 0 01-1.414 0z"
              clipRule="evenodd"
            />
          </svg>
          <span className="text-sm font-medium">{label}</span>
          <span className={badgeClass}>{frames.length}</span>
        </div>

        <div className="flex items-center gap-1 no-drag">
          <button
            className="btn-icon"
            onClick={(e) => {
              e.stopPropagation();
              onImport();
            }}
            title={`Import ${label}`}
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
            </svg>
          </button>
          {frames.length > 0 && (
            <button
              className="btn-icon text-astro-danger"
              onClick={(e) => {
                e.stopPropagation();
                onClear();
              }}
              title={`Clear ${label}`}
            >
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
              </svg>
            </button>
          )}
        </div>
      </button>

      {/* Frame list */}
      {expanded && frames.length > 0 && (
        <div className="border-t border-astro-border max-h-40 overflow-y-auto">
          {frames.map((frame, index) => (
            <div
              key={`${frame.id}-${index}`}
              className="flex items-center justify-between px-3 py-1.5 hover:bg-astro-surface-light cursor-pointer group text-xs"
              onClick={() => onSelect(frame.id)}
            >
              <span className="truncate text-astro-text-dim group-hover:text-astro-text">
                {frame.filename}
              </span>
              <div className="flex items-center gap-2 text-astro-text-dim">
                {frame.exposureTime && (
                  <span>{frame.exposureTime}s</span>
                )}
                <button
                  className="opacity-0 group-hover:opacity-100 hover:text-astro-danger transition-opacity"
                  onClick={(e) => {
                    e.stopPropagation();
                    onRemove(frame.id);
                  }}
                >
                  <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                  </svg>
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Empty state */}
      {expanded && frames.length === 0 && (
        <div className="px-3 py-4 text-center">
          <button
            className="text-xs text-astro-text-dim hover:text-astro-accent transition-colors"
            onClick={onImport}
          >
            Click + to import {label.toLowerCase()}
          </button>
        </div>
      )}
    </div>
  );
};

export default FilePanel;
