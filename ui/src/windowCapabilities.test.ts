import { describe, expect, it } from "vitest";

import capability from "../../src-tauri/capabilities/default.json";
import aclManifests from "../../src-tauri/gen/schemas/acl-manifests.json";
import config from "../../src-tauri/tauri.conf.json";

type Permission = {
  permissions?: string[];
  commands?: { allow: string[]; deny: string[] };
};

type Manifest = {
  default_permission?: Permission;
  permissions: Record<string, Permission>;
  permission_sets: Record<string, Permission>;
};

const manifests: Record<string, Manifest> = aclManifests;

function grantedCommands(windowLabel: string): Set<string> {
  const commands = new Set<string>();
  if (!capability.windows.includes(windowLabel)) {
    return commands;
  }

  const resolve = (identifier: string): void => {
    const separator = identifier.lastIndexOf(":");
    const plugin = identifier.slice(0, separator);
    const name = identifier.slice(separator + 1);
    const manifest = manifests[plugin];
    const permission = name === "default"
      ? manifest?.default_permission
      : manifest?.permissions[name] ?? manifest?.permission_sets[name];
    if (!permission) {
      throw new Error(`Unknown Tauri permission: ${identifier}`);
    }
    for (const child of permission.permissions ?? []) {
      resolve(child.includes(":") ? child : `${plugin}:${child}`);
    }
    for (const command of permission.commands?.allow ?? []) {
      const pluginName = plugin.startsWith("core:") ? plugin.slice(5) : plugin;
      commands.add(`plugin:${pluginName}|${command}`);
    }
  };

  capability.permissions.forEach(resolve);
  return commands;
}

describe("titlebar window capabilities", () => {
  it("allows native dragging and double-click maximize in the main window", () => {
    const commands = grantedCommands("main");
    expect(commands.has("plugin:window|start_dragging")).toBe(true);
    expect(commands.has("plugin:window|internal_toggle_maximize")).toBe(true);
    expect(commands.has("plugin:snap-layout|update_snap_bounds")).toBe(true);
    expect(commands.has("plugin:snap-layout|detach_snap_bounds")).toBe(true);
  });

  it("does not grant titlebar commands to other windows", () => {
    expect(grantedCommands("secondary").size).toBe(0);
  });

  it("permits internal binary IPC without granting general remote connections", () => {
    const connect = config.app.security.csp.split(";").find((directive) => directive.trim().startsWith("connect-src"));
    expect(connect?.trim().split(/\s+/).slice(1)).toEqual([
      "'self'", "ipc:", "http://ipc.localhost",
    ]);
    expect(config.app.security.csp).not.toContain("127.0.0.1");
    expect(config.app.security.devCsp).toContain("http://127.0.0.1:1420");
    expect(config.app.security.devCsp).toContain("ws://127.0.0.1:1420");
  });
});
