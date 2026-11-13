# Pi welcome logo 适配

## 目标

`grok-pi` 启动时的 welcome / minimal 欢迎卡使用 Pi 的 block-character π logo，而不是 Grok 默认 braille logo。

## 边界

| 层 | 职责 |
|---|---|
| Grok welcome logo renderer | 继续拥有布局、高度分级、shimmer、legacy hide |
| `ExternalUiProfile.logo` | 仅提供 full/small 静态 art |
| `grok-pi` composition | 注入 π art；不新建 TUI |

不改 dashboard 多 agent 产品语义；不在 adapter 内渲染。

## 实现

1. `ExternalLogoArt { full, small }` + `ExternalUiProfile.logo`
2. `views/welcome/logo.rs` 进程级 override（镜像 slash command profile 模式）
3. `event_loop` 在 External profile 时 `set_logo_override`
4. `grok-pi` 提供：

```text
  ██████
  ██  ██
  ████  ██
  ██    ██
```

## 验证

```bash
cargo test -p xai-grok-pager logo --lib
cargo check -p xai-grok-pager-bin --bin grok-pi
```

## 接缝元数据

- `allowedModifiedFiles` 增加 `views/welcome/logo.rs`
- `native_renderer_sha256.json` 同步该文件 hash

## 启动语义（2026-07-17 补）

根因：`run_external` 曾写死 `MaterializedStartup::Resume`，永远跳过 Welcome。

现改为：

- 默认 `NewAuto` → Welcome + π logo
- 仅 `resume_existing_session`（`grok-pi -c/--continue`）→ 立即 Resume

## 状态

- [x] logo override seam
- [x] ExternalUiProfile + event_loop + grok-pi
- [x] 默认 Welcome 启动
- [x] verifier / FEATURE_MATRIX
