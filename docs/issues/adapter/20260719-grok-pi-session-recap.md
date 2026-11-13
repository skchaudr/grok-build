---
id: "2026-07-19-grok-pi-session-recap"
title: "grok-pi 支持 session recap（F2 开关/模型 + 系统语言）"
status: "completed"
created: "2026-07-19"
updated: "2026-07-18"
category: "adapter"
tags: ["workhub", "recap", "settings", "extension"]
---

# Issue: grok-pi 支持 session recap

## Goal

让 `grok-pi` 具备可预测、低成本且不重复的 `/recap` + 自动 return-from-away recap：只使用 F2 配置的 recap model，正文使用操作系统首选语言，并严格限制输入上下文。

## 边界

1. **Grok Pager 是唯一 TUI** — 复用 `Action::SendRecap` / `x.ai/recap` / `SessionRecap` scrollback。
2. **Pi 是唯一 agent core** — 不改 Pi 源码；用 NamedTempFile extension + `complete()` 做 display-only 侧调用。
3. **adapter headless** — 只做 RPC/ext 映射，不渲染。
4. recap **不得写入** Pi 会话 LLM 历史（custom message `display:false` 仅桥接）。

## 方案

| 层 | 行为 |
|---|---|
| composition | 注入 `__pi_grok_recap` extension（`complete` + `sendMessage` custom） |
| adapter | `initialize.meta.sessionRecap=true`；`x.ai/recap` → bridge command；custom `pi-grok-recap/v1` → `SessionRecap` / `SessionRecapUnavailable` |
| pager | F2：`session_recap` 开关 + 可选 `recap_model` 覆盖；未配置时复用当前会话模型；recap 模型选择复用原生 `/model` picker，不自绘 DynamicEnum 列表；失焦期间 poll 预生成 |
| extension gate | auto 仅在 ≥3 user turns、最后完成 turn ≥3 分钟、且上次成功 recap 后出现新 user turn时生成；manual 只要求有 user turn |
| 输入预算 | 仅保留最新 compaction summary + 最近有效 turns 的紧凑文本，按字符上限截断；不发送整段历史/思考/完整工具结果 |
| 语言 | macOS 优先读 `AppleLanguages`，再回退 locale 环境变量；instruction 强制使用系统语言 |

## 验收

- [x] `/recap` 路径：`Action::SendRecap` → `x.ai/recap` → `__pi_grok_recap` → `SessionRecap`
- [x] F2：`session_recap` 开关（auto）+ 可选 `recap_model` 覆盖（空值回退当前会话模型）
- [x] auto gate：≥3 turn + 距最后完成 turn ≥3 分钟 + 当前失焦
- [x] 后台生成：失焦期间预生成，回焦只展示已经生成的 recap
- [x] 去重：成功 recap 后没有新 user turn时不再自动生成
- [x] manual `/recap`：有 user turn即可随时生成
- [x] 输入预算：不发送完整会话，限制最近会话切片
- [x] 输出语言：macOS `AppleLanguages` 优先，locale 环境变量回退
- [x] `cargo test -p pi-grok-adapter system_language` / `preferred_language` PASS
- [x] `cargo test -p xai-grok-pager --lib app::dispatch::tests::notes --no-default-features` PASS（9）
- [x] `cargo test -p xai-grok-pager --lib notifications::focus --no-default-features` PASS（17）
- [x] `cargo test -p xai-grok-pager --lib notifications::config --no-default-features` PASS（10）
- [x] `cargo check -p xai-grok-pager-bin --bin grok-pi` PASS
- [x] `cargo build -p xai-grok-pager-bin --bin grok-pi` PASS
- [x] `cargo test -p xai-grok-pager-bin --bin grok-pi recap_extension` PASS
- [x] 修复 external 路径 `session_recap_available` 默认 false 导致 `/recap` 隐藏
- [x] `recap_model` F2 入口复用原生 `/model` picker，并将选择定向持久化为 recap model
- [ ] 手测：冷启动可见 `/recap`、F2 关 auto、换模型、中文 locale

## 非目标

- 不实现 Grok shell 的 recap artifact 落盘
- 不改 Pi RPC 协议
