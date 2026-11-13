# Welcome 启动页 Pi 对齐与菜单修复

## 目标

修复 `grok-pi` 启动 Welcome 页：

1. π logo 行宽不一致导致按行居中错位
2. Resume session / Ctrl+S 应等价 `/resume`（Pi catalog + native SessionPicker）
3. 暂时隐藏 New worktree
4. Changelog 打开项目 GitHub 页面（无则新建 `CHANGELOG.MD`）

## 实现

| 项 | 改动 |
|---|---|
| Logo | `logo.rs` 渲染前将各行 pad 到统一视觉宽度；`PI_LOGO` 对齐 Pi 官方 `SETUP_LOGO_LINES` |
| Resume | external：`ShowSessionPicker`；Welcome 无 agent 时投影 catalog 到 welcome picker；event_loop 将 `FetchSessionList` remap 到 `ShowSessionPicker` |
| New worktree | `ExternalUiProfile.hide_new_worktree` + process-wide `ExternalWelcomeMenu` |
| Changelog | `ExternalUiProfile.changelog_url` → `Action::OpenUrl`；新建 `CHANGELOG.MD` |

## 验证

```bash
cargo check -p xai-grok-pager-bin --bin grok-pi
cargo test -p xai-grok-pager logo --lib
```

## 接缝

- `allowedModifiedFiles` 增加 `views/welcome/mod.rs`
- `native_renderer_sha256.json` 同步 logo.rs / mod.rs hash

## Session 预热（同日补）

进入 Welcome 即 `dispatch_prewarm_welcome_session`：后台 CreateSession，UI 仍留在 Welcome。首字/Enter 走 `NewSession` 时 attach 预热 agent，不再冷等 RPC。Resume 会 discard 预热；`/home` 退出后再预热。
