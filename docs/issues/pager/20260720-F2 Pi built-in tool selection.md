---
id: "2026-07-20-F2 Pi built-in tool selection"
title: "F2 Pi 内置工具选择"
status: "in_progress"
created: "2026-07-20"
updated: "2026-07-20"
category: "pager"
tags: ["pager", "pi", "settings", "tools"]
---

# Issue: F2 Pi 内置工具选择

## Goal

在 `grok-pi` 的原生 F2 设置中提供 Pi 七个内置工具的复选框，并让配置在下一次启动的 Pi 会话中生效。

## 背景/问题

Pi 默认只启用 `read`、`bash`、`edit`、`write`；`grep`、`find`、`ls` 需要使用全局 `--tools` 白名单，后者会意外排除未列出的扩展/自定义工具。需要保持 Pi 默认和已加载扩展工具不变，仅按 F2 偏好增减七个 Pi 内置工具。

## 验收标准 (Acceptance Criteria)

- [x] F2 的 Agent 类别提供 `Pi built-in tools` 原生分组，展示七个可独立勾选的工具。
- [x] 首次/未配置时，`read`、`bash`、`edit`、`write` 为开启；`grep`、`find`、`ls` 为关闭。
- [x] 设置写入 Pager 配置，并标注为下次会话生效；不改动运行中的 Pi 子进程。
- [x] `grok-pi` 将持久化选择传给内置注入的 Pi extension；该 extension 保留已启用的 extension/custom tools，仅替换七个内置工具的 active 集合。
- [ ] Pi 专属分组只在 `UiProfile::External` / grok-pi 中显示，普通 Grok F2 不出现。
- [ ] 显式 Pi `--tools`、`--no-tools`、`--no-builtin-tools` 与 `--exclude-tools` 均保持高于 F2 偏好的 CLI 优先级。
- [x] 不修改 `pi-main`；Adapter 保持 headless。

## 实施阶段

### Phase 1: 规划和准备
- [x] 分析 Pi `setActiveTools` 官方 extension API 与 CLI allowlist 约束。
- [x] 确认 F2 原生 Group 设置与配置持久化路径。
- [x] 决定下个会话生效，避免中断正在运行的 Pi 会话。

### Phase 2: 执行
- [x] 增加 UiConfig、F2 Group、Action/Setter/持久化映射。
- [x] 添加 grok-pi 内置 Pi extension 和启动期环境投影。
- [x] 添加窄范围回归测试与文档更新。

### Phase 3: 验证
- [x] 运行 grok-pi binary tests（`--no-default-features`）。
- [x] 运行 `cargo test -p pi-grok-adapter`。
- [x] 运行 `cargo check -p xai-grok-pager-bin --bin grok-pi`。
- [ ] 增加普通 Grok profile 不显示 Pi 分组的测试。
- [ ] 增加 `--no-tools`、`--no-builtin-tools`、`--exclude-tools` 不会被 F2 extension 绕过的测试。
- [x] 审查 diff；工作树已有无关改动，未修改或回退它们。

已知 blocker：`cargo test -p xai-grok-pager --lib settings::registry::tests::defaults_match_ui_config_default` 在本次改动之前即因 `views/modal.rs` 访问私有 `PickerState::query` 无法编译；与本功能无关。

Issue review 发现两个未完成项：当前 settings rows 只按 voice/kitty/minimal 过滤，没有 External profile gate；且 grok-pi 只在显式 `--tools` 时跳过该 extension，`--no-builtin-tools` 后仍可能由 `session_start` 的 `setActiveTools()` 重新启用 F2 选中的 builtin。修复并验证前不得标记 completed。

## 关键决策

| 决策 | 理由 |
|------|------|
| 通过 Pi extension 的 `setActiveTools` 应用选择 | 不改 Pi 源码，并保留 default allowlist 外已加载的扩展/custom tools。 |
| F2 改动下次会话生效 | 配置语义明确，不在现有回合中重建 Pi 的工具和系统提示。 |
| 所有工具 CLI 限制保持优先 | `--tools`、`--no-tools`、`--no-builtin-tools`、`--exclude-tools` 都不能被宿主偏好绕过。 |
| Pi 设置仅 external profile 可见 | normal Grok 不应显示无效的 Pi 专属设置。 |

## Notes

- Pi 官方 `setActiveTools()` 只激活已注册且未被 CLI allowlist 排除的工具。
- `--no-extensions` 会关闭所有注入 bridge extension，故也不会应用此偏好；这是 `grok-pi -ne` 的既有语义。
- **[2026-07-20]**: 初步实现；F2 原生复选框默认开启 read/bash/edit/write，grep/find/ls 关闭；下个 grok-pi 会话应用。
- **[2026-07-20]**: Issue review → in_progress；补齐 external-only 可见性与全部 CLI 工具限制优先级后再完成。
