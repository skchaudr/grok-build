---
id: "2026-07-18-恢复-grok-pi-timeline-rail"
title: "恢复 grok-pi Timeline rail"
status: "in_progress"
created: "2026-07-18"
updated: "2026-07-18"
category: "Pager"
tags: ["pager", "timeline", "grok-pi", "upstream-reapply"]
---

# Issue: 恢复 grok-pi Timeline rail

## Goal

将 upstream Grok Pager 的 Timeline rail 以窄范围方式恢复到 `grok-pi`：在满足原生条件的多轮会话中显示每 turn tick、支持 hover 预览与点击/箭头跳转，并保持 `pi-grok-adapter` headless。

## 背景/问题

`a8906d9`（Pi 集成初始提交）删除了 Timeline 的 renderer、scrollback 导航、slash 命令和 `AgentView` 交互路径。`5bd1312` 仅恢复了 `show_timeline` 的配置/设置接缝。因此现状是设置可开启、可持久化，却始终渲染普通 scrollbar。

当前工作树已有与本任务无关的 Remote TUI / Pi Bash 未提交改动；恢复只能编辑 Timeline 直接需要的 Pager 文件，不覆盖这些改动。

## 验收标准 (Acceptance Criteria)

- [ ] WHEN `show_timeline=true`、非 subagent、终端宽度至少 60 且 scrollback 至少两轮，系统 SHALL 用 Timeline rail 替代普通 scrollbar。
- [ ] WHERE Timeline rail 显示，系统 SHALL 支持 tick hover 预览、tick 点击和上下箭头按 turn 跳转。
- [ ] IF rail 不满足显示条件、处于 subagent view 或回退布局，THEN 系统 SHALL 继续渲染原普通 scrollbar。
- [ ] `grok-pi` SHALL 继续通过 `run_external` 使用 Pager；adapter 不新增 UI、terminal 或输入处理依赖。
- [ ] 相关 Pager 测试与 `cargo check -p xai-grok-pager-bin --bin grok-pi` SHALL 通过，或单独记录既有基础设施 blocker。

## 实施阶段

### Phase 1: 规划和准备
- [x] 分析需求和依赖：确认当前缺失 `views/timeline.rs`、`scrollback/state/timeline.rs`、`slash/commands/timeline.rs` 与 `app/agent_view/jump.rs`。
- [x] 设计技术方案：从 `upstream/main` 窄范围 reapply，不改 Pi RPC 或 adapter。
- [x] 确定实施计划：先恢复 upstream 代码及模块接缝，再增加 grok-pi 回归测试并验证。

### Phase 2: 执行
- [ ] 恢复 Timeline / jump 源模块与模块声明。
- [ ] 恢复 `AgentView` layout、render、鼠标交互与 scrollback timeline 接缝，并保留当前 external widget 改动。
- [ ] 恢复 `/timeline`、`/jump` 命令注册与外部 profile 的原生命令可见性。
- [ ] 添加/恢复以 external `grok-pi` 配置验证的最小回归测试。

### Phase 3: 验证
- [ ] 运行 Timeline / jump 定向测试。
- [ ] 运行 `cargo test -p xai-grok-pager-bin --bin grok-pi` 与 `cargo check -p xai-grok-pager-bin --bin grok-pi`。
- [ ] 审查 diff：仅含 Timeline 直接所需文件和本 Issue。

### Phase 4: 交付
- [ ] 更新本 Issue 的验证结果与已知限制。
- [ ] 创建 PR 记录（如准备提交）。

## 关键决策

| 决策 | 理由 |
|------|------|
| 以 upstream Pager 实现为唯一功能事实源 | Timeline 是 Pager 原生能力，避免在 adapter 重造 UI 或语义。 |
| 暂不修改 Pi ACP `_meta` 映射 | 当前 renderer/交互完整缺失是确定阻断；先恢复并验证本地 scrollback turn 语义。若 replay/跨视图测试失败，再独立最小补充元数据。 |
| 上游源码为优先级最高的实现事实 | Timeline 模块、glyph、测试和 layout 接缝应逐段复用 `upstream/main`；仅在当前 external widget 接缝冲突处手工合并，避免未来上游同步扩大 diff。 |
| 保留 local external widget 代码 | 工作树已有未提交 Remote TUI 改动；Timeline 接缝必须以手工合并方式避开它。 |

## 遇到的错误

| 日期 | 错误 | 解决方案 |
|------|------|---------|
| 2026-07-18 | `git diff --check` 报 `extensions/pi-grok-remote-tui/index.ts` EOF 空行 | 该改动早于本任务且无关；不修改，验证时单独声明。 |

## 相关资源

- [x] 上游实现：`upstream/main:crates/codegen/xai-grok-pager/src/views/timeline.rs`
- [x] 删除来源：`a8906d9`，删除 4 个 Timeline / jump 模块共 1210 行。
- [x] 配置恢复：`5bd1312`，只恢复 `show_timeline` 设置接缝。
- [x] 架构边界：`NATIVE_GROK_TUI_ALIGNMENT.md`
- [x] 功能矩阵：`FEATURE_MATRIX.md`

## Notes

本功能的原生显示条件为：`show_timeline` 开启、非 subagent view、宽度 >= 60、`turn_count >= 2`。Timeline 只消费 Pager 本地 `ScrollbackState`；Pi ACP 的 `promptId` / `eventId` / `isReplay` 元数据缺口属于恢复后的 replay/跨视图风险，而非现有“完全不显示”的第一阻断。

---

## Status 更新日志

- **[2026-07-18]**: 状态变更 → `in_progress`，备注: 已完成源码根因分析，开始从 upstream 窄范围恢复 Timeline Pager 接缝。
