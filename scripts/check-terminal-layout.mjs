import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { createServer } from "node:http";
import { access, mkdir, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { dirname, extname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const securityMode = process.argv.includes("--security");
const sftpMode = process.argv.includes("--sftp");
const distribution = sftpMode ? join(repository, "target", "m2-sftp-layout-build") : securityMode ? join(repository, "target", "m1-terminal-security-build") : join(repository, "ui", "dist");
const target = join(repository, "target");
const token = randomUUID();
const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const evidence = join(target, "layout-checks", `${stamp}-${sftpMode ? "sftp" : "terminal"}-${securityMode ? "security" : "layout"}`);
const browserCandidates = [
  process.env.CAPSTAN_TEST_BROWSER,
  join(process.env["ProgramFiles(x86)"] ?? "C:\\Program Files (x86)", "Microsoft", "Edge", "Application", "msedge.exe"),
  join(process.env.ProgramFiles ?? "C:\\Program Files", "Google", "Chrome", "Application", "chrome.exe"),
].filter(Boolean);
let browser;
for (const candidate of browserCandidates) {
  try { await access(candidate); browser = candidate; break; } catch { /* Try the next installed browser. */ }
}
if (!browser) throw new Error("Set CAPSTAN_TEST_BROWSER to an installed Edge or Chrome executable.");
await access(join(distribution, sftpMode ? "tests/sftp-workflow.html" : securityMode ? "tests/terminal-security.html" : "index.html"));
await mkdir(evidence, { recursive: true });
const fixture = await readFile(join(repository, "ui", "tests", "terminal-layout.html"));
const mime = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".png": "image/png", ".svg": "image/svg+xml" };
const server = createServer(async (request, response) => {
  try {
    const pathname = decodeURIComponent(new URL(request.url, "http://127.0.0.1").pathname);
    if (pathname === "/__layout_test__") {
      response.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
      response.end(fixture);
      return;
    }
    const file = resolve(distribution, `.${pathname === "/" ? "/index.html" : pathname}`);
    if (!file.startsWith(distribution + sep)) throw new Error("Invalid static asset path.");
    const contents = await readFile(file);
    response.writeHead(200, { "Content-Type": mime[extname(file)] ?? "application/octet-stream" });
    response.end(contents);
  } catch {
    response.writeHead(404);
    response.end();
  }
});
await new Promise(resolveListening => server.listen(0, "127.0.0.1", resolveListening));
const address = server.address();
if (!address || typeof address === "string") throw new Error("The layout server did not bind a TCP port.");
const origin = `http://127.0.0.1:${address.port}`;

async function connect(endpoint) {
  const socket = new WebSocket(endpoint);
  await new Promise((resolveOpen, rejectOpen) => {
    socket.addEventListener("open", resolveOpen, { once: true });
    socket.addEventListener("error", rejectOpen, { once: true });
  });
  let sequence = 0;
  const pending = new Map();
  socket.addEventListener("message", event => {
    const message = JSON.parse(String(event.data));
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    clearTimeout(request.timeout);
    if (message.error) request.reject(new Error(JSON.stringify(message.error)));
    else request.resolve(message.result);
  });
  socket.addEventListener("close", () => {
    for (const request of pending.values()) {
      clearTimeout(request.timeout);
      request.reject(new Error("The isolated browser connection closed."));
    }
    pending.clear();
  });
  return {
    send(method, params = {}, sessionId) {
      return new Promise((resolveCommand, rejectCommand) => {
        const id = ++sequence;
        const timeout = setTimeout(() => { pending.delete(id); rejectCommand(new Error(`Browser command timed out: ${method}`)); }, 5000);
        pending.set(id, { resolve: resolveCommand, reject: rejectCommand, timeout });
        socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
      });
    },
    close() { socket.close(); },
  };
}

async function launch(scale, route, screenshot, viewport, media = [], updateScene = false) {
  const profile = join(target, `m1-layout-browser-${token}-${scale}-${screenshot ? "image" : "tests"}`);
  await mkdir(profile, { recursive: true });
  let child;
  let connection;
  let closed;
  try {
    const endpoint = await new Promise((resolveEndpoint, rejectEndpoint) => {
      child = spawn(browser, [
        "--headless=new", "--no-first-run", "--no-default-browser-check", "--disable-background-networking", "--disable-extensions",
        "--disable-gpu", `--user-data-dir=${profile}`, `--force-device-scale-factor=${scale}`, "--window-size=1280,800",
        "--remote-debugging-address=127.0.0.1", "--remote-debugging-port=0", "about:blank",
      ], { windowsHide: true, stdio: ["ignore", "ignore", "pipe"] });
      closed = new Promise(resolveClose => child.once("close", resolveClose));
      let stderr = "";
      const timeout = setTimeout(() => rejectEndpoint(new Error("The isolated browser did not expose its test endpoint.")), 15_000);
      child.stderr.setEncoding("utf8").on("data", chunk => {
        stderr = (stderr + chunk).slice(-4096);
        const observed = stderr.match(/DevTools listening on (ws:\/\/127\.0\.0\.1:[^\s]+)/)?.[1];
        if (observed) { clearTimeout(timeout); resolveEndpoint(observed); }
      });
      child.once("error", error => { clearTimeout(timeout); rejectEndpoint(error); });
      child.once("close", code => {
        clearTimeout(timeout);
        rejectEndpoint(new Error(`The isolated browser exited ${code}: ${stderr}`));
      });
    });
    connection = await connect(endpoint);
    const { targetId } = await connection.send("Target.createTarget", { url: "about:blank" });
    const { sessionId } = await connection.send("Target.attachToTarget", { targetId, flatten: true });
    await connection.send("Page.enable", {}, sessionId);
    if (viewport) {
      await connection.send("Emulation.setDeviceMetricsOverride", { width: viewport[0], height: viewport[1], deviceScaleFactor: scale, mobile: false }, sessionId);
    }
    if (media.length) await connection.send("Emulation.setEmulatedMedia", { features: media }, sessionId);
    if (updateScene) {
      const contract = JSON.parse(await readFile(join(repository, "docs/agent/fixtures/M4-contract.json"), "utf8"));
      const responses = {
        vault_status: { exists: true, unlocked: true, revision: 0, epoch: 0 },
        vault_hello_status: { available: false, enabled: false, invalidated: false },
        vault_preferences: { auto_lock_minutes: 30, disconnect_on_lock: false },
        load_desktop_preferences: contract.settingsDefaults,
        update_status: { ...contract.updateStatus, available: true, phase: "available", release: { id: "11111111-1111-4111-8111-111111111111", version: "1.2.3", notes: "Capstan 1.2.3\n\nUpdate verification and recovery. 更新校验与恢复。\n<script>This remains plain text.</script>" } },
        quake_status: { enabled: false, errorCode: null },
        remote_edit_snapshot: { revision: "0", sessions: [], recoveryIncomplete: false },
        window_visibility: { visible: true },
        load_sftp_panel_preferences: { layout: { dock: "right", size: 36, collapsed: true }, session: "11111111-1111-4111-8111-111111111111", sequence: "0" },
        start_local_terminal: { kind: "local", id: "update-preview", runtime: { kind: "system" } },
      };
      await connection.send("Page.addScriptToEvaluateOnNewDocument", { source: `window.__TAURI__ = { event: {listen: async () => () => {}}, core: { Channel: class {onmessage = null;}, invoke: async (command) => {const responses = ${JSON.stringify(responses)}; return responses[command] ?? (command.startsWith('list_') || command.endsWith('_snapshots') || command === 'transfer_snapshot' ? [] : null);}}};` }, sessionId);
    }
    await connection.send("Page.navigate", { url: `${origin}${route}` }, sessionId);
    const expression = screenshot && !sftpMode
      ? '!!document.querySelector(".terminal-pane:not([hidden]) .xterm-screen")'
      : 'document.getElementById("result")?.dataset.report';
    const deadline = Date.now() + 30_000;
    let value;
    while (!value && Date.now() < deadline) {
      value = (await connection.send("Runtime.evaluate", { expression, returnByValue: true }, sessionId)).result?.value;
      if (!value) await new Promise(resolveTick => setTimeout(resolveTick, 100));
    }
    if (!value) throw new Error("The browser did not finish the layout checks.");
    if (screenshot) {
      if (updateScene) {
        const noticeDeadline = Date.now() + 5000;
        while (!(await connection.send("Runtime.evaluate", { expression: `!!document.querySelector('.update-notice button')`, returnByValue: true }, sessionId)).result?.value) {
          if (Date.now() > noticeDeadline) throw new Error("The update preview fixture did not initialize.");
          await new Promise(resolveTick => setTimeout(resolveTick, 50));
        }
        await connection.send("Runtime.evaluate", { expression: `document.querySelector('.update-notice button').click()` }, sessionId);
        await new Promise(resolveFrame => setTimeout(resolveFrame, 200));
        const inspected = await connection.send("Runtime.evaluate", { expression: `(() => {
          const dialog = document.querySelector('.update-dialog');
          const bounds = dialog.getBoundingClientRect();
          const focused = document.activeElement;
          const focus = getComputedStyle(focused);
          const errors = [];
          if (!dialog.contains(focused) || focused.classList.contains('primary-action')) errors.push('Initial focus must be on the safe action.');
          if (bounds.left < 0 || bounds.right > innerWidth || bounds.top < 0 || bounds.bottom > innerHeight) errors.push('Update dialog exceeds the viewport.');
          if (dialog.querySelector('.update-notes script')) errors.push('Release notes must remain text.');
          if (matchMedia('(prefers-reduced-motion: reduce)').matches && document.getAnimations().some(animation => animation.playState === 'running')) errors.push('Motion continues under reduced-motion.');
          if (matchMedia('(forced-colors: active)').matches && focus.outlineStyle === 'none') errors.push('Keyboard focus is not visible in forced colors.');
          return {passed: errors.length === 0, results: [{name: 'update-dialog-accessibility', passed: errors.length === 0, errors}]};
        })()`, returnByValue: true }, sessionId);
        if (inspected.exceptionDetails || !inspected.result?.value) throw new Error(`Update preview check failed: ${JSON.stringify(inspected.exceptionDetails)}`);
        const result = inspected.result.value;
        const preview = await connection.send("Page.captureScreenshot", { format: "png" }, sessionId);
        await writeFile(screenshot, Buffer.from(preview.data, "base64"));
        return result;
      }
      await new Promise(resolveFrame => setTimeout(resolveFrame, 100));
      const { data } = await connection.send("Page.captureScreenshot", { format: "png" }, sessionId);
      await writeFile(screenshot, Buffer.from(data, "base64"));
      if (sftpMode) return JSON.parse(decodeURIComponent(value));
      await connection.send("Runtime.evaluate", { expression: `window.__TAURI__ = {core:{invoke: async () => [{id:"powershell51",name:"Windows PowerShell",is_wsl:false},{id:"wsl:Ubuntu",name:"WSL: Ubuntu",is_wsl:true}]}}; document.querySelector('.local-launcher summary').click();` }, sessionId);
      await new Promise(resolveFrame => setTimeout(resolveFrame, 150));
      for (const [name, width, height] of [["shell-selector-desktop.png", 1280, 800], ["shell-selector-minimum.png", 330, 240]]) {
        await connection.send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: false }, sessionId);
        await new Promise(resolveFrame => setTimeout(resolveFrame, 150));
        const preview = await connection.send("Page.captureScreenshot", { format: "png" }, sessionId);
        await writeFile(join(dirname(screenshot), name), Buffer.from(preview.data, "base64"));
      }
      return;
    }
    return JSON.parse(decodeURIComponent(value));
  } finally {
    if (connection) {
      await connection.send("Browser.close").catch(() => undefined);
      connection.close();
    }
    if (child && child.exitCode === null) child.kill();
    if (closed) await closed;
    const profilePath = await realpath(profile);
    const targetPath = (await realpath(target)) + sep;
    if (!profilePath.toLowerCase().startsWith(targetPath.toLowerCase())) throw new Error("The browser profile cleanup path is outside target.");
    await rm(profilePath, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
}

const report = { schema: 1, browser, measured_at: new Date().toISOString(), passed: false, scales: [] };
try {
  if (sftpMode) {
    for (const [name, width, height, search] of [
      ["desktop", 1280, 800, ""], ["bottom-light-zh", 1000, 700, "?dock=bottom&theme=light&locale=zh-CN"],
      ["compact", 800, 600, ""], ["minimum-collapsed", 330, 240, "?collapsed=1"],
      ["conflict", 1280, 800, "?dialog=1"], ["conflict-zh", 800, 600, "?dialog=1&locale=zh-CN"],
      ["folder-conflict", 1280, 800, "?dialog=1&batch=1"],
      ["folder-conflict-light-zh", 800, 600, "?dialog=1&batch=1&theme=light&locale=zh-CN"],
      ["folder-conflict-minimum", 330, 240, "?dialog=1&batch=1&collapsed=1"],
      ["tree-plan", 1280, 800, "?tree-plan=1"],
      ["tree-plan-light-zh", 800, 600, "?tree-plan=1&theme=light&locale=zh-CN"],
      ["tree-plan-minimum", 330, 240, "?tree-plan=1&collapsed=1"],
      ["native-export", 1280, 800, "?native-export=1"],
      ["native-export-minimum-zh", 330, 240, "?native-export=1&collapsed=1&theme=light&locale=zh-CN"],
      ["delete-tree", 1280, 800, "?delete-tree=1"],
      ["delete-tree-result-minimum-zh", 330, 240, "?delete-tree=1&delete-result=1&collapsed=1&theme=light&locale=zh-CN"],
      ["manual-recovery", 1280, 800, "?manual-recovery=1"],
      ["manual-recovery-minimum-zh", 330, 240, "?manual-recovery=1&collapsed=1&theme=light&locale=zh-CN"],
      ["cleanup-recovery", 1280, 800, "?cleanup-recovery=1"],
      ["cleanup-recovery-minimum-zh", 330, 240, "?cleanup-recovery=1&collapsed=1&theme=light&locale=zh-CN"],
      ["upload-recovery", 1280, 800, "?upload-recovery=1"],
      ["upload-recovery-minimum-zh", 330, 240, "?upload-recovery=1&collapsed=1&theme=light&locale=zh-CN"],
      ["cwd-follow", 1280, 800, "?cwd=1"], ["cwd-aliases-light-zh", 1000, 700, "?dock=bottom&theme=light&locale=zh-CN&aliases=1"],
      ["cwd-alias-error", 1280, 800, "?alias-error=1"], ["cwd-alias-error-zh", 1000, 700, "?theme=light&locale=zh-CN&alias-error=1"],
      ["preferences-save", 1280, 800, "?preferences-save=1"], ["preferences-load-zh", 1000, 700, "?dock=bottom&theme=light&locale=zh-CN&preferences-load=1"],
      ["preferences-collapsed", 330, 240, "?collapsed=1&preferences-save=1"], ["preferences-exit", 1000, 700, "?preferences-exit=1"],
      ["preferences-exit-zh", 1000, 700, "?theme=light&locale=zh-CN&preferences-exit=1"],
      ["editor-conflict", 1280, 800, "?editor=1&editor-dialog=1"],
      ["editor-conflict-light-zh", 800, 600, "?editor=1&editor-dialog=1&theme=light&locale=zh-CN"],
      ["editor-recovery-minimum", 330, 240, "?editor=1&collapsed=1"],
      ["editor-cleanup", 1280, 800, "?editor=1&editor-cleanup=1"],
      ["editor-cleanup-light-zh", 800, 600, "?editor=1&editor-cleanup=1&theme=light&locale=zh-CN"],
      ["editor-retain", 800, 600, "?editor=1&editor-end-dialog=1"],
      ["editor-corrupt-retain-zh", 800, 600, "?editor=1&editor-corrupt=1&editor-end-dialog=1&theme=light&locale=zh-CN"],
      ["forced-colors-reduced-motion", 1000, 700, "?dock=bottom&theme=light&locale=zh-CN"],
    ]) {
      const media = name.startsWith("forced-colors") ? [{ name: "forced-colors", value: "active" }, { name: "prefers-reduced-motion", value: "reduce" }] : [];
      const result = await launch(1, `/tests/sftp-workflow.html${search}`, join(evidence, `${name}.png`), [width, height], media);
      report.scales.push({ scale: 1, scene: name, ...result });
    }
  } else for (const scale of securityMode ? [1] : [1, 1.25]) {
    const result = await launch(scale, securityMode ? "/tests/terminal-security.html" : "/__layout_test__");
    report.scales.push({ scale, ...result });
  }
  if (!securityMode && !sftpMode) {
    const media = [{ name: "forced-colors", value: "active" }, { name: "prefers-reduced-motion", value: "reduce" }];
    report.scales.push({ scale: 1, scene: "forced-colors-reduced-motion", ...await launch(1, "/__layout_test__", undefined, undefined, media) });
    for (const [scene, width, height, features] of [["update-dialog", 1280, 800, []], ["update-dialog-accessible", 800, 600, media]]) {
      report.scales.push({ scale: 1, scene, ...await launch(1, "/", join(evidence, `${scene}.png`), [width, height], features, true) });
    }
  }
  report.passed = report.scales.every(result => result.passed);
  if (report.passed && !securityMode && !sftpMode) await launch(1, "/", join(evidence, "browser-preview.png"));
} catch (error) {
  report.error = String(error);
} finally {
  await new Promise(resolveClosed => server.close(resolveClosed));
  await writeFile(join(evidence, "report.json"), JSON.stringify(report, null, 2) + "\n");
}
for (const scale of report.scales) {
  console.log(`Scale ${scale.scale}: ${scale.results.filter(result => result.passed).length}/${scale.results.length} ${securityMode ? "security" : "layout"} checks passed.`);
  for (const result of scale.results.filter(result => !result.passed)) console.error(`${result.name}: ${result.errors.join(" ")}`);
}
console.log(`Evidence directory: ${evidence}`);
if (!report.passed || report.error) {
  if (report.error) console.error(report.error);
  process.exitCode = 1;
}
