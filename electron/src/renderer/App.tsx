import React from 'react';
import TitleBar from './components/TitleBar';
import FilePanel from './components/FilePanel';
import ImagePreview from './components/ImagePreview';
import HistogramPanel from './components/HistogramPanel';
import StackingPanel from './components/StackingPanel';
import StatusBar from './components/StatusBar';

const App: React.FC = () => {
  return (
    <div className="h-screen w-screen flex flex-col bg-astro-bg">
      {/* Title bar / drag region */}
      <TitleBar />

      {/* Main content area */}
      <div className="flex-1 flex min-h-0">
        {/* Left sidebar: File panel */}
        <div className="w-72 flex-shrink-0 border-r border-astro-border overflow-y-auto">
          <FilePanel />
        </div>

        {/* Center: Image preview */}
        <div className="flex-1 min-w-0">
          <ImagePreview />
        </div>

        {/* Right sidebar: Histogram + Stacking settings */}
        <div className="w-80 flex-shrink-0 border-l border-astro-border flex flex-col overflow-y-auto">
          <HistogramPanel />
          <StackingPanel />
        </div>
      </div>

      {/* Status bar */}
      <StatusBar />
    </div>
  );
};

export default App;
