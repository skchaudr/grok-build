---
id: "2026-07-18-修复-grok-pi-Welcome-品牌文本"
title: "修复 grok-pi Welcome 品牌文本"
status: "in_progress"
created: "2026-07-18"
updated: "2026-07-18"
category: "pager"
tags: ["workhub", "welcome", "branding", "version"]
---

# Issue: 修复 grok-pi Welcome 品牌文本

## Goal

让 `grok-pi` 的原生 Pager Welcome hero card 显示 `grok-pi`、其产品版本和 Pi 描述，不再显示上游 Grok Build Beta、`0.2.102` 或 Grok feedback 文案。

## 背景/问题

用户截图显示 Pi block logo、Resume/Changelog/Quit 菜单已经来自 external profile，但 hero card 的标题、版本和副标题仍由 Pager 的 Grok 默认常量渲染：`Grok Build Beta 0.2.102` 与 `Thanks for trying Grok Build...`。因此 logo/menu policy 与文字品牌策略不完整。

## 验收标准

- [x] external profile 可声明 welcome `title`、`subtitle`、`version`。
- [x] `grok-pi` 声明 `grok-pi`、`GROK_PI_VERSION` 和 Pi 产品说明。
- [x] Pager 的 hero title/version 和 subtitle 消费 external brand override；Grok profile 保持原默认值。
- [x] `cargo check -p xai-grok-pager-bin --bin grok-pi` 通过。
- [x] 两个定点回归测试通过：brand state round-trip 与 hero inline override。
- [x] `GROK_VERSION=9.9.9 target/debug/grok-pi --version` 仍输出 `grok-pi 0.0.4-dirty`。

## 关键决策

| 决策 | 理由 |
|------|------|
| 在 `ExternalUiProfile` 扩充 copy-only brand contract | 保持 Pager 负责原生渲染，composition root 只供给品牌数据。 |
| 不改 `xai-grok-version` | 上游 Grok 的版本仍用于其自身协议与产品路径，Pi 只覆盖自己的 Welcome 文案。 |
| 复用现有 process-wide external profile override 模式 | 与 logo/menu override 一致，无额外 UI 或 adapter 可见职责。 |

## 验证记录

- `cargo check -p xai-grok-pager-bin --bin grok-pi`：PASS。
- `cargo test -p xai-grok-pager --lib hero_inline_brand_uses_external_override -- --nocapture`：PASS（1 passed）。
- `cargo test -p xai-grok-pager --lib welcome_brand_override_round_trips -- --nocapture`：PASS（1 passed）。
- `rustfmt --edition 2024`（修改文件）与 `git diff --check`：PASS。

## Status 更新日志

- **[2026-07-18]**: 状态变更 → `in_progress`，定位 HeroInline 与 hero subtitle 未消费 external profile；实现窄品牌 override 并完成定点验证。
