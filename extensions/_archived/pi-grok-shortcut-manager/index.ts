/**
 * pi-grok-shortcut-manager
 *
 * Bridges Pi extension shortcuts into grok-pi's Remote TUI key dispatch path.
 *
 * Problem: Pi extensions register shortcuts via `pi.registerShortcut("alt+t", ...)`.
 * In native Pi TUI, these are dispatched by CustomEditor.onExtensionShortcut.
 * In grok-pi RPC mode, keys flow through remote-tui's handleInput → focused component,
 * completely bypassing extension shortcut dispatch.
 *
 * Solution: Patch ExtensionRunner.prototype.setUIContext to capture the runner
 * instance, then call getShortcuts() to build a dispatch table. The remote-tui
 * handleInput calls __piGrokShortcutIntercept before dispatching to the focused
 * component.
 *
 * Scope: ONLY manages shortcuts registered by Pi extensions (pi.registerShortcut).
 * Does NOT touch grok-pi/Pager built-in keybindings.
 *
 * Config: ~/.pi/shortcut-manager.json
 */

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { realpathSync } from "node:fs";
import { pathToFileURL } from "node:url";
import type { ExtensionAPI, ExtensionCommandContext, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { matchesKey, type KeyId } from "@earendil-works/pi-tui";

// ============================================================================
// Types
// ============================================================================

interface ShortcutEntry {
  key: string;
  description: string;
  extensionPath: string;
  enabled: boolean;
  remappedTo?: string;
}

interface ShortcutManagerConfig {
  version: 1;
  shortcuts: Record<string, ShortcutEntry>;
  globalEnabled: boolean;
}

interface RegisteredShortcut {
  key: string;
  description: string;
  extensionPath: string;
  handler: (ctx: ExtensionContext) => Promise<void> | void;
}

// ============================================================================
// Config persistence (cached — avoids disk read on every keypress)
// ============================================================================

const CONFIG_DIR = join(homedir(), ".pi");
const CONFIG_PATH = join(CONFIG_DIR, "shortcut-manager.json");

let configCache: ShortcutManagerConfig | null = null;

function loadConfig(): ShortcutManagerConfig {
  if (configCache) return configCache;
  try {
    if (existsSync(CONFIG_PATH)) {
      const raw = readFileSync(CONFIG_PATH, "utf8");
      configCache = JSON.parse(raw) as ShortcutManagerConfig;
      return configCache;
    }
  } catch { /* ignore corrupt config */ }
  configCache = { version: 1, shortcuts: {}, globalEnabled: true };
  return configCache;
}

function saveConfig(config: ShortcutManagerConfig): void {
  configCache = config;
  try {
    if (!existsSync(CONFIG_DIR)) mkdirSync(CONFIG_DIR, { recursive: true });
    writeFileSync(CONFIG_PATH, JSON.stringify(config, null, 2), "utf8");
  } catch { /* best effort */ }
}

// ============================================================================
// Global shortcut registry
// ============================================================================

const shortcutRegistry: Map<string, RegisteredShortcut> = new Map();
let latestCtx: ExtensionContext | null = null;
let runnerInstance: unknown = null;

// ============================================================================
// Shortcut dispatch
// ============================================================================

function isShortcutEnabled(key: string): boolean {
  const config = loadConfig();
  if (!config.globalEnabled) return false;
  const entry = config.shortcuts[key.toLowerCase()];
  if (entry && !entry.enabled) return false;
  return true;
}

function getEffectiveKey(key: string): string {
  const config = loadConfig();
  const entry = config.shortcuts[key.toLowerCase()];
  return entry?.remappedTo ?? key;
}

function dispatchShortcut(data: string): boolean {
  if (!latestCtx) return false;
  const config = loadConfig();
  if (!config.globalEnabled) return false;

  for (const [key, shortcut] of shortcutRegistry) {
    if (!isShortcutEnabled(key)) continue;
    const effectiveKey = getEffectiveKey(key);
    if (matchesKey(data, effectiveKey as KeyId)) {
      try {
        Promise.resolve(shortcut.handler(latestCtx)).catch((err) => {
          console.error(`[shortcut-manager] Handler error for '${key}':`, err);
        });
      } catch (err) {
        console.error(`[shortcut-manager] Sync handler error for '${key}':`, err);
      }
      return true;
    }
  }
  return false;
}

// ============================================================================
// Capture runner instance via setUIContext patch (same pattern as rpc-compat)
// ============================================================================

function hostUrl(relativePath: string): string {
  const hostDistDir = dirname(realpathSync(process.argv[1]!));
  return new URL(relativePath, pathToFileURL(`${hostDistDir}/`)).href;
}

type ExtensionRunnerLike = {
  setUIContext: (uiContext: unknown, mode?: string) => void;
  getShortcuts: (resolvedKeybindings: unknown) => Map<string, {
    shortcut: string;
    description?: string;
    handler: (ctx: ExtensionContext) => Promise<void> | void;
    extensionPath: string;
  }>;
};

type ExtensionRunnerConstructor = {
  prototype: ExtensionRunnerLike & { __piGrokShortcutManagerPatched?: boolean };
};

async function installRunnerCapture(): Promise<void> {
  try {
    const module = (await import(hostUrl("core/extensions/runner.js"))) as {
      ExtensionRunner?: ExtensionRunnerConstructor;
    };
    const prototype = module.ExtensionRunner?.prototype;
    if (!prototype) return;
    if (prototype.__piGrokShortcutManagerPatched) return;

    const original = prototype.setUIContext;
    prototype.setUIContext = function setUIContext(this: ExtensionRunnerLike, uiContext: unknown, mode?: string): void {
      original.call(this, uiContext, mode);
      // Capture runner instance and populate registry from getShortcuts()
      if (!runnerInstance) {
        runnerInstance = this;
        refreshRegistry();
      }
    };
    prototype.__piGrokShortcutManagerPatched = true;
  } catch {
    // Runner not available
  }
}

function refreshRegistry(): void {
  if (!runnerInstance) return;
  const runner = runnerInstance as ExtensionRunnerLike;
  try {
    const shortcuts = runner.getShortcuts({});
    shortcutRegistry.clear();
    for (const [key, shortcut] of shortcuts) {
      const normalizedKey = key.toLowerCase();
      shortcutRegistry.set(normalizedKey, {
        key,
        description: shortcut.description ?? shortcut.extensionPath,
        extensionPath: shortcut.extensionPath,
        handler: shortcut.handler,
      });
    }
  } catch {
    // getShortcuts may fail if keybindings config is not ready
  }
}

// ============================================================================
// Install global intercept for remote-tui handleInput
// ============================================================================

function installGlobalIntercept(): void {
  const g = globalThis as typeof globalThis & {
    __piGrokShortcutIntercept?: (data: string) => boolean;
  };
  g.__piGrokShortcutIntercept = dispatchShortcut;
}

// ============================================================================
// Diagnostics (extension-to-extension conflicts only)
// ============================================================================

interface ShortcutDiagnostic {
  key: string;
  extensionPath: string;
  description: string;
  conflictType: "duplicate" | "none";
  conflictWith?: string;
  enabled: boolean;
  remappedTo?: string;
}

function buildDiagnostics(): ShortcutDiagnostic[] {
  const config = loadConfig();
  const diagnostics: ShortcutDiagnostic[] = [];
  const seenKeys = new Map<string, string>();

  for (const [key, shortcut] of shortcutRegistry) {
    const normalizedKey = key.toLowerCase();
    let conflictType: ShortcutDiagnostic["conflictType"] = "none";
    let conflictWith: string | undefined;

    if (seenKeys.has(normalizedKey)) {
      conflictType = "duplicate";
      conflictWith = seenKeys.get(normalizedKey);
    }
    seenKeys.set(normalizedKey, shortcut.extensionPath);

    const entry = config.shortcuts[normalizedKey];
    diagnostics.push({
      key,
      extensionPath: shortcut.extensionPath,
      description: shortcut.description,
      conflictType,
      conflictWith,
      enabled: entry ? entry.enabled : true,
      remappedTo: entry?.remappedTo,
    });
  }

  return diagnostics;
}

// ============================================================================
// Formatting helpers
// ============================================================================

function shortExtName(extPath: string): string {
  // "pi-language-tutor" from "/Users/x/.pi/agent/npm/node_modules/pi-language-tutor/src/index.ts"
  const parts = extPath.split("/");
  const nmIdx = parts.lastIndexOf("node_modules");
  if (nmIdx >= 0 && nmIdx + 1 < parts.length) return parts[nmIdx + 1]!;
  // Fallback: last directory or filename
  return parts[parts.length - 2] ?? parts[parts.length - 1] ?? extPath;
}

function formatKeyDisplay(key: string): string {
  return key.toUpperCase().replace(/\+/g, "+");
}

// ============================================================================
// Extension entry point
// ============================================================================

export default function (pi: ExtensionAPI): void {
  void installRunnerCapture();
  installGlobalIntercept();

  // Keep ctx fresh for handler dispatch
  pi.on("session_start", (_event, ctx) => {
    latestCtx = ctx;
    refreshRegistry();
  });

  pi.on("turn_start", (_event, ctx) => {
    latestCtx = ctx;
  });

  // ========================================================================
  // /shortcuts command
  // ========================================================================

  pi.registerCommand("shortcuts", {
    description: "Manage extension shortcuts: /shortcuts [list|enable|disable|remap|diagnostics|on|off]",
    handler: async (args: string, ctx: ExtensionCommandContext) => {
      const parts = args.trim().split(/\s+/);
      const sub = parts[0] ?? "list";
      const key = parts[1];
      const newKey = parts[2];
      const config = loadConfig();

      switch (sub) {
        case "list": {
          refreshRegistry();
          const diagnostics = buildDiagnostics();
          if (diagnostics.length === 0) {
            ctx.ui.notify("No extension shortcuts registered.\nInstall extensions that call pi.registerShortcut() to see them here.", "info");
            return;
          }

          // Group by extension
          const byExt = new Map<string, ShortcutDiagnostic[]>();
          for (const d of diagnostics) {
            const name = shortExtName(d.extensionPath);
            if (!byExt.has(name)) byExt.set(name, []);
            byExt.get(name)!.push(d);
          }

          const sections: string[] = [];
          for (const [extName, items] of byExt) {
            const lines = items.map((d) => {
              const icon = d.enabled ? "●" : "○";
              const keyStr = formatKeyDisplay(d.key);
              const remap = d.remappedTo ? ` → ${formatKeyDisplay(d.remappedTo)}` : "";
              const conflict = d.conflictType !== "none" ? " ⚠" : "";
              return `  ${icon} ${keyStr}${remap}  ${d.description}${conflict}`;
            });
            sections.push(`${extName}\n${lines.join("\n")}`);
          }

          const globalStatus = config.globalEnabled ? "" : "\n⚠ Dispatch globally disabled (/shortcuts on)";
          ctx.ui.notify(
            `Extension shortcuts (${diagnostics.length})${globalStatus}\n\n${sections.join("\n\n")}`,
            "info",
          );
          return;
        }

        case "enable": {
          if (!key) { ctx.ui.notify("Usage: /shortcuts enable <key>\nExample: /shortcuts enable alt+t", "warning"); return; }
          const nk = key.toLowerCase();
          if (!shortcutRegistry.has(nk)) {
            ctx.ui.notify(`Unknown shortcut '${key}'. Run /shortcuts list to see registered shortcuts.`, "warning");
            return;
          }
          const existing = config.shortcuts[nk];
          config.shortcuts[nk] = {
            key: nk,
            description: existing?.description ?? shortcutRegistry.get(nk)?.description ?? "",
            extensionPath: existing?.extensionPath ?? shortcutRegistry.get(nk)?.extensionPath ?? "",
            enabled: true,
            remappedTo: existing?.remappedTo,
          };
          saveConfig(config);
          ctx.ui.notify(`● ${formatKeyDisplay(key)} enabled`, "success");
          return;
        }

        case "disable": {
          if (!key) { ctx.ui.notify("Usage: /shortcuts disable <key>\nExample: /shortcuts disable alt+t", "warning"); return; }
          const nk = key.toLowerCase();
          if (!shortcutRegistry.has(nk)) {
            ctx.ui.notify(`Unknown shortcut '${key}'. Run /shortcuts list to see registered shortcuts.`, "warning");
            return;
          }
          const existing = config.shortcuts[nk];
          config.shortcuts[nk] = {
            key: nk,
            description: existing?.description ?? shortcutRegistry.get(nk)?.description ?? "",
            extensionPath: existing?.extensionPath ?? shortcutRegistry.get(nk)?.extensionPath ?? "",
            enabled: false,
            remappedTo: existing?.remappedTo,
          };
          saveConfig(config);
          ctx.ui.notify(`○ ${formatKeyDisplay(key)} disabled`, "info");
          return;
        }

        case "remap": {
          if (!key || !newKey) {
            ctx.ui.notify("Usage: /shortcuts remap <old-key> <new-key>\nExample: /shortcuts remap alt+t alt+shift+t", "warning");
            return;
          }
          const nk = key.toLowerCase();
          if (!shortcutRegistry.has(nk)) {
            ctx.ui.notify(`Unknown shortcut '${key}'. Run /shortcuts list to see registered shortcuts.`, "warning");
            return;
          }
          // Check if new key conflicts with another extension shortcut
          const conflictTarget = [...shortcutRegistry.entries()].find(
            ([k]) => k === newKey.toLowerCase() && k !== nk,
          );
          if (conflictTarget) {
            ctx.ui.notify(
              `⚠ '${newKey}' is already used by ${shortExtName(conflictTarget[1].extensionPath)}. ` +
              `Remap anyway? Use /shortcuts remap ${key} ${newKey}! to force.`,
              "warning",
            );
            if (!parts.includes("!")) return;
          }
          const existing = config.shortcuts[nk];
          config.shortcuts[nk] = {
            key: nk,
            description: existing?.description ?? shortcutRegistry.get(nk)?.description ?? "",
            extensionPath: existing?.extensionPath ?? shortcutRegistry.get(nk)?.extensionPath ?? "",
            enabled: existing?.enabled ?? true,
            remappedTo: newKey.toLowerCase(),
          };
          saveConfig(config);
          ctx.ui.notify(`${formatKeyDisplay(key)} → ${formatKeyDisplay(newKey)}`, "success");
          return;
        }

        case "reset": {
          if (!key) { ctx.ui.notify("Usage: /shortcuts reset <key>\nRemoves remap and re-enables the shortcut.", "warning"); return; }
          const nk = key.toLowerCase();
          delete config.shortcuts[nk];
          saveConfig(config);
          ctx.ui.notify(`${formatKeyDisplay(key)} reset to default`, "success");
          return;
        }

        case "diagnostics": {
          refreshRegistry();
          const diagnostics = buildDiagnostics();
          const conflicts = diagnostics.filter((d) => d.conflictType !== "none");
          const disabled = diagnostics.filter((d) => !d.enabled);

          if (conflicts.length === 0 && disabled.length === 0) {
            ctx.ui.notify(`All ${diagnostics.length} extension shortcuts active, no conflicts.`, "success");
            return;
          }

          const lines: string[] = [];
          if (conflicts.length > 0) {
            lines.push("Conflicts:");
            for (const d of conflicts) {
              lines.push(`  ⚠ ${formatKeyDisplay(d.key)} — ${shortExtName(d.extensionPath)} conflicts with ${shortExtName(d.conflictWith ?? "")}`);
            }
          }
          if (disabled.length > 0) {
            lines.push("Disabled:");
            for (const d of disabled) {
              lines.push(`  ○ ${formatKeyDisplay(d.key)} — ${d.description}`);
            }
          }
          ctx.ui.notify(lines.join("\n"), "warning");
          return;
        }

        case "on":
          config.globalEnabled = true;
          saveConfig(config);
          ctx.ui.notify("Extension shortcut dispatch enabled", "success");
          return;

        case "off":
          config.globalEnabled = false;
          saveConfig(config);
          ctx.ui.notify("Extension shortcut dispatch disabled.\nAll extension shortcuts are inactive. Use /shortcuts on to re-enable.", "info");
          return;

        default:
          ctx.ui.notify(
            "Usage: /shortcuts <command>\n\n" +
            "  list          Show all extension shortcuts\n" +
            "  enable <key>  Enable a shortcut\n" +
            "  disable <key> Disable a shortcut\n" +
            "  remap <old> <new>  Remap a shortcut\n" +
            "  reset <key>   Remove remap, re-enable\n" +
            "  diagnostics   Show conflicts and issues\n" +
            "  on / off      Global enable/disable",
            "info",
          );
      }
    },
  });
}
