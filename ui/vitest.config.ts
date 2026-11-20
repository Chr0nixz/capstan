import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: "./src/test-setup.ts",
    // Keep the stylesheet's text for the design-token contract test (styles.test.ts);
    // other CSS stays replaced with empty strings.
    css: { include: [/src\/styles\.css/] },
  },
});
