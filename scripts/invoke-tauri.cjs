const { spawnSync } = require("node:child_process");
const path = require("node:path");

const cwd = path.resolve(__dirname, "..");
const args = process.argv.slice(2);
const windows = process.platform === "win32";
const executable = windows ? "powershell" : path.join(cwd, "ui/node_modules/.bin/tauri");
const forwarded = windows
  ? ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "scripts/Invoke-Tauri.ps1", ...args]
  : args;
const result = spawnSync(executable, forwarded, { cwd, stdio: "inherit" });
if (result.error) {
  console.error(result.error.code === "ENOENT" && !windows
    ? "Install the frozen UI dependencies before invoking Tauri CLI."
    : result.error.message);
}
if (result.signal) process.kill(process.pid, result.signal);
process.exitCode = result.status ?? 1;
