---
id: "2026-07-22-upstream-workflow-pi-spawn"
title: "上游 Workflow 完整兼容：Pi Spawn 后端"
status: "done"
created: "2026-07-22"
updated: "2026-07-22"
note: "Host + slash surface landed; create-workflow is PassThrough; handtest open"
category: "架构"
tags: ["workflow", "xai-workflow", "pi-grok", "extension", "upstream"]
---

# Issue: 上游 Workflow 完整兼容（Pi Spawn 后端）

## Goal

在 **grok-pi（External ACP + Pi Core）** 上完整兼容上游 Workflow：运行 **`xai-workflow` Rhai 引擎** + shell 编排语义，仅将 **`SpawnAgent` 接到 Pi child session**；Pager 原生 `workflow_updated` / `/workflows` 零仿造 UI。

## PDCA

`work/20260722-upstream-workflow-pi-compat/`

## 架构

```text
xai-workflow (upstream engine, no fork)
    ↑ host_tx
shell workflow orchestration
    SpawnBackend pluggable
      ├─ Grok: SubagentEvent::Spawn (default, regression)
      └─ Pi: bridge → createAgentSession
    ↓ workflow_updated
Grok Pager ingest_workflow_update
```

## 约束

1. Grok Pager = 唯一 TUI；adapter headless  
2. Pi = grok-pi 唯一 Agent Core  
3. 禁止 TS 重写 Rhai 引擎  
4. 禁止改 Pi 源码扩私有 RPC；扩展走官方 API  
5. 优先复用 `xai-workflow` + shell 编排，不复制 manager 逻辑  

## 切片

| ID | 内容 | 验收 |
|----|------|------|
| S1 | `WorkflowAgentBackend` trait；Grok 默认实现 | shell workflow 单测绿 |
| S2 | External 可挂 runtime / 公开必要 API | mock backend launch 通 |
| S3 | Pi `__pi_workflow_spawn` / cancel 扩展 | 插件静态 + spawn 契约 |
| S4 | adapter 接线 + `workflow_updated` 投影 | adapter test + binary check |
| S5 | 文档 FEATURE_MATRIX + 手测清单 | PDCA 回写 |

## Acceptance

- [ ] A1 同一 `xai_workflow::run_workflow` 路径可在 Pi host 下跑 `.rhai`
- [ ] A2 registry 路径与 builtin 一致
- [ ] A3 SpawnAgent → Pi createAgentSession 返回 AgentResult
- [ ] A4 pause/stop/同进程 resume/budget 行为对齐 shell（已覆盖用例）
- [ ] A5 Pager 收到 `workflow_updated` 更新 UI
- [ ] A6 Grok 默认 Subagent 路径回归通过
- [ ] A7 FEATURE_MATRIX 声明 upstream engine + Pi spawn
- [ ] A8 adapter headless；无 TS 引擎；无 Pi 源码 RPC hack

## Residual

| 项 | 说明 |
|----|------|
| fork_context | Pi child best-effort，非 Grok parent fork 字节级一致 |
| 跨进程 resume | 与上游同：进程死后不 resume |
| `/create-workflow` | 当前为 Pager PassThrough 用户提示进 Pi，**不是**上游 SKILL.md 注入；真 skill 另开 |
| resume / save 管理 | slash 有文案；完整 host RPC 对齐仍可加强 |
| Goal driver | 另 Issue |

## Progress

- [x] 研究 + PDCA Plan + D1=split  
- [x] S1 SpawnBackend（`WorkflowAgentBackend` + Grok/Mock；`mock_backend_runs_agent_call_in_rhai` 绿）  
- [x] S2 External API（`ExternalWorkflowRuntime` + 单测）  
- [x] S3 Pi extension（`extensions/pi-grok-workflows` + grok-pi 注入）  
- [x] S4 adapter `PiWorkflowAgentBackend` 文件桥 + 单测  
- [x] S5 窄测 + FEATURE_MATRIX + grok-pi check  
- [x] **会话宿主：** `WorkflowHost` + `x.ai/workflow/{launch,pause,stop}` + `x.ai/session_notification` 投影 `workflow_updated`  
- [x] **Slash 对齐：** 注入 `/workflow` `/workflows` `/create-workflow` + 命名脚本；过滤 `__pi_workflow_spawn/cancel`；Pager 本地 `WorkflowCommand` / `CreateWorkflowCommand` + `Effect::WorkflowLaunch|Manage`  
- [x] **工具完成回传：** `workflow` 工具经 response 文件等待 host outcome，父 turn 拿报告正文  
- [x] **项目隔离：** `$GROK_PROJECT_DIR=.grok-pi`，脚本在 `<repo>/.grok-pi/workflows` / `~/.grok-pi/workflows`  
- [ ] **建议手测：** F2 开启 → 重启 → `/workflows` 可见 → `/workflow deep-research …` → `/create-workflow`  
