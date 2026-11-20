import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
  },
  build: {
    // Rollup tree-shaking on the React/xterm graph costs far more than it saves.
    // Measured 2026-11-20: on, 1,045 KB of JS in 73 s at ~3 GB peak memory;
    // off, 1,053 KB in 0.5 s at ~450 MB. Keep it off; the minifier stays on.
    rollupOptions: {
      treeshake: false,
    },
  },
});
