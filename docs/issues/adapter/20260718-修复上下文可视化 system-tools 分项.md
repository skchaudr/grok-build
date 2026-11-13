---
id: "2026-07-18-修复上下文可视化-system-tools-分项"
title: "修复 grok-pi 上下文可视化 system/tools/AGENTS 分项"
status: "completed"
created: "2026-07-18"
updated: "2026-07-18"
category: "adapter"
tags: ["workhub", "context", "session-info", "extension", "adapter"]
---

# Issue: 修复 grok-pi 上下文可视化 system/tools/AGENTS 分项

## Goal

点击原生 context bar / 运行 `/context` 时，`ContextInfoBlock` 应正确显示：

| 行 | 期望 |
|---|---|
| System prompt | 非 0（含 base + append + AGENTS.md + skills listing 的完整 system） |
| Messages | 对话文本缩放后份额 |
| Reasoning/overhead | `used - system - messages`（含 tool payload / residual） |
| Tool definitions | 活跃工具 schema 估算 + 工具数量 |
| usage_categories | AGENTS.md / Append system prompt / Skills 明细（信息行，与 system 重叠） |

当前错误表现：

```text
◆ System prompt          0 tokens   (0.0%)
◆ Messages            2.2k tokens   (0.6%)
◆ Reasoning/overhead  137k tokens    (37%)
◈ Tool definitions       0 tokens   (0.0%) · 0 tools
```

系统提示词与 tool definitions 被吞进 Reasoning/overhead。

## 根因

1. `context_projection::build_session_info_response` 因 Pi RPC 未暴露 system/tool 文本，写死：
   - `systemPromptTokens: 0`
   - `toolDefinitionsCount: 0`
   - `toolDefinitionsTokens: 0`
2. Issue `20260717-实现-x.ai-session-info-上下文点击` 已记录该边界为非目标。
3. 进程内 `pi-context` 扩展能用 `ctx.getSystemPrompt()` / `pi.getAllTools()`，但 grok-pi 不复用其 TUI。

## 设计

| 边界 | 归属 |
|---|---|
| 可见 UI | Grok 原生 `ContextInfoBlock`（不改 Pager 渲染） |
| 进程内读 prompt/tools | 注入 NamedTempFile Pi extension（官方 extension API） |
| 权威 used/window | 仍来自 Pi `get_session_stats.contextUsage` |
| 消息估算 | 仍来自 `get_messages` / `get_entries` |
| 映射 | `pi-grok-adapter` headless only |

### Extension：`__pi_context_breakdown`

- 路径：`extensions/pi-grok-context/index.ts`，由 `grok-pi` NamedTempFile 注入
- 隐藏 slash（adapter `is_bridge_command` 过滤）
- 输出文件：`PI_GROK_CONTEXT_BREAKDOWN`（进程唯一 tempfile）
- 读取：
  - `ctx.getSystemPrompt()` → full system raw tokens (`ceil(len/4)`)
  - `ctx.getSystemPromptOptions()` → append / contextFiles(AGENTS.md) / skills
  - `pi.getActiveTools()` + `pi.getAllTools()` → tool definition raw + count
- 写 JSON：

```json
{
  "version": 1,
  "systemPromptTokensRaw": 12345,
  "toolDefinitionsCount": 12,
  "toolDefinitionsTokensRaw": 8000,
  "appendTokensRaw": 200,
  "contextFiles": [{ "path": ".../AGENTS.md", "tokensRaw": 1500 }],
  "skillsCount": 8,
  "skillsTokensRaw": 900
}
```

### Adapter 投影

`handle_session_info`：

1. `get_session_stats` + `get_messages`（现有）
2. best-effort `run_bridge_command("__pi_context_breakdown")` + 读 breakdown 文件
3. `build_session_info_response(..., breakdown)`：
   - raw parts = `[system, toolDefs, messages, tool_payload]`
   - `scale_token_parts` 使和 = 权威 `used`
   - `systemPromptTokens` / `toolDefinitions*` / `messageTokens` 写缩放值
   - `usage_categories` 用同一 ratio 缩放 append / 各 AGENTS 文件 / skills（信息行）
4. bridge 失败时保持旧 fallback（system/tools=0）

### 非目标

- 不改 Pi 源码扩 RPC
- 不在 adapter 渲染 TUI / 不复用 pi-context overlay
- 不实现 Grok MCP usage 行（Pi 无对等 MCP 列表）

## 验收

- [x] 注入 `__pi_context_breakdown` NamedTempFile extension + `PI_GROK_CONTEXT_BREAKDOWN`
- [x] `build_session_info_response` 缩放 system/tools/messages；usage_categories 含 AGENTS/Append/Skills
- [x] bridge 失败时 system/tools 回退 0（兼容旧路径）
- [x] FEATURE_MATRIX 更新边界说明
- [ ] 手测 `/context` 显示非 0 System prompt / Tool definitions
- [x] `cargo test -p pi-grok-adapter`
- [x] `cargo check -p xai-grok-pager-bin --bin grok-pi`

## 验证

```bash
cargo test -p pi-grok-adapter
# 74 passed
cargo check -p xai-grok-pager-bin --bin grok-pi
# ok
cargo test -p xai-grok-pager-bin --bin grok-pi context_extension
# 1 passed
```

## 实现

- `extensions/pi-grok-context/index.ts` — 进程内读 system/tools/AGENTS/append/skills，写 JSON
- `grok_pi/context_extension.rs` — NamedTempFile 注入 + breakdown 路径
- `grok-pi.rs` — `--extension` + `PI_GROK_CONTEXT_BREAKDOWN` + `PiAgent::new(..., breakdown)`
- `context_projection.rs` — 缩放 system/tools/messages；`usageCategories` 明细
- `pi_adapter.rs` — `fetch_context_breakdown` + bridge 命令过滤
