# 补齐 grok-pi 高频 Pi 启动 flag

## 状态

- **状态**: completed
- **创建**: 2026-07-19
- **完成**: 2026-07-19
- **范围**: `xai-grok-pager-bin` 组合入口 CLI；不改 adapter / Pager / Pi 源码

## 背景

`grok-pi --help` 仅一等暴露部分 Pi 启动参数；模型、会话、细粒度 tools、trust/offline 只能靠 `-- <PI_ARGS>` 透传，且 help 未说明透传约定。

## 目标

- [x] 一等暴露 P0：`--provider` `--model` `--models` `--thinking`；`--session` `--session-id` `--session-dir` `--fork`；`--tools/-t` `--exclude-tools/-xt` `--no-builtin-tools/-nbt`
- [x] 一等暴露高频 P1：`--approve/-a` `--no-approve/-na` `--offline`
- [x] 短别名归一化：`-nbt` `-xt` `-na`（既有 `-ns/-nc/-ne/-nt` 保留）
- [x] `pi_session_dir` 在合并一等 `--session-dir` 后解析
- [x] 更新 `--help` after_help（透传 / examples / 边界）
- [x] 更新 README / FEATURE_MATRIX（中英）
- [x] 单元测试 + `cargo test -p xai-grok-pager-bin --bin grok-pi` + help 抽检

## 非目标

- 不暴露 `--resume` CLI（Welcome + 原生 SessionPicker）
- 不暴露 `--print` / messages / `@files` / `--list-models` / `--export` / `pi install|config`
- 不复制 Pi 环境变量长表

## 验收

1. `grok-pi --help` 可见新 flag 与透传说明 — PASS
2. 新 flag 经 `pi_args_with_startup_flags` 转发到 Pi — PASS（cli unit tests）
3. 相关 binary 单测通过 — PASS (`cargo test -p xai-grok-pager-bin --bin grok-pi -- cli::`)

## 变更

- `crates/codegen/xai-grok-pager-bin/src/bin/grok_pi/cli.rs`
- `crates/codegen/xai-grok-pager-bin/src/bin/grok-pi.rs`（session-dir 解析时机）
- `README.md` / `README.zh-CN.md` / `FEATURE_MATRIX.md` / `FEATURE_MATRIX.zh-CN.md`
