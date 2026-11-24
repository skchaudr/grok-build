// Pure helpers behind dpi-rpc's settings, info, hints and shortcut keys: no I/O, so each is tested on its own.
// The shim (dpi-rpc.mjs) fetches the backend data, calls these, and sends the result.

export const THINKING_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh"];

const MODEL_ID = (m) => `${m.provider}/${m.id}`;
const kTokens = (n) => `${Math.round(n / 1000)}k`;

// ---- options (the F2 settings modal) --------------------------------------------------------------------------------

// Auto-compaction is shown as one number: the share of the model's window where a request waits for a summary.
// The backend stores it as reserveTokens (window - wait) beside a background lead and an on/off flag.
export const COMPACTION_PERCENTS = [50, 60, 70, 80, 90];
const RESERVE_MIN = 1024; // the backend's own bounds for reserveTokens
const RESERVE_MAX = 1_000_000;
const MIN_ROOM = 32_768; // below this the model has no room left before every request waits for a summary

export const compactionPercent = (window, c) => Math.round(((window - c.reserveTokens) / window) * 100);

export function compactionReserveFor(window, percent) {
  return window - Math.round((window * percent) / 100);
}

// Percentages the backend would accept for this window.
const usablePercent = (window, p) => {
  const reserve = compactionReserveFor(window, p);
  return reserve >= RESERVE_MIN && reserve <= RESERVE_MAX && window - reserve >= MIN_ROOM;
};

/** PATCH /compaction body for the picked value ("off" or a percent), or throws a message for the user. */
export function compactionPatch(window, value) {
  if (value === "off") return { enabled: false };
  const p = Number(value);
  if (!Number.isInteger(p) || p < 1 || p > 99) throw new Error(`"${value}" is not a compaction setting (use off or a percent from 1 to 99)`);
  if (!(window > 0)) throw new Error("this model's context window is unknown, so a percentage cannot be turned into tokens");
  if (!usablePercent(window, p)) throw new Error(`${p}% of this model's ${kTokens(window)} window leaves no room for the backend; pick another value`);
  return { enabled: true, reserveTokens: compactionReserveFor(window, p) };
}

/**
 * The snapshot's `options`, from live backend data:
 *   agent: GET agent, device: GET /device, ext: GET extensions, compaction: GET session compaction.
 * Option ids are stable: model, thinking, read_only, prompt:<promptId>, ext:<name>, compaction.
 */
export function buildOptions({ agent, device, ext, compaction }) {
  const cfg = agent.config ?? {};
  const models = device.models ?? [];
  const current = cfg.model ? MODEL_ID({ provider: cfg.model.provider, id: cfg.model.modelId }) : undefined;
  const modelValues = models.map((m) => ({
    value: MODEL_ID(m),
    name: m.id,
    description: [m.provider, m.contextWindow ? `${kTokens(m.contextWindow)} context` : null].filter(Boolean).join(" · "),
  }));
  if (current && !modelValues.some((v) => v.value === current)) modelValues.push({ value: current, name: cfg.model.modelId, description: cfg.model.provider });
  const known = models.find((m) => current === MODEL_ID(m));

  const options = [
    { id: "model", name: "Model", description: "Used from this agent's next turn", category: "Agent", type: "select", currentValue: current, values: modelValues },
    {
      id: "thinking", name: "Thinking",
      description: known?.reasoning === false ? "This model does not reason, so this has no effect" : "How hard the model thinks before it answers",
      category: "Agent", type: "select", currentValue: cfg.thinkingLevel ?? "medium",
      values: THINKING_LEVELS.map((value) => ({ value, name: value })),
    },
    { id: "read_only", name: "Read-only", description: "Blocks write tools (edit, write, bash)", category: "Agent", type: "boolean", currentValue: !!cfg.readOnly },
  ];

  const system = (ext.extensions ?? []).find((e) => e.name === "system-prompts");
  for (const p of system?.prompts ?? []) {
    options.push({
      id: `prompt:${p.id}`, name: p.title, description: p.description || "Added to this agent's system prompt",
      category: "System prompts", type: "boolean", currentValue: !!(system.enabled && p.enabled),
    });
  }
  for (const e of ext.extensions ?? []) {
    options.push({ id: `ext:${e.name}`, name: e.title || e.name, ...(e.description ? { description: e.description } : {}), category: "Extensions", type: "boolean", currentValue: !!e.enabled });
  }

  const window = known?.contextWindow;
  const percents = window > 0 ? COMPACTION_PERCENTS.filter((p) => usablePercent(window, p)) : [];
  if (percents.length && compaction) { // a window too small for any percentage has no honest choices to offer
    const at = compaction.enabled ? compactionPercent(window, compaction) : null;
    if (at !== null && !percents.includes(at)) percents.push(at);
    options.push({
      id: "compaction", name: "Auto-compact at",
      description: "Summarise older context when it reaches this share of the model's window; shared by the session's agents",
      category: "Context", type: "select", currentValue: at === null ? "off" : String(at),
      values: [{ value: "off", name: "Off" }, ...percents.sort((a, b) => a - b).map((p) => ({ value: String(p), name: `${p}%` }))],
    });
  }
  return options;
}

// ---- info ---------------------------------------------------------------------------------------------------------

/** Tokens in the model's context after the last assistant turn that reported usage, or null when none has. */
export function usageTokens(messages) {
  const u = [...messages].reverse().find((m) => m.role === "assistant" && m.usage)?.usage;
  return u ? (u.input ?? 0) + (u.output ?? 0) + (u.cacheRead ?? 0) + (u.cacheWrite ?? 0) : null;
}

export function contextInfo(tokens, window) {
  if (tokens === null || !(window > 0)) return undefined;
  return { tokens, window, percent: Math.round((tokens / window) * 100) };
}

// ---- keys ---------------------------------------------------------------------------------------------------------

// Extension shortcuts the pager hands back as `/__pi_shortcut_dispatch <key>`; Pi KeyId format, lowercase, like
// pi-grok-shortcut-manager publishes ("alt+t"). Chosen against crates/codegen/xai-grok-pager/src/actions/defaults.rs:
// every Ctrl+letter we could want is taken (g k j u d e f c t r b o m s n p x l w q i), Alt+s is the prompt stash and
// F2/F4 are settings and acpx, so these are Alt+letters. Left out on purpose: b, f, d (word motion in text fields),
// and e, i, n, u (macOS Option dead keys: the terminal never reports them as Alt+letter).
// `label` is the short word the F2 footer shows beside the key ("Alt+C compact").
// Letters name the action: Compact, new Agent, fork (Keep a copy to experiment on), stop (X), Open playground, Help.
// Reset and archive have no key: they destroy, so they stay behind a typed command and a confirm.
export const SHORTCUTS = [
  { key: "alt+c", command: "dpi-compact", label: "compact", description: "Compact this agent's context" },
  { key: "alt+a", command: "dpi-new-agent", label: "new agent", description: "New agent in this session" },
  { key: "alt+k", command: "dpi-fork", label: "fork", description: "Fork this agent to try something" },
  { key: "alt+x", command: "dpi-stop", label: "stop", description: "Stop the turn and background tasks" },
  { key: "alt+o", command: "dpi-open", label: "playground", description: "Show the playground URL" },
  { key: "alt+h", command: "dpi-help", label: "help", description: "Durable Pi help" },
];

export const shortcutCatalog = () => ({ shortcuts: SHORTCUTS.map(({ key, label, description }) => ({ key, label, description, extension: "dpi", enabled: true })) });
export const commandForKey = (key) => SHORTCUTS.find((s) => s.key === String(key).trim().toLowerCase())?.command;
export const keyLabel = (key) => key.replace(/^alt\+/, "Alt+").replace(/^(\w)/, (c) => c.toUpperCase()).replace(/\+(\w)$/, (_, c) => `+${c.toUpperCase()}`);

// ---- playground URL -----------------------------------------------------------------------------------------------

const clean = (u) => (typeof u === "string" && u.trim() ? u.trim().replace(/\/$/, "") : undefined);
const origin = (u) => { try { return new URL(u).origin; } catch { return undefined; } };

/**
 * Where the browser playground (hub) is: DPI_HUB_URL from the environment, else the one in client.env when that file
 * is for this backend, else port 4781 (DPI_HUB_PORT overrides it) on the backend's host if `probe(url)` says something
 * answers. The Prime backend (port 4782) has no hub. null when none. The playground has no deep links to a session, so
 * this is its base URL.
 */
export async function resolvePlaygroundUrl({ env, clientEnv, base, probe }) {
  const fromEnv = clean(env.DPI_HUB_URL);
  if (fromEnv) return fromEnv;
  const u = new URL(base);
  if (u.port === "4782") return null;
  const fromFile = clean(clientEnv.DPI_HUB_URL);
  if (fromFile && (!clientEnv.DPI_URL || origin(clientEnv.DPI_URL) === u.origin)) return fromFile;
  const guess = `${u.protocol}//${u.hostname}:${clean(env.DPI_HUB_PORT) ?? "4781"}`;
  return (await probe(guess)) ? guess : null;
}

// ---- words --------------------------------------------------------------------------------------------------------

export function hintsFor({ device, session, playgroundUrl }) {
  const keys = SHORTCUTS.map((s) => `${keyLabel(s.key)} ${s.label}`).join(", ");
  return [
    `Quitting only detaches: the agent keeps working on ${device}. dpi-grok ${session} brings you back to where it got to.`,
    "ctrl+r switches between sessions and agents; /dpi-sessions lists what is on this device.",
    "F2 opens the settings: model, thinking, read-only, system prompts, extensions and auto-compaction, all live.",
    `Action keys: ${keys}.`,
    "/dpi-help has the short guide, /dpi-tasks shows what is running, /dpi-settings sums up the current setup.",
    "Deploys that need a human wait for approval: a dialog opens here, and /dpi-deploys brings it back if you dismissed it.",
    playgroundUrl ? `The playground at ${playgroundUrl} shows this agent's full history and tasks in a browser; /dpi-open prints the link.` : "This backend has no playground, so the terminal is the whole story.",
    "Fork before a risky experiment (/dpi-fork): the copy carries the conversation and the original stays untouched.",
    "Read-only blocks edit, write and bash: switch it on when you only want answers.",
    "Old context is summarised, never deleted. /dpi-compact does it now; /dpi-reset starts a fresh context.",
  ];
}

// The /dpi-help card: the pager draws it as a block with real headings and key names (`pi/ui/dpi_help`), not a
// grey notice.
export function helpCard({ device, session, playgroundUrl, commands }) {
  return {
    intro: `Durable Pi: your agent lives on ${device}, not in this window. Close the window and it keeps working; ctrl+r or dpi-grok ${session} brings you back.`,
    sections: [
      {
        title: "Keys",
        rows: [
          ["F2", "Settings: model, thinking, read-only, prompts, extensions, auto-compact"],
          ["ctrl+r", "Switch session or agent"],
          ...SHORTCUTS.map((s) => [keyLabel(s.key), s.description]),
        ],
      },
      { title: "Commands", rows: commands.map(([label, description]) => [`/${label}`, description]) },
    ],
    playgroundUrl: playgroundUrl ?? null,
  };
}
