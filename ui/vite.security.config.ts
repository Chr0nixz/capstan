import { defineConfig } from "vite";

export default defineConfig({
  build: {
    outDir: "../target/m1-terminal-security-build",
    emptyOutDir: true,
    rollupOptions: { input: "tests/terminal-security.html", treeshake: false },
  },
});
