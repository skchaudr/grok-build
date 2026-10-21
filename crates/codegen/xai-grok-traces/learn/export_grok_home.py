#!/usr/bin/env python3
"""Export grok-traces sessions into a synthetic GROK_HOME for /learn.

Reads `grok-traces list --json --roots` and, except on --dry-run,
`events <id> --json` plus `raw <id> <line>`. Never opens traces.sqlite.
Stdlib only.

Tool-name mapping (Grok names, checked against chat_history):

    Original            Grok tool               Argument keys
    Bash                run_terminal_command    command, description
    Read                read_file               file_path -> target_file, limit
    Write               write                   file_path, content
    Edit, MultiEdit     search_replace          file_path, old_string, new_string
    Grep                grep                    pattern, glob, head_limit
    Glob, LS            list_dir                target_directory
    WebFetch            web_fetch               url
    TodoWrite           todo_write              pass through
    Task, Agent         spawn_subagent          pass through
    mcp__<s>__<t>       use_tool                tool_name = <s>__<t>
    anything else       unchanged               pass through
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import re
import shutil
import subprocess
import sys
import time
import urllib.parse

# CLI defaults: list limit 50 (0 is rejected), events limit 2000, no offset flag.
# A high cap is what "export everything" can actually ask for.
LIST_LIMIT = 1_000_000
EVENT_LIMIT = 1_000_000

HOME_DEFAULT = os.path.expanduser("~/.grok/traces/learn-home")
EXCLUDE_DEFAULT = os.path.expanduser("~/.grok/traces/learn-exclude.txt")
TRACES_BIN_DEFAULT = os.path.expanduser("~/.grok/bin/grok-traces")
GROK_SRC_DEFAULT = os.path.expanduser("~/.grok")

# Sources whose native records this file can turn into chat_history lines.
# Anything else is counted and skipped (bb included: its raw row is a db envelope).
PARSABLE = {"claude", "cursor", "pi", "codex", "omp", "dsh"}

# Linked into the synthetic home when the source exists. Not workflows:
# stale-workflow retirement would move the real files.
LINK_NAMES = (
    "skills",
    "bundled",
    "config.toml",
    "settings.json",
    "hooks",
    "installed-plugins",
    "slash-mru.json",
)

COUNT_KEYS = (
    "listed",
    "export",
    "skip_grok",
    "skip_source",
    "skip_since",
    "skip_exclude",
    "skip_unparsed",
    "skip_no_user_text",
)

# grok_name, kept keys (None = pass the argument object through), aliases
TOOL_TABLE = {
    "Bash": ("run_terminal_command", ("command", "description"), {"cmd": "command"}),
    "Read": ("read_file", ("target_file", "limit"), {"file_path": "target_file", "path": "target_file"}),
    "Write": ("write", ("file_path", "content"), {"path": "file_path", "contents": "content"}),
    "Edit": (
        "search_replace",
        ("file_path", "old_string", "new_string"),
        {"path": "file_path", "old_str": "old_string", "new_str": "new_string"},
    ),
    "MultiEdit": (
        "search_replace",
        ("file_path", "old_string", "new_string"),
        {"path": "file_path", "old_str": "old_string", "new_str": "new_string"},
    ),
    "Grep": ("grep", ("pattern", "glob", "head_limit"), {"query": "pattern", "limit": "head_limit"}),
    "Glob": ("list_dir", ("target_directory",), {"path": "target_directory", "directory": "target_directory"}),
    "LS": ("list_dir", ("target_directory",), {"path": "target_directory", "directory": "target_directory"}),
    "WebFetch": ("web_fetch", ("url",), {"uri": "url"}),
    "TodoWrite": ("todo_write", None, {}),
    "Task": ("spawn_subagent", None, {}),
    "Agent": ("spawn_subagent", None, {}),
}

_QUERY_RE = re.compile(r"<user_query>(.*?)</user_query>", re.S)
_STRIP_RE = re.compile(
    r"<(?:system-reminder|timestamp|user_info|local-command-stdout|command-name|command-message|command-args)>.*?</(?:system-reminder|timestamp|user_info|local-command-stdout|command-name|command-message|command-args)>",
    re.S | re.I,
)
_CALL_TYPES = {"tool_use", "toolCall", "tool_call", "function_call"}
_SKIP_TEXT_TYPES = _CALL_TYPES | {"tool_result", "toolResult", "thinking", "redacted_thinking", "image", "image_url"}


def map_tool(name: str, args: dict) -> list[tuple[str, dict]]:
    """Map one harness tool call to one or more Grok tool calls."""
    if not isinstance(args, dict):
        args = {}
    if name.startswith("mcp__") and "__" in name[5:]:
        return [("use_tool", {"tool_name": name[5:]})]
    spec = TOOL_TABLE.get(name)
    if spec is None:
        return [(name, dict(args))]
    grok_name, keep, aliases = spec
    edits = args.get("edits")
    if name in ("Edit", "MultiEdit") and isinstance(edits, list) and edits:
        file_path = args.get("file_path") or args.get("path") or ""
        split = []
        for ed in edits:
            if not isinstance(ed, dict):
                continue
            merged = {
                "file_path": ed.get("file_path") or file_path,
                "old_string": ed.get("old_string", ed.get("old_str", "")),
                "new_string": ed.get("new_string", ed.get("new_str", "")),
            }
            split.append((grok_name, _remap(merged, ("file_path", "old_string", "new_string"), {})))
        if split:
            return split
    if keep is None:
        return [(grok_name, dict(args))]
    return [(grok_name, _remap(args, keep, aliases))]


def _remap(args: dict, keep: tuple[str, ...], aliases: dict[str, str]) -> dict:
    found: dict = {}
    for key in keep:
        if key in args:
            found[key] = args[key]
    for src, dest in aliases.items():
        if dest in keep and dest not in found and src in args:
            found[dest] = args[src]
    return {key: found[key] for key in keep if key in found}


def _as_ms(value) -> int | None:
    if isinstance(value, bool) or value is None:
        return None
    if isinstance(value, (int, float)):
        return int(value)
    return None


def iso_ms(ms: int | None) -> str:
    if ms is None:
        return ""
    return dt.datetime.fromtimestamp(ms / 1000, dt.timezone.utc).isoformat(timespec="seconds")


def session_sid(trace_id: str) -> str:
    return trace_id.replace(":", "_")


def cwd_excluded(cwd: str, prefixes: list[str]) -> bool:
    if not cwd:
        return False
    cwd_n = os.path.normcase(os.path.normpath(cwd))
    for raw in prefixes:
        prefix = os.path.normcase(os.path.normpath(raw))
        if cwd_n == prefix or cwd_n.startswith(prefix.rstrip("/\\") + os.sep):
            return True
    return False


def load_exclude(path: str) -> list[str]:
    path = os.path.expanduser(path)
    if not os.path.isfile(path):
        return []
    prefixes = []
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            prefixes.append(line)
    return prefixes


def _user_chunks(text: str) -> list[str]:
    found = [part.strip() for part in _QUERY_RE.findall(text)]
    found = [part for part in found if part]
    if found:
        return found
    cleaned = _STRIP_RE.sub("", text).strip()
    return [cleaned] if cleaned else []


def _texts_from_content(content) -> list[str]:
    if isinstance(content, str):
        return _user_chunks(content)
    if not isinstance(content, list):
        return []
    out = []
    for block in content:
        if isinstance(block, str):
            out.extend(_user_chunks(block))
        elif isinstance(block, dict) and block.get("type") not in _SKIP_TEXT_TYPES:
            text = block.get("text")
            if isinstance(text, str):
                out.extend(_user_chunks(text))
    return out


def _as_args(value) -> dict:
    if isinstance(value, str):
        try:
            parsed = json.loads(value)
        except ValueError:
            return {"_raw": value} if value else {}
        return parsed if isinstance(parsed, dict) else {}
    return value if isinstance(value, dict) else {}


def _call(name: str, args, call_id) -> dict | None:
    if not name:
        return None
    return {"name": name, "args": _as_args(args), "id": call_id}


def _calls_from_content(content) -> list[dict]:
    if not isinstance(content, list):
        return []
    out = []
    for block in content:
        if not isinstance(block, dict) or block.get("type") not in _CALL_TYPES:
            continue
        name = block.get("name") or block.get("toolName") or ""
        function = block.get("function")
        if not name and isinstance(function, dict):
            name = function.get("name") or ""
        args = block.get("input")
        if args is None:
            args = block.get("arguments", block.get("args"))
        call = _call(name, args, block.get("id") or block.get("callId") or block.get("tool_use_id"))
        if call:
            out.append(call)
    return out


def _calls_from_list(items) -> list[dict]:
    if not isinstance(items, list):
        return []
    out = []
    for block in items:
        if not isinstance(block, dict):
            continue
        name = block.get("name") or block.get("toolName") or ""
        args = block.get("arguments")
        if args is None:
            args = block.get("input", block.get("args"))
        call = _call(name, args, block.get("id") or block.get("callId"))
        if call:
            out.append(call)
    return out


def _calls_from_flat(obj: dict) -> list[dict]:
    name = obj.get("name") or obj.get("toolName") or obj.get("tool") or ""
    args = obj.get("arguments")
    if args is None:
        args = obj.get("input", obj.get("args"))
    call = _call(name, args, obj.get("id") or obj.get("callId"))
    return [call] if call else []


def parse_raw(raw) -> dict | None:
    """Pull user texts and tool calls out of one harness-native record."""
    if isinstance(raw, str):
        try:
            obj = json.loads(raw)
        except ValueError:
            return None
    elif isinstance(raw, dict):
        obj = raw
    else:
        return None
    if not isinstance(obj, dict):
        return None

    synthetic = bool(obj.get("synthetic_reason"))
    model = ""
    texts: list[str] = []
    calls: list[dict] = []

    message = obj.get("message")
    if isinstance(message, dict):
        synthetic = synthetic or bool(message.get("synthetic_reason"))
        model = message.get("model") or message.get("model_id") or ""
        role = message.get("role") or ""
        if role not in ("toolResult", "tool_result"):
            texts.extend(_texts_from_content(message.get("content")))
        calls.extend(_calls_from_content(message.get("content")))
        calls.extend(_calls_from_list(message.get("tool_calls")))

    payload = obj.get("payload")
    if isinstance(payload, dict):
        ptype = payload.get("type") or ""
        model = model or payload.get("model") or ""
        if ptype == "message":
            role = payload.get("role") or ""
            if role not in ("toolResult", "tool_result", "developer"):
                texts.extend(_texts_from_content(payload.get("content")))
            calls.extend(_calls_from_content(payload.get("content")))
        elif ptype == "function_call":
            calls.extend(_calls_from_flat(payload))

    data = obj.get("data")
    typ = obj.get("type") or ""
    if isinstance(data, dict):
        if typ in ("user/message", "user") or data.get("role") in ("user", "human"):
            content = data.get("content")
            texts.extend(_user_chunks(content) if isinstance(content, str) else _texts_from_content(content))
        if typ == "tool/call":
            calls.extend(_calls_from_flat(data))

    if not texts and not calls:
        role = obj.get("role") or ""
        if role in ("user", "human") or typ == "user":
            texts.extend(_texts_from_content(obj.get("content")))
        calls.extend(_calls_from_content(obj.get("content")))
        calls.extend(_calls_from_list(obj.get("tool_calls")))
        if typ in ("function_call", "tool_use"):
            calls.extend(_calls_from_flat(obj))

    return {"synthetic": synthetic, "texts": texts, "calls": calls, "model": model}


def user_record(text: str) -> dict:
    return {"type": "user", "content": [{"type": "text", "text": f"<user_query>\n{text}\n</user_query>"}]}


def assistant_record(calls: list[dict], model: str, line: int) -> dict:
    tool_calls = []
    n = 0
    for call in calls:
        mapped = map_tool(call["name"], call["args"])
        for offset, (gname, gargs) in enumerate(mapped):
            cid = call.get("id")
            if cid and len(mapped) > 1:
                cid = f"{cid}-{offset}"
            tool_calls.append({
                "id": cid or f"call-{line}-{n}",
                "name": gname,
                "arguments": json.dumps(gargs, ensure_ascii=False, separators=(",", ":")),
            })
            n += 1
    return {"type": "assistant", "content": "", "tool_calls": tool_calls, "model_id": model or ""}


def build_records(row: dict, events: list[dict], raw_fn) -> list[dict]:
    """Turn events JSON plus raw lines into chat_history records.

    `raw_fn` is called only for role user or tool, and only when noise is false.
    """
    model_fallback = row.get("model") or ""
    seen: set[int] = set()
    records = []
    ordered = sorted(events or [], key=lambda ev: _as_ms(ev.get("line")) or 0)
    for ev in ordered:
        if not isinstance(ev, dict) or ev.get("noise"):
            continue
        if ev.get("role") not in ("user", "tool"):
            continue
        line = _as_ms(ev.get("line"))
        if line is None or line in seen:
            continue
        seen.add(line)
        raw = raw_fn(line)
        if raw is None or raw == "":
            continue
        parsed = parse_raw(raw)
        if not parsed or parsed["synthetic"]:
            continue
        if ev.get("role") == "user":
            for text in parsed["texts"]:
                records.append(user_record(text))
        elif parsed["calls"]:
            records.append(assistant_record(parsed["calls"], parsed["model"] or model_fallback, line))
    return records


def summary_for(row: dict, sid: str) -> dict:
    return {
        "info": {"id": sid, "cwd": row.get("cwd") or ""},
        "git_root_dir": "",
        "head_branch": "",
        "generated_title": row.get("title") or "",
        "session_kind": "interactive",
        "created_at": iso_ms(_as_ms(row.get("started_at"))),
        "updated_at": iso_ms(_as_ms(row.get("updated_at"))),
        "current_model_id": row.get("model") or "",
    }


def write_session(home: str, row: dict, sid: str, records: list[dict]) -> str:
    enc = urllib.parse.quote(row.get("cwd") or "", safe="")
    dest = os.path.join(home, "sessions", enc, sid)
    os.makedirs(dest, exist_ok=True)
    with open(os.path.join(dest, "summary.json"), "w", encoding="utf-8") as handle:
        json.dump(summary_for(row, sid), handle, indent=2, ensure_ascii=False)
        handle.write("\n")
    with open(os.path.join(dest, "chat_history.jsonl"), "w", encoding="utf-8") as handle:
        for record in records:
            handle.write(json.dumps(record, ensure_ascii=False, separators=(",", ":")))
            handle.write("\n")
    return dest


def _ensure_link(home: str, grok_src: str, name: str) -> None:
    src = os.path.join(grok_src, name)
    dest = os.path.join(home, name)
    if not os.path.lexists(src):
        if os.path.islink(dest):
            os.unlink(dest)
        return
    src_abs = os.path.abspath(src)
    if os.path.islink(dest):
        current = os.readlink(dest)
        if not os.path.isabs(current):
            current = os.path.abspath(os.path.join(os.path.dirname(dest), current))
        if current == src_abs:
            return
        os.unlink(dest)
    elif os.path.lexists(dest):
        return
    os.symlink(src_abs, dest)


def prepare_home(home: str, grok_src: str) -> None:
    """Wipe sessions/ only. Keep learn/. Refresh symlinks. Leave workflows alone."""
    os.makedirs(home, exist_ok=True)
    sessions = os.path.join(home, "sessions")
    if os.path.islink(sessions):
        os.unlink(sessions)
    elif os.path.isdir(sessions):
        shutil.rmtree(sessions)
    os.makedirs(sessions)
    os.makedirs(os.path.join(home, "learn"), exist_ok=True)
    for name in LINK_NAMES:
        _ensure_link(home, grok_src, name)


def _blank_counts() -> dict:
    return {key: 0 for key in COUNT_KEYS}


def _note(by_source: dict, source: str, key: str) -> None:
    rec = by_source.get(source)
    if rec is None:
        rec = _blank_counts()
        by_source[source] = rec
    rec["listed"] += 1
    rec[key] += 1


def _totals(by_source: dict) -> dict:
    tot = _blank_counts()
    for rec in by_source.values():
        for key in COUNT_KEYS:
            tot[key] += rec[key]
    return tot


def classify_row(row: dict, want: set[str] | None, cutoff: int | None, prefixes: list[str]) -> str:
    source = str(row.get("source") or "").lower()
    if source == "grok":
        return "skip_grok"
    if want is not None and source not in want:
        return "skip_source"
    if cutoff is not None:
        updated = _as_ms(row.get("updated_at"))
        if updated is None or updated < cutoff:
            return "skip_since"
    if cwd_excluded(row.get("cwd") or "", prefixes):
        return "skip_exclude"
    if source not in PARSABLE:
        return "skip_unparsed"
    return "export"


def export(
    runner,
    home: str,
    *,
    grok_src: str = GROK_SRC_DEFAULT,
    exclude_path: str = EXCLUDE_DEFAULT,
    sources: str = "",
    since_days: int = 0,
    dry_run: bool = False,
    now_ms: int | None = None,
) -> dict:
    """Rebuild learn-home/sessions from the runner.

    --dry-run reads list rows only (no events, no raw, no writes) and treats
    user_count == 0 as no parseable user text.
    """
    rows = runner.list_rows()
    if not dry_run:
        prepare_home(home, grok_src)
    want = {part.strip().lower() for part in sources.split(",") if part.strip()} or None
    now = int(time.time() * 1000) if now_ms is None else now_ms
    cutoff = now - since_days * 86_400_000 if since_days else None
    prefixes = load_exclude(exclude_path)
    by_source: dict = {}
    for row in rows:
        if not isinstance(row, dict) or not row.get("id"):
            continue
        source = str(row.get("source") or "?").lower()
        kind = classify_row(row, want, cutoff, prefixes)
        if dry_run:
            if kind == "export" and _as_ms(row.get("user_count")) == 0:
                kind = "skip_no_user_text"
            _note(by_source, source, kind)
            continue
        if kind != "export":
            _note(by_source, source, kind)
            continue
        sid = str(row["id"])

        def raw_fn(line: int, session_id: str = sid):
            return runner.raw(session_id, line)

        records = build_records(row, runner.events(sid), raw_fn)
        if not any(record.get("type") == "user" for record in records):
            _note(by_source, source, "skip_no_user_text")
            continue
        write_session(home, row, session_sid(sid), records)
        _note(by_source, source, "export")
    return {"dry_run": dry_run, "by_source": by_source, "totals": _totals(by_source)}


class GrokTracesRunner:
    """Shells out to grok-traces. list/events are JSONL. raw is the source line."""

    def __init__(self, binary: str = TRACES_BIN_DEFAULT):
        self.binary = os.path.expanduser(binary)

    def list_rows(self) -> list[dict]:
        text = self._run(["list", "--json", "--roots", "--limit", str(LIST_LIMIT)])
        return _parse_jsonl(text)

    def events(self, session_id: str) -> list[dict]:
        # --noise would include bookkeeping. The default hides it.
        text = self._run(["events", session_id, "--json", "--limit", str(EVENT_LIMIT)])
        return _parse_jsonl(text)

    def raw(self, session_id: str, line: int) -> str | None:
        proc = subprocess.run(
            [self.binary, "raw", session_id, str(line)],
            capture_output=True,
            text=True,
        )
        if proc.returncode == 3:
            return None
        if proc.returncode != 0:
            err = (proc.stderr or "").strip()
            raise RuntimeError(f"grok-traces raw {session_id} {line} failed ({proc.returncode}): {err}")
        text = proc.stdout
        return text[:-1] if text.endswith("\n") else text

    def _run(self, args: list[str]) -> str:
        proc = subprocess.run([self.binary, *args], capture_output=True, text=True)
        if proc.returncode != 0:
            err = (proc.stderr or proc.stdout or "").strip()
            raise RuntimeError(f"grok-traces {' '.join(args)} failed ({proc.returncode}): {err}")
        return proc.stdout


def _parse_jsonl(text: str) -> list[dict]:
    rows = []
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        item = json.loads(line)
        if isinstance(item, dict):
            rows.append(item)
    return rows


def main(argv: list[str] | None = None, runner=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--source", default="", help="comma-separated sources (default: every parseable source)")
    ap.add_argument("--since", type=int, default=0, help="only sessions updated in the last N days (0 = all)")
    ap.add_argument("--dry-run", action="store_true", help="print counts per source; read list rows only")
    ap.add_argument("--home", default=HOME_DEFAULT, help="synthetic GROK_HOME (default: ~/.grok/traces/learn-home)")
    ap.add_argument("--grok-src", default=GROK_SRC_DEFAULT, help="real Grok home to link skills and config from")
    ap.add_argument("--exclude", default=EXCLUDE_DEFAULT, help="cwd-prefix exclude file")
    ap.add_argument("--traces-bin", default=TRACES_BIN_DEFAULT)
    args = ap.parse_args(argv)
    if args.since < 0:
        ap.error("--since must be >= 0")
    if runner is None:
        runner = GrokTracesRunner(args.traces_bin)
    try:
        result = export(
            runner,
            os.path.expanduser(args.home),
            grok_src=os.path.expanduser(args.grok_src),
            exclude_path=args.exclude,
            sources=args.source,
            since_days=args.since,
            dry_run=args.dry_run,
        )
    except (RuntimeError, OSError, json.JSONDecodeError) as exc:
        print(str(exc), file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
