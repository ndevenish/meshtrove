import { execSync } from 'node:child_process'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// `git describe --long` renders "52 commits past tag v1.1" as `v1.1-52-gebc55f2`.
// Fold the count in as a version component instead — `v1.1.52-gebc55f2`. Tags
// stay two-component because that third slot is derived, and --long emits the
// count even at zero, so a build on the tag reads v1.1.0-gebc55f2 rather than a
// bare v1.1. Mirrors normalize() in backend/build.rs; the two stamps are
// compared for equality to detect a redeploy, so they must agree character for
// character.
function normalize(describe: string): string {
  return describe.replace(/-(\d+)-(g[0-9a-f]+(?:-dirty)?)$/, '.$1-$2')
}

// Version stamp shared with the backend (build.rs derives it the same way), so
// client and server versions can be compared to detect a redeploy.
//
// The image build has no .git in its context, so it injects APP_VERSION as a
// build arg (see Dockerfile) exactly as the backend stage does — without that
// the SPA in the image would stamp itself "unknown" and never match the server.
function appVersion(): string {
  const injected = process.env.APP_VERSION?.trim()
  if (injected) return normalize(injected)
  try {
    return normalize(execSync('git describe --tags --always --long --dirty').toString().trim())
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
    __APP_VERSION__: JSON.stringify(appVersion()),
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
