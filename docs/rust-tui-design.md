# Rust TUI Design: Replacing TS remote-tui

## Current Architecture

```
Terminal → crossterm Event → AppView::handle_input()
  ├─ remote_tui_id active? → Effect::RemoteTuiInput { id, data }
  │                            → keyfile JSONL → TS remote-tui handleInput
  │                            → component.render() → setWidget RPC → Rust projection
  └─ normal → ActionRegistry.lookup() → prompt editor / scrollback
```

The TS remote-tui acts as a **component execution host**: Pi sends a factory
function, TS executes it in-process, renders frames, and sends pre-rendered
text lines back to Rust via `pi/ui/widget` RPC notifications.

## Key Insight

Rust already handles the **projection** side perfectly:
- `set_external_widget(key, lines, placement)` renders text lines above/below editor
- `apply_remote_tui(op, id, lines, title)` manages open/frame/close lifecycle
- `handle_input()` already captures keys when `remote_tui_id` is active

The TS layer's only real job is **executing Pi component factories** and
calling `.render(width)` to produce text lines. This can be replaced by a
thin TS bridge that only does factory execution + render, with all state
management (focus, overlay stack, key routing) moved to Rust.

## Target Architecture

```
Terminal → crossterm Event → AppView::handle_input()
  ├─ extension shortcut match? → Effect::ShortcutDispatch { key }
  │                               → RPC "shortcut/dispatch" → Pi handler
  ├─ remote_tui_id active? → Rust-side key routing
  │   ├─ overlay stack management (push/pop/focus)
  │   ├─ Effect::RemoteTuiInput for component-internal keys
  │   └─ Esc → Effect::RemoteTuiCancel
  └─ normal → ActionRegistry.lookup() → prompt editor / scrollback
```

## New RPC Messages

### Rust → Pi (requests)

```jsonc
// Dispatch an extension shortcut
{ "method": "shortcut/dispatch", "params": { "key": "alt+t" } }

// Request component render at given width
{ "method": "component/render", "params": { "id": "uuid", "width": 120 } }
```

### Pi → Rust (notifications, existing + new)

```jsonc
// Existing: pi/ui/widget — pre-rendered frame lines
{ "method": "pi/ui/widget", "params": { "widgetKey": "remote_tui", "widgetLines": [...], "widgetPlacement": "aboveEditor" } }

// New: shortcut registry sync (on session_start and registerShortcut calls)
{ "method": "pi/ui/shortcuts", "params": {
  "shortcuts": [
    { "key": "alt+t", "description": "Translate last response", "extension": "pi-language-tutor", "enabled": true }
  ]
}}
```

## Extension Shortcut Dispatch (Task 2)

### Data flow

1. Pi-side bridge extension (`pi-grok-rust-tui-bridge`) patches
   `ExtensionRunner.prototype.setUIContext` to capture the runner instance.
2. On `session_start`, calls `runner.getShortcuts({})` and sends the registry
   to Rust via `pi/ui/shortcuts` notification.
3. Rust stores the registry in `AppView.external_ui.extension_shortcuts`.
4. In `handle_input()`, BEFORE the remote_tui check, match against the
   registry. If hit → `Effect::ShortcutDispatch { key }` → adapter sends
   `shortcut/dispatch` RPC → Pi bridge calls the handler.

### Config

Rust reads `~/.pi/shortcut-manager.json` at startup and on `pi/ui/shortcuts`
refresh. Supports enable/disable/remap without Pi round-trip.

### Key matching

Rust uses crossterm `KeyEvent` directly. The registry stores keys in Pi's
KeyId format (`alt+t`, `ctrl+shift+x`). A `key_id_matches(event, key_id)`
function converts between the two.

## Component Render Pipeline (Task 3)

### Pi-side bridge (`pi-grok-rust-tui-bridge`)

A minimal TS extension that:
1. Intercepts `ctx.ui.custom(factory)` calls
2. Executes the factory with a stub TUI (same as current remote-tui)
3. Calls `component.render(width)` to get text lines
4. Sends lines to Rust via existing `pi/ui/widget` notification
5. Forwards key input from Rust to `component.handleInput(data)`
6. Re-renders after each input and sends updated frame

This is essentially the current remote-tui **minus** the keyfile mechanism.
Keys arrive via RPC request instead of file watching.

### Rust-side overlay stack

```rust
struct OverlayStack {
    stack: Vec<OverlayEntry>,  // push/pop
    focused: usize,            // index into stack
}

struct OverlayEntry {
    id: String,
    title: Option<String>,
    lines: Vec<String>,        // last rendered frame
}
```

- `open` → push entry, set `remote_tui_id`
- `frame` → update top entry's lines, re-project
- `close` → pop entry, clear `remote_tui_id` if empty
- `showOverlay` → push, `hideOverlay` → pop to previous

### Key routing in Rust

When `remote_tui_id` is active:
- Esc → cancel (pop overlay or close session)
- All other keys → `Effect::RemoteTuiInput { id, data }` → RPC to Pi bridge
  → `component.handleInput(data)` → re-render → frame back to Rust

This is **identical to current behavior** — the only change is transport
(RPC instead of keyfile).

## /shortcuts Interactive UI (Task 4)

Rust-native modal (like the existing model picker / extensions list):
- `ActionId::OpenShortcutManager` bound to no default key (invoked via `/shortcuts`)
- Renders as a scrollable list with columns: Key | Description | Extension | Status
- Navigation: ↑↓, Enter toggles enable/disable, R enters remap mode
- Remap mode: "Press new key..." → captures next KeyEvent → saves
- Esc exits, writes config to `~/.pi/shortcut-manager.json`

## Migration Path (Task 5)

1. Ship `pi-grok-rust-tui-bridge` as the new Pi-side extension
2. Rust reads shortcut registry from RPC instead of keyfile
3. Remove keyfile mechanism from remote-tui
4. Remove `pi-grok-remote-tui` and `pi-grok-shortcut-manager` extensions
5. Update `grok-pi.rs` to load only the new bridge

## Files to Create/Modify

| File | Action |
|------|--------|
| `extensions/pi-grok-rust-tui-bridge/index.ts` | Create — thin factory executor + RPC |
| `xai-grok-pager/src/app/extension_shortcuts.rs` | Create — registry, matching, config |
| `xai-grok-pager/src/app/app_view.rs` | Modify — shortcut intercept in handle_input |
| `xai-grok-pager/src/app/acp_handler/mod.rs` | Modify — handle `pi/ui/shortcuts` |
| `xai-grok-pager/src/views/shortcut_manager.rs` | Create — interactive UI modal |
| `pi-grok-adapter/src/pi_adapter.rs` | Modify — `shortcut/dispatch` RPC method |
| `grok-pi.rs` | Modify — swap extensions |
