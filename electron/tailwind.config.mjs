/** @type {import('tailwindcss').Config} */
export default {
  content: ['./src/renderer/**/*.{html,tsx,ts,jsx,js}'],
  theme: {
    extend: {
      colors: {
        // Dark astrophotography-themed palette
        'astro-bg': '#0a0e17',
        'astro-surface': '#121828',
        'astro-surface-light': '#1a2338',
        'astro-border': '#2a3548',
        'astro-text': '#c8d6e5',
        'astro-text-dim': '#6b7b8f',
        'astro-accent': '#4a9eff',
        'astro-accent-dim': '#2a6bbf',
        'astro-success': '#2ed573',
        'astro-warning': '#ffa502',
        'astro-danger': '#ff4757',
        'astro-red': '#ff6b6b',
        'astro-green': '#51cf66',
        'astro-blue': '#339af0',
      },
      fontFamily: {
        mono: ['JetBrains Mono', 'Fira Code', 'monospace'],
      },
    },
  },
  plugins: [],
};
