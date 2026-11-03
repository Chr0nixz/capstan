import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
export default defineConfig({ plugins: [react()], build: {
  outDir: "../target/m2-sftp-layout-build", emptyOutDir: true,
  rollupOptions: { input: "tests/sftp-workflow.html", treeshake: false },
} });
