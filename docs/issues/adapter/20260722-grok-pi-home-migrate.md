# grok-pi 使用 `~/.grok-pi` 并提供 migrate-home

**状态:** done  
**日期:** 2026-07-22  
**范围:** `xai-grok-pager-bin` / `grok-pi` 组合入口 only（不改上游 `default_grok_home()`）

## 问题

`grok-pi` 与 stock Grok CLI 共用 `~/.grok`，污染对方配置与缓存。

## 方案

1. 启动最早点：若未设 `GROK_HOME`，注入 `~/.grok-pi`（赶在 `grok_home()` OnceLock 之前）。
2. 子命令 `grok-pi migrate-home`：从 `~/.grok`（或 `$GROK_LEGACY_HOME`）**拷贝** allowlist 到目标 home。
3. 首次启动空 home 时 safe auto-migrate 一次，写 `.migrated-from-legacy` marker。

## Allowlist

`pager.toml`, `config.toml`, `trusted_folders.toml`, `slash-mru.json`, `tip_cursor.json`, `skills/`, `hooks/`, `projects/`；可选 `--include-auth` → `auth.json`。

**不迁移:** `sessions/`（Grok 会话 ≠ Pi `~/.pi`）、`bin/`、`downloads/`、`marketplace-cache/`、`bundled/`、`worktrees.db`、`pi-file-rollback/`。

## CLI

```text
grok-pi migrate-home
grok-pi migrate-home --status
grok-pi migrate-home --dry-run
grok-pi migrate-home --force
grok-pi migrate-home --from DIR --into DIR
grok-pi migrate-home --include-auth
```

## 验证

- `cargo test -p xai-grok-pager-bin --bin grok-pi -- home:: migrate_home::`
- `cargo check -p xai-grok-pager-bin --bin grok-pi`
