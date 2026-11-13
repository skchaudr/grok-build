# Pi Dashboard 完整适配

## 目标

`grok-pi` 使用 Grok **原生 Agent Dashboard**（`/dashboard` · Ctrl+\ · 列表/peek/dispatch），idle 会话来自 Pi session store，而不是 Grok `x.ai/session/list` 或 leader FleetView。

## 边界

| 能力 | 归属 |
|---|---|
| 渲染 / 键位 / peek / attach / dispatch | Grok Pager 原生 |
| 活跃会话 | 本进程 `app.agents` |
| 休眠/历史会话 | Pi `pi/session/list` → `pi/ui/session_catalog` → dormant roster |
| Leader multi-host roster | 边界（Grok 产品） |
| 第二套 dashboard UI | 禁止 |

## 实现

1. `PI_GROK_NATIVE_COMMANDS` 增加 `dashboard`
2. `dispatch_open_dashboard`：`external_agent` 时 `FetchExternalSessionCatalog`（cwd scope）
3. `set_external_session_catalog`：dashboard 打开且 `scope=current` 时写入 `dashboard_local_sessions`
4. event-loop roster poll：external 路径轮询 Pi catalog
5. catalog 请求失败 → `ExternalSessionCatalogFailed` 清 loading
6. attach dormant 行仍走原生 `LoadSession` → Pi `switch_session`

## 验证

```bash
cargo check -p xai-grok-pager
cargo check -p xai-grok-pager-bin --bin grok-pi
cargo test -p pi-grok-adapter
python3 crates/codegen/pi-grok-adapter/scripts/verify_native_grok.py --workspace . --pi-source pi-main
```

## 接缝

- `dispatch/dashboard.rs` 加入 allowed seams
- 既有：`event_loop` / `app_view` / `effects` / `task_result` / `actions` / `grok-pi`

## 状态

- [x] slash 暴露
- [x] open + poll 走 Pi catalog
- [x] catalog → dormant roster
- [x] 失败清 loading
- [x] FEATURE_MATRIX / verifier
