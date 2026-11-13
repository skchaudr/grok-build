---
title: context弹窗 cache graph（pi-cache-graph 对齐）
status: in_progress
date: 2026-07-22
tags: ["workhub", "context", "cache", "adapter", "pager"]
---

# context 弹窗 cache graph（pi-cache-graph 对齐）

## Goal

在 grok-pi 原生 **Context 弹窗**（`/context` / context bar 点击）内对齐 [pi-cache-graph](https://www.npmjs.com/package/pi-cache-graph) 的观测能力：

| Key | View |
|-----|------|
| `0` | 现有 Context 分项（默认） |
| `1` | Per-turn cache hit % |
| `2` | Cumulative hit % |
| `3` | Cumulative token volumes |
| `s` | Stats 表 |
| `e` | 导出 CSV 到 cwd |
| `r` | 刷新 metrics（保留当前 view） |

- 切换 1/2/3/s 时 **数据保留**（同一 snapshot，仅 re-render）
- **默认开启**；F2 `[ui].pi_cache_graph` 可关（`external_only`，无需 restart）
- **禁止** `ctx.ui.custom` / 第二套 TUI；禁止改 Pi 源码

## Architecture

```
Pi get_entries → adapter cache_metrics 投影 → SessionInfoResponse.cacheMetrics
                                                    ↓
Pager ActiveModal::ContextInfo { metrics, view } → 原生 ModalWindow 渲染
```

| Layer | Role |
|-------|------|
| Pi RPC | `get_entries` 权威 session tree + usage |
| `pi-grok-adapter` | 纯投影（hit% 公式与 pi-cache-graph 一致） |
| shell `acp_types` | 可选 wire 类型 `cacheMetrics` |
| Pager | 多视图 Modal + F2 门禁 + CSV export |

公式：`cacheHit% = cacheRead / (input + cacheRead + cacheWrite) * 100`

Active branch：从 `leafId` 沿 `parentId` 回溯 entry id 集合。

## Acceptance

- [x] F2 `pi_cache_graph` default **true**，`external_only`，关后无 1/2/3/s/e 快捷键
- [x] Context 弹窗 1/2/3 切换图，s stats，e 写 `{session}.csv` 到 cwd，toast 路径
- [x] 视图切换不重新请求；`r` 重请求且保留 view
- [x] adapter 单测覆盖 hit% / branch / empty
- [x] `cargo test -p pi-grok-adapter` + `cargo check -p xai-grok-pager-bin --bin grok-pi`

## Non-goals

- 注入 npm `pi-cache-graph` 插件本身
- 在 adapter 里画 TUI
- 修改 Pi core

## Status

- **2026-07-22**: done — adapter metrics + Context modal multi-view + F2
- 验证：`cargo test -p pi-grok-adapter` 106 passed；`cargo check -p xai-grok-pager-bin --bin grok-pi` ok
- 已知：`xai-grok-pager` lib 测试预存 `billing_surface_visible` 编译阻塞（与本改无关）
- **2026-07-22 fix**: resume 全 0 — 根因是部分 provider（如 `3838/qwen3.8max`）在 session 里写入 `usage: all 0`；content 估算回退 + 警告条；数字解析兼容 f64/string
