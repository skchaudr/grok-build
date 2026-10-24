# bb server, headless on sab-air
- What: `bb-app.js` (server + host daemon) via launchd agent `com.sab.bb-server`, node `/opt/homebrew/bin/node`, data dir `~/.bb` (same `bb.db`).
- Bind: `127.0.0.1:38886` only (API is unauthenticated, allows command execution). Set via `--server-bind-host`; `~/.bb/env.json` still says 0.0.0.0 and the flag overrides it.
- Plist: `~/Library/LaunchAgents/com.sab.bb-server.plist` (copy: `docs/bb/bb-server-air.plist`). Log: `~/Library/Logs/bb-server.log`.
- Restart: `launchctl kickstart -k gui/$(id -u)/com.sab.bb-server`
- Stop: `launchctl bootout gui/$(id -u)/com.sab.bb-server` (start again: `launchctl bootstrap gui/$(id -u) <plist>`).
- Status: `launchctl print gui/$(id -u)/com.sab.bb-server`
- Collision: bb.app uses the same port and `daemon.lock`; quit it, or boot the agent out first, before opening it.
- Mini reach: `bb-air-proxy.js` forwards `192.168.1.186:38886` -> `100.93.242.91:38886`, which does not reach a loopback-only server. Use an ssh tunnel (`ssh -L`) instead.

Launchd runs `~/.bb/bin/bb-server` (`docs/bb/bb-server`, execs node) with `AssociatedBundleIdentifiers=dev.bb.desktop` so Login Items shows "bb" instead of "node".

Mini tunnel: launchd `com.sab.bb-tunnel-air` runs `~/.bb/bin/bb-tunnel-air` (`docs/bb/bb-tunnel-air`, ssh `-L 127.0.0.1:38886:127.0.0.1:38886 sab-air`; plist copy `docs/bb/bb-tunnel-air-mini.plist`, log `~/Library/Logs/bb-tunnel-air.log`), so `http://127.0.0.1:38886` on the mini reaches the sab-air server.
Mini bb CLI: `brew upgrade --cask bb` (cask installs `/Applications/bb.app`; `/opt/homebrew/bin/bb` execs its bundled CLI). 0.42.0 failed through the tunnel; 0.47.0 works against the 0.46.1 server. `~/.bb/config.json` `serverUrl` is `http://127.0.0.1:38886` (backup `config.json.bak-pre-0.46.1`).

Mini host daemon (execution machine `sab-mini`, `host_96gbvt568s`), launchd `com.sab.bb-host-daemon-sab-air` (plist copy `docs/bb/bb-host-daemon-mini.plist`, log `~/.bb-machines/100.93.242.91-38886/logs/launchd.log`):
1. The data dir `~/.bb-machines/100.93.242.91-38886` already held the credentials from the earlier direct enrollment; `bb machine reconnect` + the bundle installer refused ("Refusing to overwrite a different machine identity") because `config.json` named the old server URL.
2. Set `serverUrl` in that dir's `config.json` to `http://127.0.0.1:38886` (backup `config.json.bak-pre-tunnel`).
3. `curl -fsS http://127.0.0.1:38886/install.sh -o install.sh`, change `service_label=` to `com.sab.bb-host-daemon-sab-air` (the stock label is `app.getbb.host-daemon.<server>-<host>` and the host part is not descriptive), then `sh install.sh --adopt --data-dir ~/.bb-machines/100.93.242.91-38886`. It installs the plist with `--auto-update` and starts it; `bb machine list` shows `sab-mini` connected.
4. Fresh enrollment instead: `bb machine create --provider manual` / `bb machine reconnect <name>` prints a one-line command whose server URL is the getbb.app connect URL; fetch `http://127.0.0.1:38886/install.sh` with the same `X-BB-Enrollment` header and rewrite `serverUrl` (and empty `headers`) in the exported `BB_ENROLLMENT` to use the tunnel.
5. Threads on the mini need a project source on host `sab-mini` (`bb project create --machine host_96gbvt568s --root <dir>`); concurrent threads need distinct `--environment` dirs (sharing one fails with "Cannot checkout branch while another thread is using this workspace").
6. Verified: claude-code, pi, acp-grok each replied "ok" via `bb thread spawn --machine host_96gbvt568s --provider <id>`.
