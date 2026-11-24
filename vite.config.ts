import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import path from 'path';
import {defineConfig} from 'vite';

export default defineConfig(() => {
  return {
    plugins: [react(), tailwindcss()],
    // Only Talktual's own entry: the Rust workspace under communication-engine/
    // can contain generated HTML (cargo doc) that the dev dependency scan must
    // not pick up.
    optimizeDeps: { entries: ['index.html'] },
    server: { watch: { ignored: ['**/communication-engine/**'] } },
    resolve: {
      alias: {
        '@': path.resolve(__dirname, '.'),
      },
    },
  };
});
