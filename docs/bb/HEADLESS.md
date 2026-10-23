# bb server, headless on sab-air
- What: `bb-app.js` (server + host daemon) via launchd agent `com.sab.bb-server`, node `/opt/homebrew/bin/node`, data dir `~/.bb` (same `bb.db`).
- Bind: `127.0.0.1:38886` only (API is unauthenticated, allows command execution). Set via `--server-bind-host`; `~/.bb/env.json` still says 0.0.0.0 and the flag overrides it.
- Plist: `~/Library/LaunchAgents/com.sab.bb-server.plist` (copy: `docs/bb/bb-server-air.plist`). Log: `~/Library/Logs/bb-server.log`.
- Restart: `launchctl kickstart -k gui/$(id -u)/com.sab.bb-server`
- Stop: `launchctl bootout gui/$(id -u)/com.sab.bb-server` (start again: `launchctl bootstrap gui/$(id -u) <plist>`).
- Status: `launchctl print gui/$(id -u)/com.sab.bb-server`
- Collision: bb.app uses the same port and `daemon.lock`; quit it, or boot the agent out first, before opening it.
- Mini reach: `bb-air-proxy.js` forwards `192.168.1.186:38886` -> `100.93.242.91:38886`, which does not reach a loopback-only server. Use an ssh tunnel (`ssh -L`) instead.
