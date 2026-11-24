import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { BenchmarkApp } from "./benchmark/BenchmarkApp";
import { detectLocale } from "./i18n";
import { isMacOS } from "./window/platform";
import "@xterm/xterm/css/xterm.css";
import "./styles.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("Capstan root element is missing");
}
// Screen readers pick their voice from lang, so it follows the UI language.
document.documentElement.lang = detectLocale();
if (isMacOS()) document.documentElement.dataset.platform = "macos";

createRoot(root).render(
  <StrictMode>
    {window.location.hash === "#/m0-benchmark" ? <BenchmarkApp /> : <App />}
  </StrictMode>,
);
