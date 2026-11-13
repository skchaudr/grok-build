---
id: "2026-07-18-修复 Pi ask_user_question QuestionView rows 崩溃"
title: "修复 Pi ask_user_question QuestionView rows 崩溃"
status: "done"
created: "2026-07-18"
updated: "2026-07-18"
category: "adapter"
tags: ["workhub", "修复 Pi ask_user_question QuestionView rows 崩溃"]
---

# Issue: 修复 Pi ask_user_question QuestionView rows 崩溃

## Goal

修复 grok-pi 下调用 ask_user_question 时 Remote TUI 的 `rows` 未定义崩溃，保留 Pager 作为唯一终端渲染面。

## 背景/问题

用户在 grok-pi 中调用 `ask_user_question` 后得到 `Cannot read properties of undefined (reading 'rows')`。该工具属于 rpiv-ask 的 `ctx.ui.custom(factory)` 路径，现由实验性 Remote TUI extension 在 Pi 进程内托管后投影到原生 Pager。

## 验收标准 (Acceptance Criteria)

- [x] WHEN rpiv `ask_user_question` 调用 `ctx.ui.custom`，系统 SHALL 不再抛出 `rows` 未定义。
- [x] WHERE Remote TUI host 接管 custom factory，系统 SHALL 按插件所需的 TUI/theme/keybinding 契约初始化组件。
- [x] IF 用户完成或取消问卷，THEN Pi tool SHALL 收到对应结果而 Pager 仍是唯一可见 TUI。

## 实施阶段

### Phase 1: 规划和准备
- [x] 分析需求和依赖
- [x] 建立最小可复现测试
- [x] 确定最小修复方案

### Phase 2: 执行
- [x] 为 Remote TUI stub 补充 `terminal.columns` 与 `terminal.rows`
- [x] 新增 host 终端尺寸契约回归测试

### Phase 3: 验证
- [x] `bun test extensions/pi-grok-remote-tui/index.test.ts`
- [x] 用安装的 rpiv `QuestionnaireSession` 构造并渲染真实问卷
- [x] `cargo test -p pi-grok-adapter pi_input_and_editor_prefer_native_freeform_annotations`

### Phase 4: 交付
- [x] 更新 Issue
- [ ] 创建 PR（未请求）
- [ ] 合并主分支（未请求）

## 关键决策

| 决策 | 理由 |
|------|------|
| 在 `extensions/pi-grok-remote-tui/index.ts` 修复 | 异常发生在 `ctx.ui.custom(factory)` 的 Pi 进程内 host；adapter/Pager ACP QuestionView 不在该调用链。 |
| 仅模拟 terminal 尺寸 | rpiv 仅在此路径读取 `tui.terminal.columns/rows`；不伪造完整 Pi `Terminal`，避免扩大实验接缝。 |

## 遇到的错误

| 日期 | 错误 | 解决方案 |
|------|------|---------|
| 2026-07-18 | `Cannot read properties of undefined (reading 'rows')` | Remote TUI `tuiStub` 缺少 `terminal`；补齐 `{ columns, rows }`。 |
| 2026-07-18 | 新测试初次加载不到 `@earendil-works/pi-tui` | 测试通过 Bun module mock 隔离依赖，避免新增 node_modules 链接。 |

## 相关资源

- [x] `FEATURE_MATRIX.md`：rpiv ask 为实验 Remote TUI 路径
- [x] `extensions/pi-grok-remote-tui/index.ts`
- [x] `/Users/dengwenyu/.pi/agent/npm/node_modules/@juicesharp/rpiv-ask-user-question/state/build-questionnaire.ts`

## Notes

根因：rpiv 的 `QuestionnaireSession` 在 render 期间读取 `tui.terminal.rows`，而 Remote TUI host 的 stub 没有 `terminal`。修复后使用该已安装插件的真实 `QuestionnaireSession` 构造和 render 验证；未执行交互式 grok-pi 端到端点击测试。

---

## Status 更新日志

- **2026-07-18**: 状态变更 → `in_progress`，备注: 已确认故障属于 rpiv custom factory → Remote TUI 投影链路；开始建立回归测试。
- **2026-07-18**: 状态变更 → `done`，备注: Remote TUI stub 已补齐 terminal 尺寸，回归测试与真实问卷 render 验证通过。