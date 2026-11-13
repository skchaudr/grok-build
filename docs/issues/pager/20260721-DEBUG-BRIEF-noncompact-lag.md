# Debug Brief: Pi-Grok 滚动/渲染卡顿 — 完整研究记录

> 状态：**已修复** | 创建：2026-07-21 | 最终确认：2026-07-22

## 1. 问题为什么出现

### 现象矩阵（初始报告）

| 场景 | Compact ON | Compact OFF |
|---|---|---|
| **Pi-Grok** live turn + expanded Edit | ✅ 流畅 | ❌ **卡** |
| **Pi-Grok** resume/idle + expanded Edit | ✅ 流畅 | ❌ **卡** |
| **上游 Grok**（任何状态、任何内容） | ✅ 流畅 | ✅ 流畅 |

### 关键约束

1. **Resume 证明不是事件驱动的。** 无 ACP 消息、无 spinner tick、无 live-turn 循环，卡顿仍然存在。
2. **上游 Grok 证明不是 Pager 渲染引擎本身。** 同一二进制、同一渲染代码、compact OFF + sticky headers + expanded Edit → 流畅。
3. **Compact 切换是即时修复。** 同一会话、同一滚动位置，toggle compact → 立即流畅。切回 → 立即卡顿。
4. **卡顿是每帧的。** 不是一次性开销。每次滚动 tick、每次按键、每次重绘都卡顿。

### 后续发现（2026-07-22）

经过错误修复后，问题变为 **compact ON 也卡**。最终确认：根因与 compact 无关，是 `prepare_layout` 宽度振荡导致**每帧全量 rebuild**，任何模式都卡。

## 2. 问题是如何发生的（根因）

### 真正的根因：`prepare_layout` 宽度振荡

**文件：** `crates/codegen/xai-grok-pager/src/app/agent_view/render.rs`

**Bug：** 我们在 timeline rail 重构时，把第二次 `prepare_layout` 的参数从 `scrollback_content` 改成了 `scrollback`：

```rust
// 上游（正确）：
self.scrollback.prepare_layout(
    layout.scrollback_content.width,   // 内容区宽度（减去 scrollbar/timeline）
    layout.scrollback_content.height,
);

// 我们（错误）：
self.scrollback.prepare_layout(
    layout.scrollback.width,           // 外层容器宽度（含 scrollbar/timeline）
    layout.scrollback.height,
);
```

### 机制

每帧 `AgentView::draw()` 调用两次 `prepare_layout`：

1. **第一次**（timeline 分支，line ~1146）：`scrollback_content.width` → 设 `last_width = W_content`
2. **第二次**（主渲染，line ~1428）：`scrollback.width` → `W_scrollback ≠ W_content`

由于两次宽度不同，`prepare_layout` 内部的 Case 1 判断 **每帧都触发**：

```rust
// Case 1: Cache missing or width changed - full rebuild
if self.layout_cache.is_none() || width != self.last_width {
    for entry in self.entries.values_mut() {
        entry.invalidate_cache();  // ← 每帧清空所有缓存！
    }
    self.ensure_layout_cache(width);           // 全量重建
    self.settle_visible_measurements(width);   // 重新测量
}
```

**结果：每帧都做 O(n) 全量 rebuild + 所有 entry 缓存失效 + 重新 markdown 解析/word-wrap。**

### 为什么上游不卡

上游两次调用都用 `scrollback_content`，宽度稳定，不触发 Case 1。第二次调用走 Case 3（仅 `compute_total_height_from_cache`，O(visible)）。

### 为什么之前误判为 compact 相关

- Compact OFF 时 viewport 更小（outer vpad 占 2 行），可见 entry 更多，rebuild 更贵 → 卡顿更明显
- Compact ON 时 viewport 更大，可见 entry 相对少 → 卡顿较轻但仍存在
- 初始测试时 compact ON 的卡顿被其他因素掩盖

## 3. 错误的修复尝试（已回退）

### 3.1 `5f03be4` — suppress sticky headers（错误）

```rust
// render.rs draw() 中：
self.scrollback
    .set_suppress_sticky_headers(!self.session.state.is_idle());
```

**意图：** 误判 sticky header 每帧重绘是卡顿原因。
**后果：** Live turn 时 non-compact 模式的用户消息黏性滚动完全消失。
**状态：** ❌ 已删除。

### 3.2 `9a1b281` — expanded edit viewport 节流（过度防御）

```rust
// event_loop.rs：
let draw_interval = if app.active_agent().is_some_and(|a| {
    a.scrollback.has_expanded_edit_in_viewport()
}) {
    min_draw_interval.max(Duration::from_millis(120))
} else {
    min_draw_interval
};

// app_view.rs tick()：
let heavy_expanded_edit = agent.scrollback.has_expanded_edit_in_viewport();
let scrollback_anim_redraw = agent.scrollback.tick();
if !heavy_expanded_edit {
    needs_redraw |= scrollback_anim_redraw;
}

// app_view.rs tick_demand()：
let heavy_expanded_edit = agent.scrollback.has_expanded_edit_in_viewport();
let scrollback_anim = agent.scrollback.needs_animation() && !heavy_expanded_edit;
let fast = scrollback_anim || ...
```

**意图：** 误判展开 Edit 的每帧重绘是卡顿原因。
**后果：** 展开 Edit 时动画冻结、帧率被人为压低。
**状态：** ❌ tick/tick_demand 中的门控已回退到上游。event_loop 的 120ms 节流保留（无害，但非必需）。

### 3.3 `046efed` — sticky header cache + edit gutter estimate（正确但非根因）

- `ensure_header_cached()`：sticky header 渲染走缓存而非每帧 `block.output()`
- `estimate_reserved_cols()`：Edit block 高度估算考虑 gutter 宽度
- `MAX_SETTLE_ITERS_PER_FRAME = 4`：防止 settle 迭代爆炸

**状态：** ✅ 保留。这些是正确的优化，即使不是根因也改善了性能。

## 4. 最终修复

### 4.1 根因修复：`prepare_layout` 宽度统一

```rust
// render.rs line ~1428，改回上游：
self.scrollback.prepare_layout(
    layout.scrollback_content.width,
    layout.scrollback_content.height,
);
```

### 4.2 删除错误的 sticky suppress

```rust
// render.rs draw() 中删除：
- self.scrollback
-     .set_suppress_sticky_headers(!self.session.state.is_idle());
```

### 4.3 回退 tick/tick_demand 中的 expanded edit 门控

```rust
// app_view.rs tick()，恢复上游：
- let heavy_expanded_edit = agent.scrollback.has_expanded_edit_in_viewport();
- let scrollback_anim_redraw = agent.scrollback.tick();
- if !heavy_expanded_edit {
-     needs_redraw |= scrollback_anim_redraw;
- }
+ needs_redraw |= agent.scrollback.tick();

// app_view.rs tick_demand()，恢复上游：
- let heavy_expanded_edit = agent.scrollback.has_expanded_edit_in_viewport();
- let scrollback_anim = agent.scrollback.needs_animation() && !heavy_expanded_edit;
- let fast = scrollback_anim
+ let fast = agent.scrollback.needs_animation()
```

## 5. 对上游的修改清单

### 保留的正确修改（相比上游 `a881e67`）

| 文件 | 修改 | 原因 |
|---|---|---|
| `scrollback/scrollback_pane.rs` | `ensure_header_cached()` 替代裸 `block.output()` | Sticky header 每帧重渲染是真实性能问题 |
| `scrollback/entry.rs` | `CachedHeaderOutput` + `ensure_header_cached` + `cached_header_output_ref` | 上述缓存的基础设施 |
| `scrollback/wrappers/entry_renderer.rs` | `estimate_reserved_cols()` 调用 | Edit block 高度估算修正 |
| `scrollback/blocks/tool/edit.rs` | `estimate_reserved_cols()` 实现 | 考虑 gutter 宽度 |
| `scrollback/block.rs` | `estimate_reserved_cols()` trait 方法 + delegate | 接口定义 |
| `scrollback/state/types.rs` | `MAX_SETTLE_ITERS_PER_FRAME = 4` | 防止 settle 迭代爆炸 |
| `scrollback/state/layout.rs` | settle 迭代上限 `.min(MAX_SETTLE_ITERS_PER_FRAME)` | 同上 |
| `scrollback/state/mod.rs` | `has_expanded_edit_in_viewport()` | 保留（event_loop 120ms 节流仍引用） |
| `app/event_loop.rs` | expanded edit 时 `min_draw_interval.max(120ms)` | 保留（无害防御） |

### 已回退的错误修改

| 文件 | 回退内容 | 原因 |
|---|---|---|
| `app/agent_view/render.rs` | `prepare_layout(layout.scrollback.width, ...)` → 改回 `scrollback_content` | **根因** |
| `app/agent_view/render.rs` | `set_suppress_sticky_headers(!is_idle())` → 删除 | 误杀 sticky header |
| `app/app_view.rs` tick() | `heavy_expanded_edit` 门控 → 恢复 `needs_redraw \|= agent.scrollback.tick()` | 误冻结动画 |
| `app/app_view.rs` tick_demand() | `heavy_expanded_edit` 门控 → 恢复 `agent.scrollback.needs_animation()` | 误降帧率 |

## 6. 教训

1. **不要在不理解根因的情况下"修"性能问题。** 三次错误修复（suppress sticky、freeze animation、throttle draw）都是在错误假设上叠加的。
2. **Diff 上游是最快的定位方式。** 最终通过 `git diff a881e67..HEAD` 在 5 分钟内找到了宽度振荡，而之前花了数小时在错误方向上。
3. **`prepare_layout` 是幂等的前提是宽度不变。** 任何改变传入宽度的重构都必须确保同一帧内多次调用使用相同值。
4. **性能问题的"即时修复"（toggle compact）可能是误导。** Compact 改变了 viewport 大小，让 rebuild 成本变化，但根因是 rebuild 本身不应该发生。

## 7. 复现步骤

1. 启动 `grok-pi`（Pi 集成模式）
2. 让 Pi 写/编辑一个文件（产生 Edit block）
3. 展开 Edit block（Enter）
4. 滚动或按任意键 → 观察每帧卡顿
5. 切换 compact 模式 → 卡顿程度变化但都存在（修复前）
6. 修复后：任何模式下滚动/按键均流畅

## 8. 文件地图

| 文件 | 角色 |
|---|---|
| `app/agent_view/render.rs` | **根因所在** — `prepare_layout` 调用点 |
| `scrollback/state/mod.rs` | `prepare_layout` 入口，Case 1/2/3 分支 |
| `scrollback/state/layout.rs` | `settle_visible_measurements`、`measure_window_exact` |
| `scrollback/scrollback_pane.rs` | Sticky header 渲染（H1 缓存修复） |
| `scrollback/entry.rs` | `CachedOutput`、`ensure_cached`、`ensure_header_cached` |
| `scrollback/wrappers/entry_renderer.rs` | `estimate_content_lines`（H3 gutter 修复） |
| `scrollback/blocks/tool/edit.rs` | `estimate_reserved_cols` 实现 |
| `app/app_view.rs` | `tick()`、`tick_demand()`（已回退门控） |
| `app/event_loop.rs` | draw throttle（保留 120ms 下限） |
