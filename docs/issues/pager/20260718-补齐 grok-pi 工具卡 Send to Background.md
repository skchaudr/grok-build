---
id: "2026-07-18-grok-pi-tool-send-background"
title: "补齐 grok-pi 工具卡 Send to Background"
status: "in_progress"
created: "2026-07-18"
updated: "2026-07-18"
category: "pager"
tags: ["grok-pi", "pager", "bash", "background-tasks", "tool-card"]
---

# Issue: 补齐 grok-pi 工具卡 Send to Background

## Goal

让 `grok-pi` 中已开始运行的前台 Pi Bash 工具卡可复用 Pager 原生 **Send to Background** 操作，并保持 Pi 为 Bash 执行与任务语义的唯一所有者。

## 背景/问题

已有 `docs/issues/adapter/20260718-grok-pi Pi Bash 后台任务桥接.md` 已实现模型在调用前传入 `is_background: true` 时的私有 extension 与任务生命周期投影。用户实测前台 `bash` 工具卡未出现或无法生效 Send to Background；该交互路径未被该 Issue 覆盖。

本任务只补齐 Pager 原生工具运行控制到 Pi 的窄接缝。不得让 `pi-grok-adapter` 拥有 terminal、PTY 或独立任务调度；不得改 Pi 源码扩展 RPC。

## 验收标准 (Acceptance Criteria)

- [ ] WHEN `grok-pi` 中前台 Bash 工具仍在运行，原生工具卡 SHALL 展示可用的 Send to Background 操作。
- [ ] WHEN 用户触发该操作，系统 SHALL 通过 Pi 现有语义取消或转交原前台执行，并创建可追踪的后台任务；不得重复执行命令。
- [ ] WHEN 后台任务运行、完成、失败或被终止，Pager SHALL 使用既有原生任务卡与任务面板更新状态和输出。
- [ ] WHERE 非 `grok-pi` profile 或非 Bash 工具，系统 SHALL 保持上游既有行为。
- [ ] 实现 SHALL 保持 Pager 为唯一 TUI、Pi 为唯一 agent core、adapter headless library-only，且不改 Pi 源码扩 RPC。

## 实施阶段

### Phase 1: 规划和准备
- [x] 检索既有后台 Bash Issue、功能矩阵与历史会话。
- [x] 追踪 Pager 原生 Send to Background 的 action/effect/protocol，及 Pi Bash 的可用控制语义。
- [x] 确定无重复执行的最小映射方案：经用户授权，私有 extension 托管前台 child，同时复用 Pi `createBashToolDefinition` 的输出/渲染语义。

### Phase 2: 执行
- [x] 在 adapter 的既有 ACP ExtRequest 接缝接入 `x.ai/terminal/background`，并验证 `terminalId` 对应 live foreground Bash。
- [x] 由 extension 托管前台/后台 child，以进程专属临时 control metadata 按 `toolCallId` 转交同一进程；复用现有 `x.ai/task_*` lifecycle 投影。
- [x] 为 adapter 控制文件写入、bridge tool result 与 extension source injection 添加/更新聚焦测试。

### Phase 3: 验证
- [x] `bun --check` 与 Bun foreground→background→completion 冒烟验证通过。
- [ ] 运行 `cargo test -p pi-grok-adapter`、目标 Pager/bin 测试与 `cargo check -p xai-grok-pager-bin --bin grok-pi`：被既有 compaction 编译错误阻断。
- [x] 审查实现差异范围与既有未提交改动隔离；未覆盖或格式化无关代码。

## 关键决策

| 决策 | 理由 |
|------|------|
| 复用 Pager 原生任务 UI 与生命周期 | Pager 是唯一 TUI；已有 `x.ai/task_*` 承载面，避免再造工具卡或任务面板。 |
| Pi extension 持有 Bash child | extension 在 Pi 进程内保有前台 child，才能将同一进程转后台；adapter 只验证和投递控制事件。 |
| 进程专属 metadata 控制通道 | composition binary 创建并传入 Pi/adapter 的临时 metadata 路径，避免多个 `grok-pi` 进程互相串扰。 |
| 不改 Pi RPC | 不扩展 Pi RPC；通过受控 extension 控制通道保持升级路径与协议边界。 |

## 遇到的错误

| 日期 | 错误 | 解决方案 |
|------|------|---------|
| 2026-07-18 | `rg` 在此环境实际解析为不支持递归/`--glob` 的兼容命令。 | 改用 `fd` 定位文件，并由只读子代理追踪调用链；不依赖该参数。 |
| 2026-07-18 | 全量 Rust 测试在既有 `pi_adapter.rs` compaction 改动处失败：返回 `()` 的 async 函数使用 `?`。 | 不修改无关改动；创建独立 blocker，待其修复后重跑全量验证。 |

## 相关资源

- `NATIVE_GROK_TUI_ALIGNMENT.md`
- `FEATURE_MATRIX.md`
- `docs/issues/adapter/20260718-grok-pi Pi Bash 后台任务桥接.md`
- `crates/codegen/xai-grok-pager/src/app/acp_handler/background.rs`
- `crates/codegen/pi-grok-adapter/src/background_bash_bridge.rs`
- `crates/codegen/xai-grok-pager-bin/src/bin/grok_pi/bash_extension.rs`

## Notes

- 已知工作树含 31 个修改文件和 8 个未跟踪文件；本任务只能增量修改确认的接缝文件，不能覆盖或整理现有改动。
- Pager 已完整实现 `Action::DemoteToBackground` → `Effect::DemoteToBackground` → `x.ai/terminal/background { sessionId, terminalId: toolCallId }`，并在收到 `x.ai/task_backgrounded` 后原位替换 Execute 卡为 BgTask。
- `pi-grok-adapter` 现处理 `x.ai/terminal/background`：只接受 extension metadata 中声明为 live foreground 的 `toolCallId`，并将控制事件追加到该 process 的 control 文件。
- `pi-grok-bash` 现在托管前台与后台 `ChildProcess`，但仍以 Pi `createBashToolDefinition` 包装前台 output accumulation、stream、取消、超时、非零退出与截断显示；转后台时 native wrapper 结束，Pager 将原 Execute 卡原位迁移为 BgTask，child 不重启。
- Bun 冒烟已验证 foreground → metadata publication → control event → background result → completion custom message，且输出完整保留。全量 Cargo 验证等待无关 compaction 编译 blocker 解除。

---

## Status 更新日志

- **[2026-07-18]**: 状态变更 → in_progress，备注: 已确认既有 `is_background` 启动路径与工具卡交互路径的范围差异，开始协议/调用链定位。
- **[2026-07-18]**: Phase 1 完成，备注: Pager 原生 demotion 已就绪，但 Pi 侧缺少接管运行中前台 Bash 的官方控制 API；等待架构选择后再进入实现。
- **[2026-07-18]**: Phase 2 完成，备注: 用户授权 extension 托管前台 child；已接入受控 metadata 通道、adapter ExtRequest 与既有 BgTask lifecycle。
- **[2026-07-18]**: Phase 3 部分完成，备注: Bun 语法/生命周期 smoke 通过；全量 Rust 验证被无关 compaction 编译错误阻断。