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
    // Rollup tree-shaking currently exhausts memory on the React/xterm graph.
    // Keep the production minifier enabled while retaining modules for now.
    rollupOptions: {
      treeshake: false,
    },
  },
});
