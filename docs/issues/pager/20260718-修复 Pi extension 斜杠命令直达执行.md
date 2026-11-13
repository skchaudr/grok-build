---
id: "2026-07-18-修复-pi-extension-斜杠命令直达执行"
title: "修复 Pi extension 斜杠命令直达执行"
status: "in_progress"
created: "2026-07-18"
updated: "2026-07-18"
category: "pager"
tags: ["workhub", "pager", "adapter", "slash", "queue"]
---

# Issue: 修复 Pi extension 斜杠命令直达执行

## Goal

让 Pi `pi.registerCommand()` 注册的动态 extension slash command 在 Grok Pager 中立即交给 Pi 执行，不进入 Pager 本地 prompt 队列或 Pi steer/follow-up 队列。

## 背景/问题

Pi RPC `get_commands` 为动态命令提供 `source: extension | prompt | skill`。adapter 当前将这些来源统一投影为 ACP `AvailableCommand` 且丢弃 source；Pager 因而把所有非 skill ACP command 作为普通 prompt passthrough，运行中的 agent 会使其先进入 Pager 本地 `pending_prompts`。

Pi `AgentSession.prompt()` 的真实顺序恰好相反：已注册 extension command 在 streaming 判断前立即运行；prompt template、skill 和未知 slash 则保留普通 prompt 语义。

## 验收标准 (Acceptance Criteria)

- [ ] WHEN Pi catalog 中 `source="extension"` 的 slash command 在运行中被提交，系统 SHALL 直接交给 Pi command handler，且 Pager `pending_prompts` 与 Queue Pane 不出现该命令。
- [ ] WHERE Pi catalog 中 `source="prompt"`、`source="skill"` 或用户输入未知 `/foo`，系统 SHALL 保持现有普通 prompt / queue 行为。
- [ ] IF extension handler 触发 agent 工作或 UI 通知，THEN Pi 仍是 lifecycle 与队列语义的唯一所有者，Pager 只通过既有 ACP event 投影。
- [ ] IF direct command RPC 失败，THEN Pager SHALL 保留现有 fire-and-forget ACP notification 的错误日志行为，不伪造 turn completion。

## 实施阶段

### Phase 1: 规划和准备
- [x] 分析 Pi command source、RPC prompt 和 AgentSession 直接执行语义。
- [x] 定位 Pager passthrough → 本地队列路径及 adapter source 丢失点。
- [x] 确定最小契约：仅 `extension` 使用私有 ACP metadata 与 fire-and-forget `pi/extension_command` notification。

### Phase 2: 执行
- [x] adapter 在 `AvailableCommand.meta` 保留 `piCommandSource`。
- [x] Pager 将 extension metadata 分类为 direct command，并发出不占 turn slot 的 effect。
- [x] adapter 接收 direct notification，调用既有 Pi RPC `prompt`；不创建 active prompt 或 queue mirror reservation。
- [x] 将新增 Pager slash/dispatch seams 显式加入 source-identity verifier 的 allowlist。

### Phase 3: 验证
- [x] 单元测试：extension metadata 产生 direct result；prompt/skill/unknown 保持 passthrough/skill 语义。
- [x] 单元测试：运行中提交 direct command 不增加 Pager 本地队列且产生 direct effect。
- [x] 单元测试：adapter catalog source metadata。
- [x] `cargo test -p pi-grok-adapter`（70 tests）与 JSON/Python manifest 检查通过；已运行定向 Pager 测试、binary check、source-identity verifier 并记录既有工作树 blocker；已审查本 Issue diff。

### Phase 4: 交付
- [x] 更新 FEATURE_MATRIX 与本 Issue 的真实状态。
- [ ] 不创建 commit 或 PR，除非用户另行要求。

## 关键决策

| 决策 | 理由 |
|------|------|
| 仅 `source=extension` 绕过队列 | 这是 Pi 真实的 command handler 语义；template、skill 仍是 agent prompt。 |
| 使用私有 ACP metadata | `AvailableCommand` 现有 metadata 足以保留 source，不改 Pi RPC。 |
| 使用 fire-and-forget ext notification | command handler 本身不必拥有 ACP prompt turn；避免 active prompt、queue mirror 和虚假 PromptResponse。 |
| 不改 Pi 源码 | Pi 继续拥有 command dispatch、agent lifecycle、session 与 queue。 |
| 显式更新 verifier seam metadata | 新增 Pager 逻辑属于必要的 slash/dispatch integration seam，不能偷偷放宽验证器。 |

## 遇到的错误

| 日期 | 错误 | 解决方案 |
|------|------|---------|
| 2026-07-18 | 当前工作树已有其他会话的 30+ 修改与未跟踪文件。 | 仅修改本 Issue 声明的文件，不格式化、恢复或整理无关改动。 |

## 相关资源

- `pi-main/packages/coding-agent/src/core/agent-session.ts`
- `crates/codegen/pi-grok-adapter/src/pi_adapter.rs`
- `crates/codegen/xai-grok-pager/src/slash/acp_command.rs`
- `crates/codegen/xai-grok-pager/src/app/dispatch/prompt.rs`
- `crates/codegen/pi-grok-adapter/scripts/verify_native_grok.py`

## Notes

- 当前 `run_bridge_command()` 会建立 active prompt 并等待 completion，适用于导航/recap 等 bridge 操作，不可复用给通用 direct extension command。
- 现有 queue mirror 只镜像 Pi steering/follow-up；direct command 不得 reserve 或 rebroadcast一个虚假的 queue row。

---

## Status 更新日志

- **[2026-07-18]**: 状态变更 → in_progress，备注: 已确认 source 丢失导致 Pager 将 Pi extension command 当普通 prompt 本地排队。
- **[2026-07-18]**: Phase 2 完成，备注: `source=extension` 通过 `piCommandSource` metadata 投影为 Pager direct effect，再以 `pi/extension_command` 通知调用 Pi RPC；不占 turn slot、不写本地/镜像队列。
- **[2026-07-18]**: Phase 3 完成，备注: `cargo test -p pi-grok-adapter` 通过（70）；Pager 定向测试仍被既有 settings/paste/resource/AppView fixture 编译错误阻断；binary check 被无关 `views/modal.rs` 缺逗号和 `pi_config` 类型错误阻断；verifier 的本次 seam 集合 PASS，整体仍有既有 baseline/slash/tree/completion blocker。
