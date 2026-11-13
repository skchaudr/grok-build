# Session Tree 与 Pi-main 对齐修复

**日期**: 2026-07-22
**涉及文件**:
- `crates/codegen/xai-grok-pager/src/views/session_tree.rs`
- `crates/codegen/xai-grok-pager/src/app/modals.rs`

**参考实现**: `pi-main/packages/coding-agent/src/modes/interactive/components/tree-selector.ts`

---

## 背景

grok-pager 的 Session Tree 组件（Rust/ratatui）与 pi-main 的 TreeSelector（TypeScript/pi-tui）存在三处功能/体验不对齐，影响用户操作效率和可发现性。

---

## 问题一：快捷键不透明

### 问题描述

pi-main 在 tree 顶部有常驻 `TreeHelp` 组件，通过 `TREE_HELP_ITEMS` 数组动态渲染所有可用快捷键。grok-pager 仅底部一行硬编码状态栏，只覆盖 6 个键，大量功能（复制、标签、filter 直切、详情展开、文件回滚等）对用户完全不可见。

### 修复方案

将底部 help bar 扩展为完整覆盖，并按 focus 状态显示上下文提示：

| Focus 状态 | Help bar 内容 |
|---|---|
| List（默认） | `↑/↓ move · ←/→ page · Tab/Alt+←/→ branch · c copy · l label · Shift+T time · filters Ctrl+D/T/U/L/A · cycle Ctrl+O · Ctrl+R detail · r rollback · Enter navigate · / search · Esc` |
| LabelEdit | `label edit · Enter save · Esc cancel` |
| SummarizePrompt | `↑/↓ navigate · Enter select · Esc cancel` |
| SummarizeCustom | `type instructions · Enter confirm · Esc cancel` |
| DetailExpanded | `Ctrl+R collapse · ↑/↓ scroll detail` |

---

## 问题二：折叠功能不好用

### 问题描述

| 维度 | pi-main | grok-pager（修复前） |
|---|---|---|
| 折叠键 | `ctrl+←` / `ctrl+→`（双功能：折叠 or 跳分支） | `Tab`（纯 toggle） |
| 分支跳转 | `findBranchSegmentStart()` — 不可折叠时跳到上/下一个分支点 | 无 |
| 可折叠判定 | 必须是 root 或 segment start（父节点有多个可见子节点） | 只要有可见后代就可折叠 |

### 修复方案

1. **新增 `FoldDirection` 枚举**（Up / Down）

2. **新增 `fold_or_navigate()` 方法** — 双功能语义：
   - 可折叠且未折叠 → 折叠
   - 已折叠 → 展开
   - 否则 → 调用 `find_branch_segment_start()` 跳到下一个分支点

3. **收紧 `is_foldable()`** — 只在以下情况返回 true：
   - 节点是可见 root（无可见父节点）
   - 节点是 segment start（可见父节点有多个可见子节点）

4. **按键绑定**：
   - `Tab` → `fold_or_navigate(Down)`
   - `BackTab` (Shift+Tab) → `fold_or_navigate(Up)`
   - `Alt+←` → `fold_or_navigate(Up)`（匹配 pi-main 的 `option+←`）
   - `Alt+→` → `fold_or_navigate(Down)`（匹配 pi-main 的 `option+→`）

5. **match arm 排序**：`Alt+←/→` 放在 `Left/Right | PageUp/PageDown` 之前，避免被普通翻页捕获。

---

## 问题三：缺少 "Summarize branch?" 交互提示

### 问题描述

pi-main 在 Enter 选中非当前节点后弹出三选一交互：
```
Summarize branch?
 → No summary
   Summarize
   Summarize with custom prompt
```
支持 Esc 取消回退到 tree、自定义指令编辑器。

grok-pager 修复前：`Enter` 直接导航（`summarize=false`），`Shift+Enter` 隐藏快捷键导航（`summarize=true`），无自定义指令，无取消回退。

### 修复方案

1. **新增 focus 状态**：`SummarizePrompt`、`SummarizeCustom`

2. **新增状态字段**：
   - `summarize_target_id: Option<String>` — 待导航的 entry id
   - `summarize_cursor: usize` — 三选一光标
   - `summarize_custom_draft: String` — 自定义指令草稿

3. **交互流程**：
   ```
   Enter（非当前节点）
     → begin_summarize_prompt(entry_id)
     → 显示三选一（detail pane 接管，body 文本隐藏）
     → ↑/↓ 移动光标
     → Enter 确认：
         • No summary → Navigate { summarize: false }
         • Summarize → Navigate { summarize: true }
         • Custom → 切换到 SummarizeCustom 编辑器
     → Esc → cancel_summarize()，回到 List focus
   ```

4. **UI 细节**：
   - Detail pane 高度在 summarize 激活时从 4 行扩展到 8 行
   - Body 文本在 summarize 激活时不渲染（避免溢出/杂乱）
   - 选中当前 leaf 时显示 "Already at this point" toast，不弹 prompt
   - 鼠标双击也走 summarize prompt 流程

5. **`SummarizeConfirmAction` 枚举**：
   - `Navigate { entry_id, summarize, custom_instructions }` — 执行导航
   - `EnterCustomEditor` — 切换到自定义编辑器

---

## 验证

```bash
cargo check -p xai-grok-pager
# 0 errors, 0 warnings
```

---

## 后续可选优化

- [x] 添加 `branchSummarySkipPrompt` 设置（跳过 prompt 直接导航）
  - 新增 `pi_tree_skip_summary_prompt` UiConfig 字段 + 设置注册表 + dispatch 全链路
  - Enter 时检查 `state.skip_summary_prompt`，为 true 则直接导航不弹 prompt
- [x] Summarize 进行中显示状态指示器
  - 导航时 toast 区分三种模式："Navigating session tree…" / "Navigating · generating branch summary…" / "Navigating · generating branch summary (custom)…"
- [ ] 支持 Escape 中断正在进行的 branch summary 生成
  - **阻塞原因**：grok-pager 的 ACP 层没有通用请求取消机制。branch summary 由 Pi 服务端在 `navigate_tree` ACP 调用中生成，无法从客户端中断。需要先在 ACP 协议层实现 request cancellation 才能实现此功能。
- [x] Help bar 从配置动态生成（支持用户自定义 keybinding）
  - 重构为 `TREE_HELP_ITEMS` 常量 + `session_tree_help_line()` 函数，按 focus 状态渲染上下文提示
