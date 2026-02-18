import React, { useRef, useEffect } from 'react';
import { useAppStore } from '../store';

const LogPanel: React.FC = () => {
  const logLines = useAppStore((s) => s.logLines);
  const addLogLine = useAppStore((s) => s.addLogLine);
  const clearLog = useAppStore((s) => s.clearLog);
  const scrollRef = useRef<HTMLDivElement>(null);
  const isProcessing = useAppStore((s) => s.isProcessing);
  const progress = useAppStore((s) => s.progress);

  // Subscribe to log events from the CLI process
  useEffect(() => {
    const unsubscribe = window.astro.onLog((line: string) => {
      addLogLine(line);
    });
    return unsubscribe;
  }, [addLogLine]);

  // Auto-scroll to bottom when new lines are added
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [logLines]);

  return (
    <div className="border-t border-astro-border flex flex-col" style={{ height: '160px' }}>
      {/* Header bar */}
      <div className="flex items-center justify-between px-3 py-1 bg-astro-surface border-b border-astro-border flex-shrink-0">
        <div className="flex items-center gap-3">
          <span className="text-xs font-medium text-astro-text-dim">Output</span>
          {isProcessing && progress && (
            <div className="flex items-center gap-2">
              <span className="text-xs text-astro-accent">{progress.stage}</span>
              <div className="w-20 h-1 bg-astro-border rounded-full overflow-hidden">
                <div
                  className="h-full bg-astro-accent rounded-full transition-all duration-300"
                  style={{ width: `${progress.progress * 100}%` }}
                />
              </div>
              <span className="text-xs text-astro-text-dim font-mono">
                {Math.round(progress.progress * 100)}%
              </span>
            </div>
          )}
        </div>
        <div className="flex items-center gap-2">
          <button
            className="text-xs text-astro-text-dim hover:text-astro-accent transition-colors disabled:opacity-30"
            onClick={() => window.astro.saveLogs()}
            disabled={logLines.length === 0}
            title="Save logs to file for diagnostics"
          >
            Save Logs
          </button>
          <button
            className="text-xs text-astro-text-dim hover:text-astro-text transition-colors"
            onClick={clearLog}
            title="Clear log"
          >
            Clear
          </button>
        </div>
      </div>

      {/* Log content */}
      <div
        ref={scrollRef}
        className="flex-1 overflow-y-auto overflow-x-hidden px-3 py-1"
        style={{ backgroundColor: '#060a12' }}
      >
        {logLines.length === 0 ? (
          <div className="text-xs text-astro-text-dim/50 py-2 select-none">
            Processing engine output will appear here...
          </div>
        ) : (
          logLines.map((line, i) => (
            <div
              key={i}
              className={`text-xs font-mono leading-relaxed whitespace-pre-wrap break-all ${getLineClass(line)}`}
            >
              {line}
            </div>
          ))
        )}
      </div>
    </div>
  );
};

/** Color log lines based on their content/level. */
function getLineClass(line: string): string {
  if (line.includes('ERROR') || line.includes('error') || line.includes('FAILED') || line.includes('[system]')) {
    return 'text-red-400';
  }
  if (line.includes('WARN') || line.includes('warn')) {
    return 'text-yellow-400';
  }
  if (line.includes('Pipeline') || line.includes('Complete') || line.includes('SUCCESS')) {
    return 'text-green-400';
  }
  if (line.includes('INFO')) {
    return 'text-slate-300';
  }
  return 'text-slate-400';
}

export default LogPanel;
