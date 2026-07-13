import { execSync } from 'node:child_process'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// Version stamp shared with the backend (build.rs uses the same git describe),
// so client and server versions can be compared to detect a redeploy.
function gitVersion(): string {
  try {
    return execSync('git describe --tags --always --dirty').toString().trim()
  } catch {
    return 'unknown'
  }
}

// The backend proxies to this dev server (same-origin trick, no CORS): the port
// must stay in sync with the backend's MESHTROVE_VITE_URL default. HMR talks to
// Vite directly; the backend also proxies the HMR websocket when the app is
// opened through the backend origin (http://localhost:3000).
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    strictPort: true,
    hmr: { host: 'localhost', port: 5173, protocol: 'ws' },
  },
  define: {
    __APP_VERSION__: JSON.stringify(gitVersion()),
  },
  optimizeDeps: {
    include: [
      '@mui/material',
      '@mui/icons-material',
      '@emotion/react',
      '@emotion/styled',
      'react-router-dom',
      '@tanstack/react-query',
    ],
  },
})
