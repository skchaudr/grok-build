---
name: learn-all
description: >-
  Learn from every harness the traces index already holds (Claude, Pi, Cursor,
  Codex, OMP, DSH), not only Grok's own sessions. Use on "/learn-all",
  "learn from all traces", "learn from Claude sessions", "learn across
  harnesses". Exports a synthetic Grok home, then runs the bundled learn
  collector and the learn-traces workflow against that home.
user-invocable: true
argument-hint: "[--source claude,pi,…] [--since DAYS] [--mode step|auto|report] [--days N | --since-last | --limit N]"
---

# learn-all

Grok's `/learn` reads `~/.grok/sessions`. This skill copies other harnesses out of the traces index into a synthetic home, then runs the bundled learn skill there.

Bindings for every step below:

- `$LH` = `~/.grok/traces/learn-home`
- `<GROK_HOME>` = `$LH`
- `<skill dir>` = `~/.grok/bundled/skills/learn` (read it; do not copy or edit anything under `~/.grok/bundled/`)
- Exporter = `export_grok_home.py` next to this SKILL.md
- Workflow name = `learn-traces`, with `args.skill_dir` = `<skill dir>` and `args.grok_home` = `$LH`

The exporter links `$LH/skills` at the real `~/.grok/skills`. Curation writes through that link, so skill edits land in the real user skills. `state.py trash` moves files into `$LH/learn/trash/`. The exporter leaves `workflows` unlinked; leave it that way so retirement stays inside `$LH`.

Every `state.py` invocation takes `--grok-home "$LH"` (`get`, `set`, `wait`, `decide`, `trash`).

## 1. Export

Probe the interpreter the way `<skill dir>/SKILL.md` describes: run `state.py --grok-home "$LH" get` with `python`, then `py -3`, then `python3`. The first that prints JSON (exit 0, or 3 when a run is pending) is `<PYTHON>`. Its `python` field is the absolute interpreter to quote in later commands. Its `grok_home` field is `$LH`.

```bash
<PYTHON> <directory of this SKILL.md>/export_grok_home.py [--source claude,pi,…] [--since DAYS]
```

No flags exports every parseable root session. `--source` and `--since` narrow it. `~/.grok/traces/learn-exclude.txt` (one cwd prefix per line, `#` comments) skips prefixes when the file exists. A learn pass runs the exporter without `--dry-run`. `--dry-run` prints counts and does not build the home.

## 2. Bundled learn, steps 0 through 4

Follow `~/.grok/bundled/skills/learn/SKILL.md` from step 0 through step 4 with the bindings above.

- Step 0 prices the run with `<PYTHON> <skill dir>/collect_sessions.py --grok-home "$LH" --estimate`.
- Step 1 is the collection: `<PYTHON> <skill dir>/collect_sessions.py --grok-home "$LH"` plus the bundled flags the user asked for (`--days`, `--since-last`, `--limit`, `--cwd`, `--include-headless`, `--drop-pattern`). That command is the only collection.
- Step 2 launches workflow `learn-traces` by name. `args` is a JSON object: `run_dir`, `count`, `skill_dir` = `<skill dir>`, `grok_home` = `$LH`, `python` = `<PYTHON>`, `batch`, `fan`, `today`, `user_label`, and `focus` when the user gave one. Then `<PYTHON> <skill dir>/state.py --grok-home "$LH" wait`, as the bundled skill requires.
- Steps 3 and 4 present the report and curate. Apply paths come from `$LH`'s `surfaces.json` (`user_skills_dir` follows the skills symlink). Trash with `<PYTHON> <skill dir>/state.py --grok-home "$LH" trash …`.
