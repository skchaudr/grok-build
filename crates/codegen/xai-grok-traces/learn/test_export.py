#!/usr/bin/env python3
"""Synthetic fixtures only. Never calls the real grok-traces binary."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import export_grok_home as ex

NOW = 1_700_000_000_000
COLLECTOR = os.path.expanduser("~/.grok/bundled/skills/learn/collect_sessions.py")


class FakeRunner:
    def __init__(self, rows, events=None, raw=None):
        self.rows = rows
        self._events = events or {}
        self._raw = raw or {}
        self.event_calls = []
        self.raw_calls = []

    def list_rows(self):
        return self.rows

    def events(self, session_id):
        self.event_calls.append(session_id)
        return self._events.get(session_id, [])

    def raw(self, session_id, line):
        self.raw_calls.append((session_id, line))
        return self._raw.get(session_id, {}).get(line)


def _claude_row():
    return {
        "id": "claude:abc",
        "source": "claude",
        "title": "Fix the widget",
        "path": "/x/claude.jsonl",
        "model": "claude-opus",
        "cwd": "/proj/widget-app",
        "started_at": 1700000000000,
        "updated_at": 1700000005000,
        "user_count": 1,
        "tool_count": 1,
    }


def _claude_events():
    return [
        {"line": 1, "type": "user", "kind": "message", "role": "user", "title": "User",
         "preview": "please fix the widget bug", "noise": False, "timestamp": 1700000000000},
        {"line": 2, "type": "assistant/tool_use", "kind": "tool", "role": "tool", "title": "Read",
         "preview": "SKILL.md", "noise": False, "timestamp": 1700000001000},
        {"line": 3, "type": "user/tool_result", "kind": "tool", "role": "tool_result", "title": "Read",
         "noise": False},
        {"line": 4, "type": "user", "kind": "message", "role": "user", "title": "User",
         "noise": True, "preview": "hidden"},
    ]


def _claude_raw(skill_path):
    user = {
        "type": "user",
        "message": {"role": "user", "content": [{
            "type": "text",
            "text": "<system-reminder>\nignore this reminder\n</system-reminder>\n<user_query>\nplease fix the widget bug\n</user_query>",
        }]},
    }
    tool = {
        "type": "assistant",
        "message": {
            "role": "assistant",
            "model": "claude-opus",
            "content": [{
                "type": "tool_use",
                "id": "toolu_1",
                "name": "Read",
                "input": {"file_path": skill_path, "limit": 20},
            }],
        },
    }
    return {1: json.dumps(user), 2: json.dumps(tool), 3: "{}", 4: "{}"}


class MappingTests(unittest.TestCase):
    def test_tool_table(self):
        cases = [
            ("Bash", {"command": "ls", "description": "list", "timeout": 1},
             [("run_terminal_command", {"command": "ls", "description": "list"})]),
            ("Read", {"file_path": "a.py", "limit": 5},
             [("read_file", {"target_file": "a.py", "limit": 5})]),
            ("Write", {"path": "a.py", "contents": "hi"},
             [("write", {"file_path": "a.py", "content": "hi"})]),
            ("Edit", {"file_path": "a.py", "old_string": "a", "new_string": "b"},
             [("search_replace", {"file_path": "a.py", "old_string": "a", "new_string": "b"})]),
            ("MultiEdit", {"file_path": "a.py", "edits": [
                {"old_string": "a", "new_string": "b"},
                {"old_string": "c", "new_string": "d"},
            ]}, [
                ("search_replace", {"file_path": "a.py", "old_string": "a", "new_string": "b"}),
                ("search_replace", {"file_path": "a.py", "old_string": "c", "new_string": "d"}),
            ]),
            ("Grep", {"query": "x", "glob": "*.py", "limit": 3},
             [("grep", {"pattern": "x", "glob": "*.py", "head_limit": 3})]),
            ("Glob", {"path": "src"}, [("list_dir", {"target_directory": "src"})]),
            ("LS", {"target_directory": "src"}, [("list_dir", {"target_directory": "src"})]),
            ("WebFetch", {"url": "https://example.com"},
             [("web_fetch", {"url": "https://example.com"})]),
            ("TodoWrite", {"todos": [{"content": "a"}]},
             [("todo_write", {"todos": [{"content": "a"}]})]),
            ("Task", {"description": "look"}, [("spawn_subagent", {"description": "look"})]),
            ("Agent", {"prompt": "look"}, [("spawn_subagent", {"prompt": "look"})]),
            ("mcp__github__create_issue", {"title": "t"},
             [("use_tool", {"tool_name": "github__create_issue"})]),
            ("Shell", {"command": "ls"}, [("Shell", {"command": "ls"})]),
        ]
        for name, args, expected in cases:
            self.assertEqual(ex.map_tool(name, args), expected, name)


class RenderTests(unittest.TestCase):
    def test_events_fixture_writes_expected_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = os.path.join(tmp, "learn-home")
            skill = "/proj/widget-app/skills/fixture-skill/SKILL.md"
            row = _claude_row()
            runner = FakeRunner([row], {"claude:abc": _claude_events()}, {"claude:abc": _claude_raw(skill)})
            result = ex.export(runner, home, grok_src=os.path.join(tmp, "empty-src"), exclude_path=os.path.join(tmp, "missing.txt"))
            self.assertEqual(result["totals"]["export"], 1)
            self.assertEqual(runner.raw_calls, [("claude:abc", 1), ("claude:abc", 2)])
            dest = os.path.join(home, "sessions", "%2Fproj%2Fwidget-app", "claude_abc")
            with open(os.path.join(dest, "summary.json"), encoding="utf-8") as handle:
                summary = json.load(handle)
            self.assertEqual(summary["info"], {"id": "claude_abc", "cwd": "/proj/widget-app"})
            self.assertEqual(summary["git_root_dir"], "")
            self.assertEqual(summary["head_branch"], "")
            self.assertEqual(summary["generated_title"], "Fix the widget")
            self.assertEqual(summary["session_kind"], "interactive")
            self.assertEqual(summary["created_at"], "2023-11-14T22:13:20+00:00")
            self.assertEqual(summary["updated_at"], "2023-11-14T22:13:25+00:00")
            self.assertEqual(summary["current_model_id"], "claude-opus")
            with open(os.path.join(dest, "chat_history.jsonl"), encoding="utf-8") as handle:
                lines = handle.read().splitlines()
            records = [json.loads(line) for line in lines]
            self.assertEqual(records[0], {
                "type": "user",
                "content": [{"type": "text", "text": "<user_query>\nplease fix the widget bug\n</user_query>"}],
            })
            self.assertNotIn("system-reminder", lines[0])
            self.assertNotIn("synthetic_reason", records[0])
            tool = records[1]
            self.assertEqual(tool["type"], "assistant")
            self.assertEqual(tool["content"], "")
            self.assertEqual(tool["model_id"], "claude-opus")
            self.assertEqual(tool["tool_calls"][0]["id"], "toolu_1")
            self.assertEqual(tool["tool_calls"][0]["name"], "read_file")
            self.assertEqual(json.loads(tool["tool_calls"][0]["arguments"]), {"target_file": skill, "limit": 20})


class FilterTests(unittest.TestCase):
    def _rows(self):
        def row(sid, source, cwd, updated, user_count=2):
            return {"id": sid, "source": source, "cwd": cwd, "updated_at": updated,
                    "started_at": updated, "user_count": user_count, "title": "t", "model": "m"}
        return [
            row("claude:new", "claude", "/proj/keep", NOW - 3_600_000),
            row("claude:old", "claude", "/proj/keep", NOW - 10 * 86_400_000),
            row("claude:skip", "claude", "/skip/tree/app", NOW),
            row("claude:tree2", "claude", "/skip/tree2", NOW, user_count=1),
            row("claude:empty", "claude", "/proj/keep", NOW, user_count=0),
            row("pi:one", "pi", "/proj/pi", NOW, user_count=1),
            row("grok:one", "grok", "/proj/keep", NOW, user_count=4),
            row("bb:one", "bb", "/proj/bb", NOW, user_count=3),
        ]

    def test_source_since_exclude_and_dry_run_reads_list_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = os.path.join(tmp, "home")
            exclude = os.path.join(tmp, "learn-exclude.txt")
            with open(exclude, "w", encoding="utf-8") as handle:
                handle.write("# comment\n\n/skip/tree\n")
            runner = FakeRunner(self._rows())
            result = ex.export(
                runner, home, grok_src=tmp, exclude_path=exclude,
                sources="claude", since_days=2, dry_run=True, now_ms=NOW,
            )
            self.assertEqual(runner.event_calls, [])
            self.assertFalse(os.path.exists(home))
            claude = result["by_source"]["claude"]
            self.assertEqual(claude["listed"], 5)
            self.assertEqual(claude["export"], 2)  # new + tree2
            self.assertEqual(claude["skip_since"], 1)
            self.assertEqual(claude["skip_exclude"], 1)
            self.assertEqual(claude["skip_no_user_text"], 1)
            self.assertEqual(result["by_source"]["pi"]["skip_source"], 1)
            self.assertEqual(result["by_source"]["grok"]["skip_grok"], 1)
            self.assertEqual(result["by_source"]["bb"]["skip_source"], 1)
            self.assertEqual(result["totals"]["listed"], 8)
            unparsed = ex.export(
                runner, home, grok_src=tmp, exclude_path=exclude,
                since_days=2, dry_run=True, now_ms=NOW,
            )
            self.assertEqual(unparsed["by_source"]["bb"]["skip_unparsed"], 1)
            self.assertEqual(unparsed["by_source"]["pi"]["export"], 1)
            self.assertEqual(runner.event_calls, [])

    def test_rebuild_keeps_learn_and_drops_filtered_sessions(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = os.path.join(tmp, "home")
            os.makedirs(os.path.join(home, "learn"))
            with open(os.path.join(home, "learn", "preserved.txt"), "w", encoding="utf-8") as handle:
                handle.write("keep\n")
            os.makedirs(os.path.join(home, "sessions", "stale"))
            user = json.dumps({"type": "user", "message": {"role": "user", "content": "please fix the widget bug"}})
            runner = FakeRunner(
                self._rows(),
                events={"claude:new": [{"line": 1, "role": "user", "noise": False}]},
                raw={"claude:new": {1: user}},
            )
            exclude = os.path.join(tmp, "learn-exclude.txt")
            with open(exclude, "w", encoding="utf-8") as handle:
                handle.write("/skip/tree\n")
            result = ex.export(
                runner, home, grok_src=os.path.join(tmp, "src"), exclude_path=exclude,
                sources="claude", since_days=2, now_ms=NOW,
            )
            self.assertEqual(runner.event_calls, ["claude:new", "claude:tree2", "claude:empty"])
            self.assertEqual(result["by_source"]["claude"]["export"], 1)
            self.assertEqual(result["by_source"]["claude"]["skip_no_user_text"], 2)  # empty + tree2 with no raw
            self.assertTrue(os.path.isfile(os.path.join(home, "learn", "preserved.txt")))
            self.assertFalse(os.path.isdir(os.path.join(home, "sessions", "stale")))
            self.assertTrue(os.path.isfile(os.path.join(home, "sessions", "%2Fproj%2Fkeep", "claude_new", "chat_history.jsonl")))
            self.assertFalse(os.path.exists(os.path.join(home, "workflows")))


class CollectorTests(unittest.TestCase):
    def test_collector_keeps_sessions_and_marks_skill_loaded(self):
        with tempfile.TemporaryDirectory() as tmp:
            grok_src = os.path.join(tmp, "grok-src")
            skill_dir = os.path.join(grok_src, "skills", "fixture-skill")
            os.makedirs(skill_dir)
            with open(os.path.join(skill_dir, "SKILL.md"), "w", encoding="utf-8") as handle:
                handle.write("---\nname: fixture-skill\ndescription: Fixture skill.\n---\n\nFixture.\n")
            os.makedirs(os.path.join(grok_src, "workflows"))
            with open(os.path.join(grok_src, "config.toml"), "w", encoding="utf-8") as handle:
                handle.write("[plugins]\nenabled = []\n")
            home = os.path.join(tmp, "learn-home")
            skill_path = os.path.join(home, "skills", "fixture-skill", "SKILL.md")
            claude_raw = _claude_raw(skill_path)
            pi_user = json.dumps({
                "type": "message",
                "message": {"role": "user", "content": [{"type": "text", "text": "please fix the pi parser"}]},
            })
            rows = [
                _claude_row(),
                {"id": "pi:one", "source": "pi", "title": "Pi task", "cwd": "/proj/pi-app",
                 "model": "pi-model", "started_at": NOW, "updated_at": NOW, "user_count": 1},
                {"id": "grok:local", "source": "grok", "cwd": "/proj/grok", "updated_at": NOW, "user_count": 2},
                {"id": "bb:one", "source": "bb", "cwd": "/proj/bb", "updated_at": NOW, "user_count": 2},
            ]
            runner = FakeRunner(
                rows,
                events={
                    "claude:abc": _claude_events(),
                    "pi:one": [{"line": 1, "role": "user", "noise": False}],
                },
                raw={"claude:abc": claude_raw, "pi:one": {1: pi_user}},
            )
            result = ex.export(runner, home, grok_src=grok_src, exclude_path=os.path.join(tmp, "no-exclude.txt"))
            self.assertEqual(result["totals"]["export"], 2)
            self.assertEqual(result["by_source"]["grok"]["skip_grok"], 1)
            self.assertEqual(result["by_source"]["bb"]["skip_unparsed"], 1)
            self.assertTrue(os.path.islink(os.path.join(home, "skills")))
            self.assertTrue(os.path.islink(os.path.join(home, "config.toml")))
            self.assertFalse(os.path.lexists(os.path.join(home, "workflows")))
            out = os.path.join(tmp, "run")
            proc = subprocess.run(
                [sys.executable, COLLECTOR, "--grok-home", home, "--out", out, "--include-headless"],
                capture_output=True, text=True,
            )
            self.assertEqual(proc.returncode, 0, proc.stderr + proc.stdout)
            with open(os.path.join(out, "manifest.json"), encoding="utf-8") as handle:
                manifest = json.load(handle)
            self.assertEqual(manifest["sessions_kept"], 2)
            with open(os.path.join(out, "usage.json"), encoding="utf-8") as handle:
                usage = json.load(handle)
            loaded = [item for item in usage["items"] if item.get("kind") == "skill" and item.get("name") == "fixture-skill"]
            self.assertEqual(len(loaded), 1)
            self.assertTrue(loaded[0]["loaded"])
            self.assertGreater(loaded[0]["count"], 0)


if __name__ == "__main__":
    unittest.main()
