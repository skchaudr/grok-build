# Packet: `/learn` over every harness (HARNESS-BRIDGE phase 2)

From: Opus traces lead (Herdr pane `w36:p5`), 2026-10-21. For: native Grok in grok-fork.

## The goal

Grok Build's `/learn` turns past sessions into lessons for the agent, but it only reads Grok's own sessions. Sab's traces index already holds Claude, Pi, Cursor, Codex and other sessions from both Macs. Make `/learn` run over those too, without touching Grok's bundled files. Background: `docs/custom/HARNESS-BRIDGE.md` (phase 2) and `docs/traces/MAP.md`.

## Deliverables

1. **The exporter:** `crates/codegen/xai-grok-traces/learn/export_grok_home.py`, Python 3 standard library only.
   - Reads sessions via `~/.grok/bin/grok-traces list --json --roots` and `grok-traces events <id> --json`. It never opens `traces.sqlite` directly.
   - Writes a synthetic `GROK_HOME` at `~/.grok/traces/learn-home/`, in exactly the layout the bundled collector reads.
     - Read `~/.grok/bundled/skills/learn/collect_sessions.py` first to get that layout. HARNESS-BRIDGE says `sessions/<enc_cwd>/<sid>/{summary.json,chat_history.jsonl}`; the collector is the source of truth.
   - Maps tool names to Grok's (Bash → the Grok shell tool, Edit/Write → Grok's edit tool, and so on), with the mapping as a table at the top of the file.
   - Rebuilds `learn-home/` from scratch on each run.
2. **Optional filters.**
   - Export everything by default.
   - `--source claude,pi,…` and `--since <days>` narrow the export.
   - `~/.grok/traces/learn-exclude.txt` is optional: one `cwd` prefix per line to skip, `#` for comments. A missing file means nothing is excluded.
   - `--dry-run` prints counts per source and never content.
3. **A user-scope skill:** `~/.grok/skills/learn-all/SKILL.md`.
   - It runs the exporter, then the bundled `collect_sessions.py --grok-home ~/.grok/traces/learn-home`, then continues the same way the bundled `learn` skill does.
   - Never name it `learn-traces` or `learn`, and never copy or edit anything under `~/.grok/bundled/`. The collector moves differing user copies of `learn-traces` into `learn/trash/`.
   - Keep the repo copy at `crates/codegen/xai-grok-traces/learn/SKILL.md`, and install it with `cp`.
4. **Tests:** `crates/codegen/xai-grok-traces/learn/test_export.py` (stdlib `unittest`) covering:
   - a fixture of events JSON → expected files;
   - source, since and exclude filtering;
   - the tool-name mapping;
   - a run of the real collector against the fixture output, which must not error.

## Rules

- **No Rust, no cargo.** Another lead holds the build on this machine.
- **Worktree:** work in `git worktree add .worktrees/learn-all -b learn-all main`. Push `learn-all` to origin; don't merge.
- **Commits:** end every commit with `Co-authored-by: grok-cli <grok-cli@x.ai>`.
- **Real data:**
  - Never commit real session content or anything from `learn-home/`.
  - The fixtures are synthetic.
  - For the real run, use `--dry-run` only and report the counts.
- **Tools:** use `rg`, not `grep -r`.
- **When done:** send one message to Herdr pane `w36:p5` (`herdr pane send-text w36:p5 "<msg>"` then `herdr pane send-keys w36:p5 enter`). The message carries the branch, the test result, the dry-run counts, and anything unverified.

## Verified facts (binding; from a Sonnet check of the real collector, `state.py`, the skills loader and `grok-traces` output)

**Synthetic home layout**
- Path: `learn-home/sessions/<enc_cwd>/<sid>/{summary.json,chat_history.jsonl}`.
  - `enc_cwd` = `urllib.parse.quote(cwd, safe="")`.
  - `sid` = the trace id with `:` replaced by `_`.
- `summary.json` fields:
  - `info: {id: sid, cwd}`;
  - `git_root_dir` (may be `""`) and `head_branch`;
  - `generated_title` = the trace title;
  - `session_kind: "interactive"`. Never `subagent*` or `headless`, or the session is dropped;
  - `created_at` and `updated_at` as ISO-8601, from the epoch ms;
  - `current_model_id`.
- `chat_history.jsonl` records:
  - User: `{"type":"user","content":[{"type":"text","text":"<user_query>\n…full text…\n</user_query>"}]}`. Without the `<user_query>` wrapper the session has 0 turns and is dropped.
  - Assistant: `{"type":"assistant","content":"","tool_calls":[{"id":"call-…","name":<grok tool>,"arguments":"<JSON-encoded string>"}],"model_id":…}`.
  - Tool result (optional): `{"type":"tool_result","tool_call_id":…,"content":"…"}`.
  - Never emit system or reminder text. User records with `synthetic_reason` are skipped.
- **The collector drops a session when:**
  - it has no real user turn;
  - it has no tool use and every turn is under 3 words;
  - its `cwd` is under an OS temp dir or contains `grok-e2e`.

**Rebuild scope**
- Wipe and recreate only `learn-home/sessions/`. Keep `learn-home/learn/` (`state.json`, `runs/`, `trash/`, `decisions.jsonl`).
- Symlink from `~/.grok` into `learn-home/`, if present: `skills`, `bundled`, `config.toml`, `settings.json`, `hooks`, `installed-plugins` and `slash-mru.json`. Without them, `/learn` sees every real skill as unused.
- Do **not** symlink `workflows`. Stale-workflow retirement would move real files.

**Input** (`list --json --roots` and `events <id> --json` both print JSONL)
- `list` keys: `id` (`claude:<hex>`), `source`, `title`, `path`, `model`, `cwd`, `started_at`/`updated_at` (epoch ms), counts, `status`, `parent_id` and others.
- `events` keys: `line`, `type`, `kind` (`message`/`tool`), `role` (`user`/`assistant`/`tool`/`tool_result`), `title` (the tool name), `preview` and `detail` (both truncated to 101 chars), `timestamp`, `noise`, and others.
- Full content comes from `grok-traces raw <id> <line>`, which returns the harness-native record. For Claude, that's `message.content[]` with `tool_use {name, input}`.
- Parse native records per source:
  - Claude is required.
  - Other sources are best effort: skip any source you can't parse, and count it in `--dry-run`.
- Call `raw` only for `role in (user, tool)` events with `noise=false`, and never use `--noise`.
- Skip `source == "grok"`; those sessions are already in `~/.grok/sessions`.
- `--dry-run` also counts sessions skipped for having no parseable user text.

**Tool mapping** (Grok names, checked against 400 real `chat_history` files)

| Original | Grok tool | Argument keys |
|---|---|---|
| `Bash` | `run_terminal_command` | `command`, `description` |
| `Read` | `read_file` | rename `file_path` → `target_file` (+ `limit`) |
| `Write` | `write` | `file_path`, `content` |
| `Edit`, `MultiEdit` | `search_replace` | `file_path`, `old_string`, `new_string` |
| `Grep` | `grep` | `pattern`, `glob`, `head_limit` |
| `Glob`, `LS` | `list_dir` | `target_directory` |
| `WebFetch` | `web_fetch` | `url` |
| `TodoWrite` | `todo_write` | |
| `Task`, `Agent` | `spawn_subagent` | |
| `mcp__<server>__<tool>` | `use_tool` | `{"tool_name":"<server>__<tool>"}` |
| anything else | unchanged | |

Remap argument keys too. The collector reads `target_file`, `file_path` or `path`, and detects skill loads from paths matching `…/skills/<name>/SKILL.md`.

**Skill**
- Frontmatter: `name: learn-all`, a `description`, `user-invocable: true`. Discovery path: `~/.grok/skills/<name>/SKILL.md` (`xai-grok-shell/src/extensions/skills.rs`).
- Steps:
  1. Run the exporter.
  2. Set `LH=~/.grok/traces/learn-home`, run `state.py --grok-home "$LH" get`, and read its `python` field.
  3. Run the collector with `--grok-home "$LH"`, plus the bundled skill's optional flags.
  4. Follow the bundled `~/.grok/bundled/skills/learn/SKILL.md` from step 0 through step 4, with `<GROK_HOME>` = `$LH`.
- **Every `state.py` call needs `--grok-home "$LH"`.**
- `<skill dir>` stays the bundled `~/.grok/bundled/skills/learn`. Launch the workflow by name `learn-traces`, with `args.skill_dir` = the bundled path.
- Curation edits land in the real `~/.grok/skills`, through the symlink. `state.py trash` moves items into `$LH/learn/trash/`.
- `learn-all` collides with nothing. The collector only retires `.rhai` files under `<home>/workflows` whose meta name is `learn-traces`.

**Tests and side effects**
- Run the collector as `collect_sessions.py --grok-home <tmp fixture home> --out <tmp>/run --include-headless`. It must exit 0 and its `manifest.json` must show `sessions_kept == N`.
- With the symlinks present, assert that `usage.json` shows the fixture user's skills as `loaded`.
- Tests never call the real `grok-traces`. Inject a fake runner, using `list` + `raw` fixtures for one Claude session and one other source.
- **Never run `collect_sessions.py`** against a real home without `--out` or a temp `--grok-home`. By default it writes `learn/runs/` and `state.json`, and prunes `runs/` older than 30 days.
- The real-data run is `--dry-run` only: it reads `list` rows and nothing else.
