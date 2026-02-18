import React from 'react';

const TitleBar: React.FC = () => {
  return (
    <div className="h-10 bg-astro-surface border-b border-astro-border flex items-center px-4 drag-region select-none">
      {/* macOS traffic lights occupy the left side */}
      <div className="w-20" />

      {/* App title */}
      <div className="flex-1 text-center">
        <span className="text-sm font-medium text-astro-text-dim tracking-wide">
          sidera
        </span>
      </div>

      {/* Spacer for symmetry */}
      <div className="w-20" />
    </div>
  );
};

export default TitleBar;
