# Tool 运行呼吸动画（Wave Accent）完全失效 — 排查研究文档

## 问题描述

grok-pi 在 **fullscreen 模式**下，tool 运行时的左侧 `┃` accent bar 波浪动画完全消失。
- 长时间运行的 bash 命令：无动画
- 长时间思考（Thinking）：无动画
- 所有 tool 类型：无动画
- 不是时间/批处理问题，是**完全**没有效果

---

## 动画管线完整架构

```
┌─────────────────────────────────────────────────────────────────────┐
│ 1. Pi Adapter 层                                                     │
│    pi_adapter.rs::handle_tool_start()                                │
│    → send_update(SessionUpdate::ToolCall(InProgress))                │
│    → meta 中 stamp promptId (来自 state.live_prompt_id)              │
├─────────────────────────────────────────────────────────────────────┤
│ 2. ACP Handler 层                                                    │
│    acp_handler/mod.rs::handle()                                      │
│    → promptId-mismatch gate 检查                                     │
│    → 通过后调用 agent.session.handle_update(update, meta, scrollback)│
├─────────────────────────────────────────────────────────────────────┤
│ 3. Tracker 层                                                        │
│    tracker.rs::handle_tool_call()                                    │
│    → scrollback.push_block(block)                                    │
│    → scrollback.set_last_running(true)                               │
│    → running.insert(entry_id)                                        │
├─────────────────────────────────────────────────────────────────────┤
│ 4. Tick 驱动层                                                       │
│    app_view.rs::tick_demand()                                        │
│    → needs_animation() || !session.state.is_idle()                   │
│    event_loop.rs::schedule_tick()                                    │
│    → 每 33ms 触发 AppView::tick()                                    │
│    → scrollback.tick() 递增 tick 计数器                              │
├─────────────────────────────────────────────────────────────────────┤
│ 5. 渲染层                                                            │
│    scrollback_pane.rs::render_content()                              │
│    → 传递 state.current_tick() 给 render_scrolled_entries            │
│    entry_renderer.rs::render()                                       │
│    → self.accent(content_width) 获取 AccentStyle                     │
│    → 如果 accent_style.animated == true:                             │
│      → wave_brightness(tick, row, wave_rows, WAVE_SPEED)             │
│      → blend_color(bg, color, brightness)                            │
│      → 逐行绘制 ┃ accent bar                                         │
├─────────────────────────────────────────────────────────────────────┤
│ 6. 数学层                                                            │
│    tokyonight.rs::wave_brightness(tick, row, wave_rows, speed)       │
│    → sin²(tick * speed + row / wave_rows * 2π)                       │
│    → 输出 [0, 1] 亮度值                                              │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 已排除的原因（全部验证过）

### ❌ 1. Minimal 模式 hide_accent

**假设**：`live.rs:400` 的 `.with_hide_accent(true)` 隐藏了 accent bar。

**排除原因**：用户确认使用 fullscreen 模式。`hide_accent` 只在 minimal 模式的
`commit.rs` 和 `live.rs` 中设置为 true。Fullscreen 模式的 `scrollback_pane.rs`
渲染路径不设置 `hide_accent`，默认为 false。

**验证**：`entry_renderer.rs` L71 默认 `hide_accent: false`，L91 构造函数确认。

---

### ❌ 2. 事件批处理/时间问题

**假设**：Pi 处理 tool 太快（<33ms），ToolCall(InProgress) 和 ToolCallUpdate(Completed)
在同一事件循环迭代中被处理，running 状态存在 0 帧。

**排除原因**：用户明确说长时间运行的 bash 命令（如 `sleep 9999`）和长时间思考也没有动画。
这完全不是时间问题。

---

### ❌ 3. blend_color 返回 None

**假设**：如果 `bg_base` 或 `accent_running` 是 named ANSI 颜色（如 `Color::Red`），
`blend_color()` 返回 `None`，`.unwrap_or(color)` 给出静态颜色，波浪不可见。

**排除原因**：Pi 主题通过 `resolve_color()` 产生颜色，`derive_canvas()` 始终产生
`Color::Rgb`。`quantized()` 可能转为 `Color::Indexed`，但 `blend_color` 对 Indexed
也返回 `Some`。只有 named ANSI 颜色（`Color::Red` 等）和 `Color::Reset` 返回 None。

**验证**：`color.rs` L121-124：`Rgb → Some`, `Indexed → Some`, 其他 → None。
Pi 主题的 `bg_base` 来自 `derive_canvas()`，始终 Rgb。

---

### ❌ 4. HorizontalLayout::ACCENT 为 0

**假设**：accent bar 宽度为 0，无处渲染。

**排除原因**：`layout.rs` L38 明确定义 `pub const ACCENT: u16 = 1`。
只有 `hide_accent == true` 时才设为 0（minimal 模式专属）。

---

### ❌ 5. 上游 merge 破坏

**假设**：`e588a2e merge(upstream)` 的 467 行 tracker.rs 变更破坏了动画。

**排除原因**：实际 diff 全部是 rustfmt 格式化变更（空格、换行、json! 宏格式）。
`acp_handler/mod.rs` 变更仅为 import 添加和格式化。`app_view.rs` 变更为
`AppRenderParams` 重构（voice/esc 参数分组），不影响动画逻辑。

---

### ❌ 6. accent_enabled 默认 false

**假设**：appearance 配置中 accent 被禁用。

**排除原因**：所有 tool block 的 `accent_enabled` 默认为 `true`。
`ThinkingConfig::default()` 中 `accent_enabled: true, animate: true`。
`ExecuteConfig::default()` 中 `accent_enabled: true`。

---

### ❌ 7. ThinkingConfig.animate 默认 false

**假设**：Thinking block 的动画开关默认关闭。

**排除原因**：`config.rs` L575：`animate: true` 是默认值。

---

### ❌ 8. tick_demand() 返回 None

**假设**：动画 tick 从未被调度。

**排除原因**：`tick_demand()` 在 `!agent.session.state.is_idle()` 时返回 Fast。
只要 session 处于 TurnRunning 状态，tick 就会被调度。
即使 `needs_animation()` 返回 false，`!is_idle()` 也保证 Fast。

**验证**：`app_view.rs` L6100：`|| !agent.session.state.is_idle()`

---

### ❌ 9. scrollback.tick() 未被调用

**假设**：事件循环中 `AppView::tick()` 未被调用。

**排除原因**：`event_loop.rs` L2360：`else if app.tick() { presenter.request(false); }`
在 animation_tick 触发时调用。`app_view.rs` L5797：`needs_redraw |= agent.scrollback.tick()`。

---

### ❌ 10. current_tick() 始终为 0

**假设**：tick 计数器从未递增。

**排除原因**：`scrollback.tick()` 第一行就是 `self.tick = self.tick.wrapping_add(1)`。
只要 `AppView::tick()` 被调用，tick 就会递增。

---

### ❌ 11. RenderBlock::accent() 委托错误

**假设**：`delegate_block!` 宏未正确委托到内部 block 的 accent()。

**排除原因**：`block.rs` L488：`fn accent(&self, ctx: &BlockContext) -> Option<AccentStyle> { delegate_block!(self, accent(ctx)) }`。
所有 ToolCallBlock 变体（Execute、Read、Edit、Search、Other 等）都实现了 `accent()`。

---

### ❌ 12. ExecuteConfig.running_accent 颜色问题

**假设**：`running_accent` 颜色与背景相同，波浪不可见。

**排除原因**：`ExecuteConfig::default()` 中 `running_accent: Theme::current().accent_running`。
Pi 主题中 `accent_running: border_accent`，这是一个有对比度的颜色。

---

### ❌ 13. promptId gate 丢弃所有事件

**假设**：`acp_handler/mod.rs` 的 promptId-mismatch gate 丢弃了所有 ACP 通知。

**排除原因**：如果所有事件被丢弃，用户不会看到任何 tool 输出。但用户能看到 tool 结果，
说明事件确实到达了 tracker。gate 只在 `meta.prompt_id != current_prompt_id` 时丢弃。
Pi adapter 的 `send_update()` 从 `state.live_prompt_id` 取 promptId，这个值来自
pager 发送的 PromptRequest 中的 `meta.promptId`，所以应该匹配。

---

### ❌ 14. session.state 未设为 TurnRunning

**假设**：Pi adapter 路径下 `start_turn()` 未被调用，state 保持 Idle。

**排除原因**：`dispatch/queue.rs` L342 在 drain queue 时设置 `current_prompt_id`，
`agent_view/session.rs` L469 的 `start_turn_boundary()` 调用 `session.start_turn()`
设置 `state = TurnRunning`。grok-pi 的 prompt 提交走同样的 queue drain 路径。

---

### ❌ 15. 9a1b281 throttle redraw 提交

**假设**：`has_expanded_edit_in_viewport()` 始终返回 true，抑制了所有动画重绘。

**排除原因**：该函数只在有 Expanded 的 Edit block 在视口内时返回 true。
后续的 `5f03be4` 提交已经修复了 spinner 被 gate 的问题（`needs_redraw |= !is_idle && spinner_frame_tick` 不再被 `heavy_expanded_edit` gate）。

---

### ❌ 16. suppress_sticky_headers 影响动画

**假设**：`5f03be4` 添加的 `suppress_sticky_headers` 影响了动画渲染。

**排除原因**：`suppress_sticky_headers` 只影响 sticky header 的渲染，不影响
entry 内容的渲染。accent bar 是 entry 内容的一部分，不受 sticky 逻辑影响。

---

## 未排除的可能原因

### 🔍 1. EntryRenderer 的 `self.accent()` 返回 None

**条件**：如果 `accent()` 返回 None，则整个 accent 渲染被跳过（包括波浪）。

**可能触发**：
- `accent_enabled = false`（已排除默认值，但运行时配置可能覆盖）
- `ctx.mode == DisplayMode::Collapsed`（Thinking block 在 Collapsed 时返回 None）
- 某些 tool block 的 `accent()` 在特定条件下返回 None

**关键问题**：tool 在运行时是什么 DisplayMode？如果是 Collapsed，Thinking 的 accent 返回 None。

---

### 🔍 2. DisplayMode 问题 — tool 运行时被设为 Collapsed

**条件**：如果 tool entry 在运行时 `display_mode == Collapsed`，某些 block 的
`accent()` 返回 None（如 ThinkingBlock），或者 `use_collapsed_accent` 为 true
导致使用 dimmed 静态字符而非波浪。

**关键代码**（entry_renderer.rs L770-775）：
```rust
let use_collapsed_accent =
    self.groupable && self.entry.display_mode == DisplayMode::Collapsed && !has_hook_lines;
```

如果 `use_collapsed_accent == true`，即使 `accent_style.animated == true`，
渲染路径也会走到 dimmed collapsed accent 分支（L835-845），**跳过波浪动画**。

**等等——不对**。看代码逻辑：
```rust
if is_pending && accent_style.animated { ... }
else if accent_style.animated { /* 波浪 */ }
else if use_collapsed_accent && !self.is_selected { /* dimmed */ }
else { /* static */ }
```

`animated` 分支在 `use_collapsed_accent` 之前，所以如果 `animated == true`，
波浪应该优先渲染。**除非** `accent_style` 本身是 None 或 `animated == false`。

---

### 🔍 3. verb-group 折叠导致 running entry 不可见

**条件**：如果 tool entry 被 verb-group 折叠（`group_tool_verbs` 开启），
running 的 entry 可能被折叠在组内，不在视口中渲染。

**关键**：`any_running_in_viewport()` 检查 running entry 是否在 measurement_window 内。
如果被折叠，entry 的 index 可能不在 paint_window 内。

---

### 🔍 4. `is_running` 在渲染时为 false（状态竞态）

**条件**：`handle_tool_call_update` 的非完成路径中，`replace_tool_block()` 被调用。
`replace_tool_block()` 替换了 `entry.block`，但**不改变** `entry.is_running`。
所以 `is_running` 应该保持 true。

但是——`handle_tool_call_update` 的完成路径调用 `scrollback.finish_running(entry_id)`，
这会设置 `is_running = false`。如果完成事件在渲染之前到达...

**但用户说长时间命令也没有动画**，所以这不是唯一原因。

---

### 🔍 5. Pi adapter 的 tool_kind() 映射问题

**条件**：如果 Pi 发送的 tool name 被 `tool_kind()` 映射为错误的 ToolKind，
可能影响 block 类型和 accent 行为。

**关键**：`tool_call_to_block()` 根据 `tc.kind` 决定 block 类型。
如果所有 Pi tool 都被映射为 `ToolKind::Other`，则使用 `OtherToolCallBlock`。

`OtherToolCallBlock::accent()` 的实现：
```rust
fn accent(&self, ctx: &BlockContext) -> Option<AccentStyle> {
    if ctx.mode == DisplayMode::Collapsed { return None; }
    if self.error.is_some() { Some(AccentStyle::static_color(theme.accent_error)) }
    else if ctx.is_running { Some(AccentStyle::animated(theme.accent_running)) }
    else { Some(AccentStyle::static_color(theme.accent_tool)) }
}
```

**关键**：如果 `ctx.mode == DisplayMode::Collapsed`，返回 None！

---

### 🔍 6. ⭐ DisplayMode::Collapsed 是根因？

**假设**：Pi adapter 路径下，tool entry 的 `display_mode` 在运行时被设为 `Collapsed`，
导致所有 tool block 的 `accent()` 返回 `None`，accent bar 完全不渲染。

**验证方向**：
- 检查 `handle_tool_call()` 中 `push_block()` 后 entry 的 `display_mode`
- 检查 `default_display_mode()` 对各 tool 类型返回什么
- 检查是否有全局配置将所有 tool 设为 Collapsed

**关键代码**：
- `ScrollbackEntry::new()` 中 `display_mode` 的初始值
- `RenderBlock::default_display_mode()` 的实现
- `appearance.scrollback.display.group_max_visible` 和 verb-group 折叠

---

### 🔍 7. ⭐ verb-group 自动折叠 + groupable 标记

**假设**：`group_tool_verbs` 配置开启后，连续的同类 tool 被自动折叠为 verb-group。
折叠后的 entry `display_mode = Collapsed`，accent() 返回 None。

**关键**：`push_block()` 后的 `prepare_layout()` 中，如果 `group_max_visible > 0`
或 `group_tool_verbs == true`，新 push 的 groupable entry 可能被立即折叠。

**验证**：检查 `appearance.scrollback.display.group_tool_verbs` 的默认值和 Pi 配置。

---

## 下一步排查方向

1. **最优先**：确认 tool entry 在运行时的 `display_mode` 是什么
   - 在 `handle_tool_call()` 的 `set_last_running(true)` 后打印 `entry.display_mode`
   - 或者在 `EntryRenderer::accent()` 中打印 `ctx.mode` 和 `ctx.is_running`

2. **次优先**：确认 verb-group 折叠是否在运行时立即折叠了 running entry
   - 检查 `group_tool_verbs` 和 `group_max_visible` 的运行时值
   - 检查 `prepare_layout()` 中是否对 running entry 做了折叠

3. **第三**：确认 `accent()` 的返回值
   - 在 `EntryRenderer::accent()` 中打印返回的 `Option<AccentStyle>`

---

## 关键文件索引

| 文件 | 关键行 | 作用 |
|------|--------|------|
| `entry_renderer.rs` | L767-850 | accent 渲染决策链 |
| `entry_renderer.rs` | L355-361 | `self.accent()` 构建 BlockContext |
| `entry.rs` | L568-584 | `context()` 传递 `is_running` |
| `types.rs` | L28-48 | `AccentStyle` 定义 |
| `state/mod.rs` | L1243-1258 | `set_entry_running()` |
| `state/mod.rs` | L453-485 | `tick()` 递增计数器 |
| `state/mod.rs` | L519-521 | `needs_animation()` |
| `tracker.rs` | L914-928 | `handle_tool_call()` 设置 running |
| `tracker.rs` | L1440+ | `tool_call_to_block()` 映射 |
| `app_view.rs` | L6062-6120 | `tick_demand()` |
| `app_view.rs` | L5797 | `scrollback.tick()` 调用 |
| `event_loop.rs` | L2342-2367 | animation tick handler |
| `event_loop.rs` | L2930-2966 | `schedule_tick()` |
| `scrollback_pane.rs` | L960 | 传递 `state.current_tick()` |
| `render.rs` | L435 | `EntryRenderer.with_tick(tick)` |
| `tokyonight.rs` | L305-320 | `wave_brightness()` |
| `color.rs` | L190-207 | `blend_color()` |
| `layout.rs` | L38 | `ACCENT = 1` |
| `config.rs` | L383-410 | `AnimationConfig` |
| `config.rs` | L544-580 | `ThinkingConfig` |
| `config.rs` | L682-710 | `ExecuteConfig` |
| `pi/map.rs` | L133 | Pi 主题 `accent_running: border_accent` |
| `acp_handler/mod.rs` | L322-340 | promptId-mismatch gate |
| `pi_adapter.rs` | L1241-1268 | `send_update()` stamp promptId |
| `pi_adapter.rs` | L2287-2315 | `handle_tool_start()` |
| `pi_adapter.rs` | L3140-3155 | `live_prompt_id` 设置 |

---

## 教训

1. **不要假设是时间问题**：用户明确说长时间命令也没有动画，这排除了所有时序相关的假设。
2. **不要假设是模式问题**：用户明确说 fullscreen 模式，排除 minimal 相关假设。
3. **不要给多个选项**：用户要的是一个确定性的修复，不是 A/B/C/D。
4. **静态分析有极限**：当代码在每一层都"看起来正确"时，问题可能在运行时状态
   （如 DisplayMode、verb-group 折叠、配置覆盖）而非代码逻辑。
5. **最可能的根因方向**：tool entry 在运行时的 `display_mode` 为 `Collapsed`，
   导致 `accent()` 返回 `None`，accent bar 完全不渲染。这解释了为什么
   "完全没有动画"——不是动画代码有问题，而是 accent 根本没有被渲染。
