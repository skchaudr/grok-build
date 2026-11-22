#!/usr/bin/env python3
"""Adapter server bridging acpx flow run bundles to the piw terminal viewer."""

from __future__ import annotations

import argparse
import asyncio
import copy
import datetime
import decimal
import json
import math
import os
import re
import shutil
import signal
import socket
import subprocess
import sys
import uuid
from pathlib import Path
from typing import Any

SCHEMA_CLIENT = "pi-workflows.client.v1"
SCHEMA_RUN_LIST_PAGE = "pi-workflows.run-list-page.v1"
SCHEMA_RUN_PAGE = "pi-workflows.run-page.v1"
SCHEMA_RUN_VIEW = "pi-workflows.run-view.v1"
SCHEMA_MANIFEST = "pi-workflows.run-manifest.v1"
SCHEMA_STATE = "pi-workflows.run-state.v1"
SCHEMA_WORKFLOW = "pi-workflows.definition-snapshot.v1"
SCHEMA_TRACE = "pi-workflows.trace-event.v1"

DEFAULT_PIW_VERSION = "0.17.7"
MAX_LINE_BYTES = 1_048_576
SNAPSHOT_BUDGET_BYTES = 900_000
MAX_TEXT_CHARS = 4000
MAX_TRACE_PAYLOAD_CHARS = 2000
MAX_RUNS = 200

VALID_STATUSES = {"running", "waiting", "completed", "failed", "timed_out"}
VALID_OUTCOMES = {"ok", "timed_out", "failed", "cancelled"}
NODE_TYPE_MAP = {
    "acp": "agent",
    "agent": "agent",
    "action": "action",
    "compute": "compute",
    "checkpoint": "checkpoint",
    "notify": "notify",
}
ACTION_KEYS = ("actionType", "command", "args", "cwd", "exitCode", "signal", "durationMs")

_JSON_CACHE: dict[str, Any] = {}
_SURROGATE_RE = re.compile(r"[\ud800-\udfff]")


def _log_err(msg: str) -> None:
    print(f"acpx-piw: {msg}", file=sys.stderr, flush=True)


def _clean_surrogates(s: str) -> str:
    if _SURROGATE_RE.search(s):
        return _SURROGATE_RE.sub("\ufffd", s)
    return s


def _utf16_sort_key(s: str) -> list[int]:
    cleaned = _clean_surrogates(s)
    b = cleaned.encode("utf-16-be")
    return [int.from_bytes(b[i : i + 2], "big") for i in range(0, len(b), 2)]


def _format_ecmascript_float(x: float) -> str:
    if math.isnan(x) or math.isinf(x):
        return "null"
    if x == 0.0:
        return "0"
    neg = x < 0
    ax = abs(x)
    d = decimal.Decimal(repr(ax))
    t = d.as_tuple()
    digits_str = "".join(str(dg) for dg in t.digits)
    n = len(t.digits) + int(t.exponent)
    s = digits_str.rstrip("0")
    if not s:
        return "0"
    k = len(s)
    if k <= n <= 21:
        body = s + "0" * (n - k)
    elif 0 < n <= 21:
        body = s[:n] + "." + s[n:]
    elif -6 < n <= 0:
        body = "0." + ("0" * (-n)) + s
    else:
        mant = s[0] if k == 1 else (s[0] + "." + s[1:])
        exp = n - 1
        sign = "+" if exp > 0 else "-"
        body = f"{mant}e{sign}{abs(exp)}"
    return ("-" + body) if neg else body


def canon(obj: Any) -> str:
    if obj is None:
        return "null"
    if isinstance(obj, bool):
        return "true" if obj else "false"
    if isinstance(obj, int):
        if -(1 << 63) <= obj < (1 << 64):
            return str(obj)
        return _format_ecmascript_float(float(obj))
    if isinstance(obj, float):
        return _format_ecmascript_float(obj)
    if isinstance(obj, str):
        return json.dumps(_clean_surrogates(obj), ensure_ascii=False)
    if isinstance(obj, (list, tuple)):
        return "[" + ",".join(canon(item) for item in obj) + "]"
    if isinstance(obj, dict):
        items: list[tuple[list[int], str, Any]] = []
        for k, v in obj.items():
            ks = str(k)
            items.append((_utf16_sort_key(ks), ks, v))
        items.sort(key=lambda item: item[0])
        parts = [f"{canon(ks)}:{canon(v)}" for _, ks, v in items]
        return "{" + ",".join(parts) + "}"
    return canon(str(obj))


def read_json(path: str | Path) -> Any:
    p = Path(path)
    key = str(p.resolve()) if p.exists() else str(p)
    try:
        raw = p.read_bytes()
    except OSError:
        return copy.deepcopy(_JSON_CACHE.get(key))

    try:
        text = raw.decode("utf-8", errors="replace")
    except Exception:
        return copy.deepcopy(_JSON_CACHE.get(key))

    if p.suffix == ".ndjson":
        if not text:
            _JSON_CACHE[key] = []
            return []
        lines = text.split("\n")
        if lines and lines[-1] == "":
            lines.pop()
        elif lines:
            # Drop trailing partial line not terminated by '\n'
            lines.pop()
        records: list[Any] = []
        for line in lines:
            line_s = line.strip()
            if not line_s:
                continue
            try:
                records.append(json.loads(line_s))
            except Exception:
                return copy.deepcopy(_JSON_CACHE.get(key))
        _JSON_CACHE[key] = records
        return copy.deepcopy(records)
    else:
        try:
            val = json.loads(text)
        except Exception:
            return copy.deepcopy(_JSON_CACHE.get(key))
        _JSON_CACHE[key] = val
        return copy.deepcopy(val)


def _truncate_str(s: str, limit: int = MAX_TEXT_CHARS) -> str:
    s = _clean_surrogates(s)
    if len(s) > limit:
        return s[:limit] + "…"
    return s


def _truncate_Strings_in_value(val: Any, limit: int = MAX_TEXT_CHARS) -> Any:
    if isinstance(val, str):
        return _truncate_str(val, limit)
    if isinstance(val, list):
        return [_truncate_Strings_in_value(x, limit) for x in val]
    if isinstance(val, dict):
        return {str(k): _truncate_Strings_in_value(v, limit) for k, v in val.items()}
    return val


def _map_status(raw_status: Any) -> str:
    if isinstance(raw_status, str) and raw_status in VALID_STATUSES:
        return raw_status
    return "ambiguous"


def _map_outcome(raw_outcome: Any) -> str:
    if isinstance(raw_outcome, str) and raw_outcome in VALID_OUTCOMES:
        return raw_outcome
    return "failed"


def _map_node_type(raw_type: Any) -> str:
    if isinstance(raw_type, str):
        return NODE_TYPE_MAP.get(raw_type, raw_type)
    return "compute"


def _normalize_error(err: Any) -> str | None:
    if err is None:
        return None
    if isinstance(err, str):
        s = _truncate_str(err)
        return s if s else "error"
    if isinstance(err, dict):
        msg = err.get("message")
        if isinstance(msg, str) and msg:
            return _truncate_str(msg)
    s = _truncate_str(canon(err))
    return s if s else "error"


def _parse_rfc3339(ts: Any) -> datetime.datetime | None:
    if not isinstance(ts, str) or not ts:
        return None
    try:
        if ts.endswith("Z"):
            ts = ts[:-1] + "+00:00"
        dt = datetime.datetime.fromisoformat(ts)
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=datetime.timezone.utc)
        return dt
    except Exception:
        return None


def _compute_liveness(status: str, updated_at: Any) -> tuple[bool, bool]:
    if status != "running":
        return False, False
    dt = _parse_rfc3339(updated_at)
    if dt is None:
        return False, True
    now = datetime.datetime.now(datetime.timezone.utc)
    age = (now - dt).total_seconds()
    if age <= 90.0:
        return True, False
    return False, True


def _build_manifest_and_display(
    run_dir: Path,
    manifest_raw: dict[str, Any],
    live_raw: dict[str, Any] | None,
    run_raw: dict[str, Any] | None,
) -> tuple[dict[str, Any], dict[str, Any], bool, bool, str]:
    src: dict[str, Any] = {}
    if isinstance(manifest_raw, dict):
        src.update(manifest_raw)
    if isinstance(run_raw, dict):
        src.update(run_raw)
    if isinstance(live_raw, dict):
        src.update(live_raw)

    run_id = str(src.get("runId") or manifest_raw.get("runId") or run_dir.name)
    flow_name = str(src.get("flowName") or manifest_raw.get("flowName") or "flow")
    run_title = src.get("runTitle") if isinstance(src.get("runTitle"), str) else manifest_raw.get("runTitle")
    flow_path = str(src.get("flowPath") or manifest_raw.get("flowPath") or "")
    started_at = str(src.get("startedAt") or manifest_raw.get("startedAt") or "1970-01-01T00:00:00.000Z")
    updated_at = str(src.get("updatedAt") or started_at)
    status = _map_status(src.get("status") or manifest_raw.get("status"))

    if status == "running":
        finished_at = None
    else:
        fin = src.get("finishedAt")
        if not isinstance(fin, str) or not fin:
            fin = updated_at
        finished_at = fin

    manifest: dict[str, Any] = {
        "schema": SCHEMA_MANIFEST,
        "runId": run_id,
        "workflowName": flow_name,
        "workflowSource": {
            "kind": "file",
            "path": flow_path,
            "hash": "",
        },
        "startedAt": started_at,
        "finishedAt": finished_at,
        "status": status,
        "traceSchema": SCHEMA_TRACE,
        "paths": {
            "workflow": "acpx",
            "state": "acpx",
            "trace": "acpx",
        },
    }
    if isinstance(run_title, str) and run_title:
        manifest["runTitle"] = _truncate_str(run_title)

    status_detail = src.get("statusDetail")
    reason = _truncate_str(status_detail) if isinstance(status_detail, str) and status_detail else None

    display: dict[str, Any] = {
        "status": status,
        "activity": "supervised_runner" if status == "running" else None,
        "controls": [],
        "reason": reason,
    }

    live, possibly_interrupted = _compute_liveness(status, updated_at)
    return manifest, display, live, possibly_interrupted, updated_at


def run_row(run_dir: str | Path) -> dict[str, Any] | None:
    rdir = Path(run_dir)
    try:
        manifest_raw = read_json(rdir / "manifest.json")
        if not isinstance(manifest_raw, dict):
            return None
        paths = manifest_raw.get("paths") if isinstance(manifest_raw.get("paths"), dict) else {}
        live_rel = paths.get("liveProjection", "projections/live.json")
        run_rel = paths.get("runProjection", "projections/run.json")
        live_raw = read_json(rdir / live_rel)
        run_raw = read_json(rdir / run_rel)

        manifest, display, live, possibly_interrupted, _ = _build_manifest_and_display(
            rdir,
            manifest_raw,
            live_raw if isinstance(live_raw, dict) else None,
            run_raw if isinstance(run_raw, dict) else None,
        )
        return {
            "manifest": manifest,
            "display": display,
            "live": live,
            "possiblyInterrupted": possibly_interrupted,
        }
    except Exception as exc:
        _log_err(f"skipping run {rdir.name}: {exc}")
        return None


def _extract_prompt_text_from_event(msg: dict[str, Any]) -> str | None:
    params = msg.get("params")
    if not isinstance(params, dict):
        return None
    prompt = params.get("prompt")
    if isinstance(prompt, str):
        return _truncate_str(prompt)
    if isinstance(prompt, list):
        chunks: list[str] = []
        for item in prompt:
            if isinstance(item, dict):
                txt = item.get("text")
                if isinstance(txt, str):
                    chunks.append(txt)
            elif isinstance(item, str):
                chunks.append(item)
        if chunks:
            return _truncate_str("\n".join(chunks))
    return None


def _extract_update_content_text(update: dict[str, Any]) -> str | None:
    content = update.get("content")
    if isinstance(content, list):
        texts: list[str] = []
        for item in content:
            if not isinstance(item, dict):
                continue
            item_type = item.get("type")
            if item_type == "text" and isinstance(item.get("text"), str):
                texts.append(item["text"])
            elif item_type == "content" and isinstance(item.get("content"), dict):
                inner = item["content"]
                if isinstance(inner.get("text"), str):
                    texts.append(inner["text"])
            elif isinstance(item.get("text"), str):
                texts.append(item["text"])
        if texts:
            return _truncate_str("\n".join(texts))
    elif isinstance(content, dict):
        if isinstance(content.get("text"), str):
            return _truncate_str(content["text"])
        inner = content.get("content")
        if isinstance(inner, dict) and isinstance(inner.get("text"), str):
            return _truncate_str(inner["text"])
    elif isinstance(content, str) and content:
        return _truncate_str(content)

    raw_output = update.get("rawOutput")
    if isinstance(raw_output, str) and raw_output:
        return _truncate_str(raw_output)
    return None


def session_entries(run_dir: str | Path, steps: list[dict[str, Any]]) -> list[dict[str, Any]]:
    rdir = Path(run_dir)
    manifest_raw = read_json(rdir / "manifest.json")
    sessions_dir_rel = "sessions"
    if isinstance(manifest_raw, dict) and isinstance(manifest_raw.get("paths"), dict):
        sd = manifest_raw["paths"].get("sessionsDir")
        if isinstance(sd, str) and sd:
            sessions_dir_rel = sd

    session_events_cache: dict[str, list[dict[str, Any]]] = {}

    def get_session_events(session_id: str) -> list[dict[str, Any]]:
        if session_id in session_events_cache:
            return session_events_cache[session_id]
        ev_path = rdir / sessions_dir_rel / session_id / "events.ndjson"
        if not ev_path.exists() and isinstance(manifest_raw, dict):
            for s_entry in manifest_raw.get("sessions") or []:
                if isinstance(s_entry, dict) and s_entry.get("id") == session_id:
                    ep = s_entry.get("eventsPath")
                    if isinstance(ep, str) and ep:
                        ev_path = rdir / ep
                        break
        raw_ev = read_json(ev_path)
        ev_list = [e for e in raw_ev if isinstance(e, dict)] if isinstance(raw_ev, list) else []
        session_events_cache[session_id] = ev_list
        return ev_list

    all_entries: list[dict[str, Any]] = []
    seq_counter = 1

    for step in steps:
        if not isinstance(step, dict):
            continue
        attempt_id = str(step.get("attemptId") or step.get("nodeId") or "step")
        started_at = str(step.get("startedAt") or "1970-01-01T00:00:00.000Z")
        finished_at = str(step.get("finishedAt") or started_at)

        conv_meta = step.get("_traceConversation")
        if not isinstance(conv_meta, dict):
            trace_obj = step.get("trace")
            if isinstance(trace_obj, dict) and isinstance(trace_obj.get("conversation"), dict):
                conv_meta = trace_obj["conversation"]

        if not isinstance(conv_meta, dict):
            continue

        session_id = conv_meta.get("sessionId")
        if not isinstance(session_id, str) or not session_id:
            continue

        start_seq = conv_meta.get("eventStartSeq")
        end_seq = conv_meta.get("eventEndSeq")
        events = get_session_events(session_id)

        sliced_events: list[dict[str, Any]] = []
        for ev in events:
            ev_seq = ev.get("seq")
            if isinstance(ev_seq, int):
                if isinstance(start_seq, int) and ev_seq < start_seq:
                    continue
                if isinstance(end_seq, int) and ev_seq > end_seq:
                    continue
            msg = ev.get("message")
            if isinstance(msg, dict) and msg.get("method") == "_auth/status_update":
                continue
            sliced_events.append(ev)

        step_entries: list[tuple[str, dict[str, Any]]] = []  # (at, message_dict)

        prompt_val = step.get("prompt")
        if prompt_val is None:
            prompt_val = step.get("promptText")
        prompt_text: str | None = None
        if isinstance(prompt_val, str) and prompt_val:
            prompt_text = _truncate_str(prompt_val)
        elif prompt_val is not None:
            prompt_text = _truncate_str(canon(prompt_val))

        prompt_at = started_at
        for ev in sliced_events:
            msg = ev.get("message")
            if isinstance(msg, dict) and msg.get("method") == "session/prompt":
                if isinstance(ev.get("at"), str):
                    prompt_at = ev["at"]
                if prompt_text is None:
                    prompt_text = _extract_prompt_text_from_event(msg)
                break

        if prompt_text is not None:
            step_entries.append((prompt_at, {"role": "user", "content": prompt_text}))

        assistant_parts: list[dict[str, Any]] = []
        assistant_at: str | None = None
        tool_call_index: dict[str, dict[str, Any]] = {}

        def flush_assistant(default_at: str) -> None:
            nonlocal assistant_parts, assistant_at, tool_call_index
            if not assistant_parts:
                return
            for part in assistant_parts:
                if part.get("type") == "toolCall":
                    part["name"] = _truncate_str(str(part.get("name") or part.get("id") or "tool"))
                    args = part.get("arguments")
                    if not isinstance(args, dict):
                        part["arguments"] = {}
                    else:
                        part["arguments"] = _truncate_Strings_in_value(args)
                elif part.get("type") == "thinking":
                    part["thinking"] = _truncate_str(str(part.get("thinking") or ""))
                elif part.get("type") == "text":
                    part["text"] = _truncate_str(str(part.get("text") or ""))
            step_entries.append((assistant_at or default_at, {"role": "assistant", "content": assistant_parts}))
            assistant_parts = []
            assistant_at = None
            tool_call_index = {}

        for ev in sliced_events:
            ev_at = ev.get("at") if isinstance(ev.get("at"), str) else finished_at
            msg = ev.get("message")
            if not isinstance(msg, dict):
                continue
            method = msg.get("method")
            if method == "_auth/status_update":
                continue
            if method != "session/update":
                continue
            params = msg.get("params")
            if not isinstance(params, dict):
                continue
            update = params.get("update")
            if not isinstance(update, dict):
                continue
            su_kind = update.get("sessionUpdate")

            if su_kind == "agent_thought_chunk":
                content = update.get("content")
                txt = content.get("text") if isinstance(content, dict) else None
                if isinstance(txt, str) and txt:
                    if assistant_at is None:
                        assistant_at = ev_at
                    if assistant_parts and assistant_parts[-1].get("type") == "thinking":
                        assistant_parts[-1]["thinking"] += txt
                    else:
                        assistant_parts.append({"type": "thinking", "thinking": txt})

            elif su_kind == "agent_message_chunk":
                content = update.get("content")
                txt = content.get("text") if isinstance(content, dict) else None
                if isinstance(txt, str) and txt:
                    if assistant_at is None:
                        assistant_at = ev_at
                    if assistant_parts and assistant_parts[-1].get("type") == "text":
                        assistant_parts[-1]["text"] += txt
                    else:
                        assistant_parts.append({"type": "text", "text": txt})

            elif su_kind == "tool_call":
                tc_id = str(update.get("toolCallId") or f"call_{len(assistant_parts) + 1}")
                title = update.get("title")
                if not isinstance(title, str) or not title:
                    meta = update.get("_meta")
                    if isinstance(meta, dict) and isinstance(meta.get("claudeCode"), dict):
                        title = meta["claudeCode"].get("toolName")
                name = str(title) if isinstance(title, str) and title else tc_id
                raw_in = update.get("rawInput")
                args = raw_in if isinstance(raw_in, dict) and len(raw_in) > 0 else {}
                if assistant_at is None:
                    assistant_at = ev_at
                tc_part: dict[str, Any] = {
                    "type": "toolCall",
                    "id": tc_id,
                    "name": name,
                    "arguments": args,
                }
                assistant_parts.append(tc_part)
                tool_call_index[tc_id] = tc_part

                status = update.get("status")
                if status in ("completed", "failed"):
                    flush_assistant(ev_at)
                    res_text = _extract_update_content_text(update) or str(status)
                    step_entries.append(
                        (
                            ev_at,
                            {
                                "role": "toolResult",
                                "content": [{"type": "text", "text": _truncate_str(res_text)}],
                            },
                        )
                    )

            elif su_kind == "tool_call_update":
                tc_id = str(update.get("toolCallId") or "")
                tc_part_ref = tool_call_index.get(tc_id)
                title = update.get("title")
                raw_in = update.get("rawInput")
                if tc_part_ref is not None:
                    if isinstance(title, str) and title:
                        tc_part_ref["name"] = title
                    if isinstance(raw_in, dict) and len(raw_in) > 0:
                        tc_part_ref["arguments"] = raw_in
                status = update.get("status")
                if status in ("completed", "failed"):
                    if tc_part_ref is None and tc_id:
                        fallback_name = str(title) if isinstance(title, str) and title else tc_id
                        fallback_args = raw_in if isinstance(raw_in, dict) and len(raw_in) > 0 else {}
                        if assistant_at is None:
                            assistant_at = ev_at
                        assistant_parts.append(
                            {
                                "type": "toolCall",
                                "id": tc_id,
                                "name": fallback_name,
                                "arguments": fallback_args,
                            }
                        )
                    flush_assistant(ev_at)
                    res_text = _extract_update_content_text(update) or str(status)
                    step_entries.append(
                        (
                            ev_at,
                            {
                                "role": "toolResult",
                                "content": [{"type": "text", "text": _truncate_str(res_text)}],
                            },
                        )
                    )

        flush_assistant(finished_at)

        # Fallback if events had no assistant chunks at all, but rawText exists
        has_assistant = any(msg.get("role") == "assistant" for _, msg in step_entries)
        if not has_assistant:
            raw_text = step.get("_rawText")
            if isinstance(raw_text, str) and raw_text:
                step_entries.append(
                    (
                        finished_at,
                        {"role": "assistant", "content": [{"type": "text", "text": _truncate_str(raw_text)}]},
                    )
                )

        if step_entries:
            first_id = f"{attempt_id}:1"
            last_id = f"{attempt_id}:{len(step_entries)}"
            step["conversation"] = {"firstEntryId": first_id, "lastEntryId": last_id}
            for k_idx, (entry_at, msg_obj) in enumerate(step_entries, start=1):
                all_entries.append(
                    {
                        "seq": seq_counter,
                        "at": entry_at,
                        "entry": {
                            "type": "message",
                            "id": f"{attempt_id}:{k_idx}",
                            "message": msg_obj,
                        },
                    }
                )
                seq_counter += 1
        else:
            step.pop("conversation", None)

    return all_entries


def _build_workflow(flow_raw: Any, fallback_name: str) -> dict[str, Any]:
    if not isinstance(flow_raw, dict):
        flow_raw = {}
    name = str(flow_raw.get("name") or fallback_name or "flow")
    raw_nodes = flow_raw.get("nodes") if isinstance(flow_raw.get("nodes"), dict) else {}
    nodes: dict[str, dict[str, Any]] = {}
    for node_id, n_obj in raw_nodes.items():
        if not isinstance(n_obj, dict):
            continue
        mapped_node: dict[str, Any] = {
            "nodeType": _map_node_type(n_obj.get("nodeType")),
        }
        if isinstance(n_obj.get("actionExecution"), str):
            mapped_node["actionExecution"] = n_obj["actionExecution"]
        if isinstance(n_obj.get("summary"), str):
            mapped_node["summary"] = _truncate_str(n_obj["summary"])
        nodes[str(node_id)] = mapped_node

    start_at = flow_raw.get("startAt")
    if not isinstance(start_at, str) or not start_at:
        start_at = next(iter(nodes.keys()), "start")
    if not nodes:
        nodes[start_at] = {"nodeType": "compute"}

    raw_edges = flow_raw.get("edges") if isinstance(flow_raw.get("edges"), list) else []
    edges: list[dict[str, Any]] = []
    for edge in raw_edges:
        if not isinstance(edge, dict):
            continue
        frm = edge.get("from")
        if not isinstance(frm, str) or not frm:
            continue
        to = edge.get("to")
        sw = edge.get("switch")
        if isinstance(to, str) and to:
            edges.append({"from": frm, "to": to})
        elif isinstance(sw, dict):
            on_val = sw.get("on")
            cases_val = sw.get("cases")
            if isinstance(on_val, str) and isinstance(cases_val, dict):
                clean_cases = {str(ck): str(cv) for ck, cv in cases_val.items() if isinstance(cv, str)}
                edges.append({"from": frm, "switch": {"on": on_val, "cases": clean_cases}})

    return {
        "schema": SCHEMA_WORKFLOW,
        "name": name,
        "startAt": start_at,
        "nodes": nodes,
        "edges": edges,
    }


def _compute_transitions_and_graph_steps(
    steps: list[dict[str, Any]], cursor: int
) -> tuple[list[dict[str, Any]], list[str]]:
    if not steps:
        return [], []
    clamped = max(0, min(cursor, len(steps) - 1))
    sliced = steps[: clamped + 1]

    latest_by_node: dict[str, tuple[int, dict[str, Any]]] = {}
    transitions: list[str] = []
    seen_transitions: set[str] = set()
    prev_node: str | None = None

    for idx, st in enumerate(sliced):
        node_id = str(st.get("nodeId") or "")
        if prev_node is not None and node_id:
            tr = f"{prev_node}->{node_id}"
            if tr not in seen_transitions:
                seen_transitions.add(tr)
                transitions.append(tr)
        if node_id:
            prev_node = node_id
            compact = dict(st)
            compact["prompt"] = None
            compact["output"] = None
            latest_by_node[node_id] = (idx, compact)

    ordered = [item[1] for item in sorted(latest_by_node.values(), key=lambda pair: pair[0])]
    return ordered, transitions


def run_snapshot(run_dir: str | Path, revision: int = 1) -> dict[str, Any] | None:
    rdir = Path(run_dir)
    try:
        manifest_raw = read_json(rdir / "manifest.json")
        if not isinstance(manifest_raw, dict):
            return None

        paths = manifest_raw.get("paths") if isinstance(manifest_raw.get("paths"), dict) else {}
        flow_rel = paths.get("flow", "flow.json")
        trace_rel = paths.get("trace", "trace.ndjson")
        run_rel = paths.get("runProjection", "projections/run.json")
        live_rel = paths.get("liveProjection", "projections/live.json")
        steps_rel = paths.get("stepsProjection", "projections/steps.json")

        flow_raw = read_json(rdir / flow_rel)
        trace_raw = read_json(rdir / trace_rel)
        run_raw = read_json(rdir / run_rel)
        live_raw = read_json(rdir / live_rel)
        steps_raw = read_json(rdir / steps_rel)

        manifest, display, live, possibly_interrupted, updated_at = _build_manifest_and_display(
            rdir,
            manifest_raw,
            live_raw if isinstance(live_raw, dict) else None,
            run_raw if isinstance(run_raw, dict) else None,
        )

        run_id = manifest["runId"]
        workflow_name = manifest["workflowName"]
        status = manifest["status"]
        started_at = manifest["startedAt"]
        finished_at = manifest["finishedAt"]

        workflow = _build_workflow(flow_raw, workflow_name)

        # Build steps
        if not isinstance(steps_raw, list):
            if isinstance(run_raw, dict) and isinstance(run_raw.get("steps"), list):
                steps_raw = run_raw["steps"]
            else:
                steps_raw = []

        mapped_steps: list[dict[str, Any]] = []
        for idx, st in enumerate(steps_raw):
            if not isinstance(st, dict):
                continue
            node_id = str(st.get("nodeId") or f"node_{idx}")
            attempt_id = str(st.get("attemptId") or f"{node_id}#{idx + 1}")
            node_type = _map_node_type(st.get("nodeType"))
            outcome = _map_outcome(st.get("outcome"))
            st_started = str(st.get("startedAt") or started_at)
            st_finished = str(st.get("finishedAt") or updated_at or st_started)

            prompt_val = st.get("promptText") if "promptText" in st else st.get("prompt")
            if prompt_val is not None:
                prompt_val = _truncate_Strings_in_value(prompt_val)

            output_val = st.get("output")
            if output_val is not None:
                output_val = _truncate_Strings_in_value(output_val)

            step_rec: dict[str, Any] = {
                "attemptId": attempt_id,
                "nodeId": node_id,
                "nodeType": node_type,
                "outcome": outcome,
                "startedAt": st_started,
                "finishedAt": st_finished,
                "prompt": prompt_val,
                "output": output_val,
            }

            err_norm = _normalize_error(st.get("error"))
            if err_norm is not None:
                step_rec["error"] = err_norm

            trace_info = st.get("trace")
            action_obj = None
            if isinstance(trace_info, dict) and isinstance(trace_info.get("action"), dict):
                action_obj = trace_info["action"]
            elif isinstance(st.get("action"), dict):
                action_obj = st["action"]

            if isinstance(action_obj, dict):
                filtered_action: dict[str, Any] = {}
                for k in ACTION_KEYS:
                    v = action_obj.get(k)
                    if v is not None:
                        filtered_action[k] = _truncate_Strings_in_value(v)
                if "actionType" in filtered_action:
                    step_rec["action"] = filtered_action

            if isinstance(trace_info, dict) and isinstance(trace_info.get("conversation"), dict):
                step_rec["_traceConversation"] = trace_info["conversation"]
            if isinstance(st.get("rawText"), str):
                step_rec["_rawText"] = st["rawText"]

            mapped_steps.append(step_rec)

        entries = session_entries(rdir, mapped_steps)

        step_entry_Count: list[tuple[int, int]] = []
        for idx, st in enumerate(mapped_steps):
            st.pop("_traceConversation", None)
            st.pop("_rawText", None)
            conv = st.get("conversation")
            if isinstance(conv, dict):
                prefix = f"{st['attemptId']}:"
                cnt = sum(
                    1
                    for e in entries
                    if isinstance(e.get("entry"), dict)
                    and str(e["entry"].get("id", "")).startswith(prefix)
                )
                step_entry_Count.append((idx, cnt))

        _, taken_transitions = _compute_transitions_and_graph_steps(
            mapped_steps, len(mapped_steps) - 1 if mapped_steps else 0
        )

        # Trace page
        trace_items: list[dict[str, Any]] = []
        last_trace_seq = 0
        if isinstance(trace_raw, list):
            for tev in trace_raw:
                if not isinstance(tev, dict):
                    continue
                t_seq = tev.get("seq")
                if isinstance(t_seq, int) and t_seq > last_trace_seq:
                    last_trace_seq = t_seq
                t_type = tev.get("type")
                if t_type == "node_heartbeat" or not isinstance(t_type, str):
                    continue
                raw_payload = tev.get("payload")
                if raw_payload is None:
                    payload_val: Any = {}
                else:
                    payload_val = _truncate_Strings_in_value(raw_payload)
                    if len(canon(payload_val)) > MAX_TRACE_PAYLOAD_CHARS:
                        payload_val = {"truncated": True}

                t_item: dict[str, Any] = {
                    "seq": t_seq if isinstance(t_seq, int) and t_seq >= 0 else len(trace_items) + 1,
                    "type": t_type,
                    "payload": payload_val,
                    "at": str(tev.get("at") or started_at),
                }
                if isinstance(tev.get("nodeId"), str) and tev["nodeId"]:
                    t_item["nodeId"] = tev["nodeId"]
                if isinstance(tev.get("attemptId"), str) and tev["attemptId"]:
                    t_item["attemptId"] = tev["attemptId"]
                trace_items.append(t_item)

        # State
        src_proj: dict[str, Any] = {}
        if isinstance(run_raw, dict):
            src_proj.update(run_raw)
        if isinstance(live_raw, dict):
            src_proj.update(live_raw)

        input_val = run_raw.get("input") if isinstance(run_raw, dict) and "input" in run_raw else None
        input_val = _truncate_Strings_in_value(input_val)

        outputs_val = (
            run_raw.get("outputs")
            if isinstance(run_raw, dict) and isinstance(run_raw.get("outputs"), dict)
            else {}
        )
        outputs_val = _truncate_Strings_in_value(outputs_val)

        raw_results = (
            run_raw.get("results")
            if isinstance(run_raw, dict) and isinstance(run_raw.get("results"), dict)
            else {}
        )
        results_val: dict[str, Any] = {}
        for r_key, r_obj in raw_results.items():
            if not isinstance(r_obj, dict):
                continue
            r_copy = _truncate_Strings_in_value(dict(r_obj))
            r_copy["nodeType"] = _map_node_type(r_copy.get("nodeType"))
            r_copy["outcome"] = _map_outcome(r_copy.get("outcome"))
            if "error" in r_copy:
                r_err = _normalize_error(r_copy.get("error"))
                if r_err is not None:
                    r_copy["error"] = r_err
                else:
                    r_copy.pop("error", None)
            results_val[str(r_key)] = r_copy

        state: dict[str, Any] = {
            "schema": SCHEMA_STATE,
            "traceSeq": last_trace_seq,
            "runId": run_id,
            "workflowName": workflow_name,
            "workflowSource": manifest["workflowSource"],
            "startedAt": started_at,
            "updatedAt": updated_at,
            "finishedAt": finished_at,
            "status": status,
            "input": input_val,
            "outputs": outputs_val,
            "results": results_val,
            "steps": mapped_steps,
        }
        if "runTitle" in manifest:
            state["runTitle"] = manifest["runTitle"]

        for opt_key in ("currentNode", "waitingOn", "statusDetail"):
            val = src_proj.get(opt_key)
            if isinstance(val, str) and val:
                state[opt_key] = _truncate_str(val)

        state_err = _normalize_error(src_proj.get("error"))
        if state_err is not None:
            state["error"] = state_err

        snapshot: dict[str, Any] = {
            "schema": SCHEMA_RUN_VIEW,
            "runId": run_id,
            "revision": int(revision),
            "manifest": manifest,
            "state": state,
            "workflow": workflow,
            "display": display,
            "takenTransitions": taken_transitions,
            "stepStart": 0,
            "stepTotal": len(mapped_steps),
            "tracePage": {
                "start": 0,
                "total": len(trace_items),
                "items": trace_items,
            },
            "session": {
                "binding": None,
                "capture": None,
                "integrity": None,
                "eventPage": {"start": 0, "total": 0, "items": []},
                "entryPage": {
                    "start": 0,
                    "total": len(entries),
                    "items": entries,
                },
            },
            "live": live,
            "possiblyInterrupted": possibly_interrupted,
        }

        _enforce_snapshot_budget(snapshot, step_entry_Count)
        return snapshot
    except Exception as exc:
        _log_err(f"failed building snapshot for {rdir.name}: {exc}")
        return None


def _renumber_entries(entries: list[dict[str, Any]]) -> None:
    for idx, item in enumerate(entries, start=1):
        item["seq"] = idx


def _enforce_snapshot_budget(
    snapshot: dict[str, Any], step_entry_counts: list[tuple[int, int]]
) -> None:
    if len(canon(snapshot).encode("utf-8")) <= SNAPSHOT_BUDGET_BYTES:
        return

    entries: list[dict[str, Any]] = snapshot["session"]["entryPage"]["items"]
    steps: list[dict[str, Any]] = snapshot["state"]["steps"]

    # Drop session entries of oldest steps first
    for step_idx, count in step_entry_counts:
        if len(canon(snapshot).encode("utf-8")) <= SNAPSHOT_BUDGET_BYTES:
            break
        if 0 <= step_idx < len(steps):
            steps[step_idx].pop("conversation", None)
        if count > 0 and entries:
            del entries[:count]
            _renumber_entries(entries)
            snapshot["session"]["entryPage"]["total"] = len(entries)

    if len(canon(snapshot).encode("utf-8")) <= SNAPSHOT_BUDGET_BYTES:
        return

    # Clear any remaining entries if still over budget
    if entries:
        entries.clear()
        snapshot["session"]["entryPage"]["total"] = 0
        for st in steps:
            st.pop("conversation", None)

    # Drop oldest tracePage items until it fits
    trace_items: list[dict[str, Any]] = snapshot["tracePage"]["items"]
    while trace_items and len(canon(snapshot).encode("utf-8")) > SNAPSHOT_BUDGET_BYTES:
        drop_n = max(1, len(trace_items) // 4)
        del trace_items[:drop_n]
        snapshot["tracePage"]["total"] = len(trace_items)

    # Last resort: trim large step outputs/prompts if still huge
    if len(canon(snapshot).encode("utf-8")) > SNAPSHOT_BUDGET_BYTES:
        snapshot["state"]["outputs"] = {}
        snapshot["state"]["results"] = {}
        for st in steps:
            if len(canon(snapshot).encode("utf-8")) <= SNAPSHOT_BUDGET_BYTES:
                break
            st["output"] = None
            st["prompt"] = None


def page_receipt(snapshot: dict[str, Any], kind: str, cursor: int) -> dict[str, Any]:
    run_id = str(snapshot.get("runId") or "")
    rev = int(snapshot.get("revision") or 0)
    cur = max(0, int(cursor))
    steps: list[dict[str, Any]] = snapshot.get("state", {}).get("steps", [])

    base: dict[str, Any] = {
        "schema": SCHEMA_RUN_PAGE,
        "runId": run_id,
        "revision": rev,
        "kind": kind,
        "cursor": cur,
        "start": 0,
        "total": 0,
        "items": [],
    }

    if kind == "steps":
        graph_steps, transitions = _compute_transitions_and_graph_steps(steps, cur)
        base["total"] = len(steps)
        base["items"] = steps
        base["graphSteps"] = graph_steps
        base["graphCursor"] = cur
        base["takenTransitions"] = transitions
        return base

    if kind == "trace_at_step":
        all_trace: list[dict[str, Any]] = snapshot.get("tracePage", {}).get("items", [])
        if 0 <= cur < len(steps):
            attempt_id = steps[cur].get("attemptId")
            filtered = [t for t in all_trace if isinstance(t, dict) and t.get("attemptId") == attempt_id]
        else:
            filtered = []
        base["total"] = len(filtered)
        base["items"] = filtered
        return base

    if kind == "trace":
        all_trace = snapshot.get("tracePage", {}).get("items", [])
        base["total"] = len(all_trace)
        base["items"] = all_trace
        return base

    if kind == "session_entries":
        ep = snapshot.get("session", {}).get("entryPage", {})
        items = ep.get("items", []) if isinstance(ep, dict) else []
        base["start"] = int(ep.get("start", 0)) if isinstance(ep, dict) else 0
        base["total"] = len(items)
        base["items"] = items
        return base

    if kind == "session_events":
        evp = snapshot.get("session", {}).get("eventPage", {})
        items = evp.get("items", []) if isinstance(evp, dict) else []
        base["start"] = int(evp.get("start", 0)) if isinstance(evp, dict) else 0
        base["total"] = len(items)
        base["items"] = items
        base["replayCheckpoint"] = None
        return base

    return base


def _list_run_dirs(runs_dir: Path) -> list[Path]:
    if not runs_dir.exists() or not runs_dir.is_dir():
        return []
    try:
        candidates = [p for p in runs_dir.iterdir() if p.is_dir() and (p / "manifest.json").exists()]
    except OSError as exc:
        _log_err(f"cannot list runs dir {runs_dir}: {exc}")
        return []
    candidates.sort(key=lambda p: p.name, reverse=True)
    return candidates


def build_runs_list(runs_dir: str | Path) -> list[dict[str, Any]]:
    rdir = Path(runs_dir)
    rows: list[tuple[str, str, dict[str, Any]]] = []
    for sub in _list_run_dirs(rdir):
        row = run_row(sub)
        if row is not None:
            started = str(row["manifest"].get("startedAt") or "")
            run_id = str(row["manifest"].get("runId") or sub.name)
            rows.append((started, run_id, row))
    rows.sort(key=lambda item: (item[0], item[1]), reverse=True)
    return [item[2] for item in rows[:MAX_RUNS]]


def detect_piw_version(piw_bin: str, explicit_version: str | None) -> str:
    if explicit_version:
        return explicit_version.strip()
    try:
        proc = subprocess.run(
            [piw_bin, "--version"],
            capture_output=True,
            text=True,
            timeout=5,
            check=False,
        )
        out = (proc.stdout or proc.stderr or "").strip()
        if out:
            parts = out.split()
            for part in reversed(parts):
                if part[0].isdigit():
                    return part
            return parts[-1]
    except Exception:
        pass
    return DEFAULT_PIW_VERSION


def prepare_socket_path(sock_path: Path) -> None:
    if not sock_path.exists():
        sock_path.parent.mkdir(parents=True, exist_ok=True)
        return
    test_sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        test_sock.settimeout(1.0)
        test_sock.connect(str(sock_path))
    except OSError:
        try:
            sock_path.unlink()
        except FileNotFoundError:
            pass
    else:
        print(f"already serving on {sock_path}", file=sys.stderr, flush=True)
        sys.exit(1)
    finally:
        test_sock.close()


class ClientSession:
    def __init__(
        self,
        reader: asyncio.StreamReader,
        writer: asyncio.StreamWriter,
        runs_dir: Path,
        piw_version: str,
    ) -> None:
        self.reader = reader
        self.writer = writer
        self.runs_dir = runs_dir
        self.piw_version = piw_version
        self.conn_id = f"conn-{uuid.uuid4().hex[:12]}"
        self.closed = False
        self.runs_watch_task: asyncio.Task[None] | None = None
        self.run_watch_tasks: dict[str, asyncio.Task[None]] = {}
        self.run_snapshots: dict[str, dict[str, Any]] = {}
        self.run_snapshot_digests: dict[str, str] = {}
        self.run_revisions: dict[str, int] = {}
        self.runs_revision: int = 0
        self.runs_last_digest: str | None = None

    async def send_msg(self, obj: dict[str, Any]) -> bool:
        if self.closed:
            return False
        try:
            line = canon(obj)
            data = (line + "\n").encode("utf-8")
            self.writer.write(data)
            await self.writer.drain()
            return True
        except Exception:
            self.closed = True
            return False

    async def send_response(
        self,
        request_id: str,
        outcome: str,
        *,
        revision: int | None = None,
        receipt: dict[str, Any] | None = None,
        error: str | None = None,
    ) -> None:
        msg: dict[str, Any] = {
            "schema": SCHEMA_CLIENT,
            "type": "response",
            "requestId": request_id,
            "outcome": outcome,
        }
        if revision is not None:
            msg["revision"] = int(revision)
        if receipt is not None:
            msg["receipt"] = receipt
        if error is not None and error:
            msg["error"] = error
        await self.send_msg(msg)

    async def send_event(
        self,
        subscription_id: str,
        event: str,
        payload: dict[str, Any],
        *,
        run_id: str | None = None,
        revision: int | None = None,
    ) -> None:
        msg: dict[str, Any] = {
            "schema": SCHEMA_CLIENT,
            "type": "event",
            "subscriptionId": subscription_id,
            "event": event,
            "payload": payload,
        }
        if run_id is not None:
            msg["runId"] = run_id
        if revision is not None:
            msg["revision"] = int(revision)
        await self.send_msg(msg)

    async def emit_runs_if_changed(self, subscription_id: str, force: bool = False) -> None:
        items = await asyncio.to_thread(build_runs_list, self.runs_dir)
        digest = canon(items)
        if not force and digest == self.runs_last_digest:
            return
        self.runs_last_digest = digest
        self.runs_revision += 1
        payload = {
            "schema": SCHEMA_RUN_LIST_PAGE,
            "revision": str(self.runs_revision),
            "start": 0,
            "total": len(items),
            "items": items,
        }
        await self.send_event(subscription_id, "runs", payload)

    async def _poll_runs_loop(self, subscription_id: str) -> None:
        try:
            while not self.closed:
                await asyncio.sleep(1.0)
                if self.closed:
                    break
                await self.emit_runs_if_changed(subscription_id, force=False)
        except asyncio.CancelledError:
            pass
        except Exception as exc:
            _log_err(f"runs poll error: {exc}")

    def _build_and_track_snapshot(self, run_id: str) -> tuple[dict[str, Any] | None, bool]:
        run_dir = self.runs_dir / run_id
        if not run_dir.is_dir() or not (run_dir / "manifest.json").exists():
            return None, False
        snap0 = run_snapshot(run_dir, revision=0)
        if snap0 is None:
            return None, False
        digest = canon(snap0)
        prev_digest = self.run_snapshot_digests.get(run_id)
        if prev_digest == digest and run_id in self.run_snapshots:
            return self.run_snapshots[run_id], False
        rev = self.run_revisions.get(run_id, 0) + 1
        self.run_revisions[run_id] = rev
        snap0["revision"] = rev
        self.run_snapshot_digests[run_id] = digest
        self.run_snapshots[run_id] = snap0
        return snap0, True

    async def _poll_single_run_loop(self, run_id: str, subscription_id: str) -> None:
        try:
            while not self.closed:
                await asyncio.sleep(0.5)
                if self.closed:
                    break
                snap, changed = await asyncio.to_thread(self._build_and_track_snapshot, run_id)
                if changed and snap is not None:
                    rev = int(snap["revision"])
                    await self.send_event(
                        subscription_id,
                        "run_snapshot",
                        snap,
                        run_id=run_id,
                        revision=rev,
                    )
        except asyncio.CancelledError:
            pass
        except Exception as exc:
            _log_err(f"run poll error ({run_id}): {exc}")

    def _stop_all_tasks(self) -> None:
        if self.runs_watch_task is not None:
            self.runs_watch_task.cancel()
            self.runs_watch_task = None
        for t in self.run_watch_tasks.values():
            t.cancel()
        self.run_watch_tasks.clear()

    async def handle_request(self, req: dict[str, Any]) -> bool:
        if req.get("schema") != SCHEMA_CLIENT or req.get("type") != "request":
            return False
        request_id = req.get("requestId")
        operation = req.get("operation")
        payload = req.get("payload")
        if not isinstance(request_id, str) or not request_id or not isinstance(operation, str):
            return False
        if not isinstance(payload, dict):
            payload = {}

        run_id = req.get("runId") if isinstance(req.get("runId"), str) else None

        if operation == "view.runs.watch":
            sub_id = str(payload.get("subscriptionId") or "runs")
            await self.send_response(request_id, "accepted", receipt={"subscribed": True})
            await self.emit_runs_if_changed(sub_id, force=True)
            if self.runs_watch_task is not None:
                self.runs_watch_task.cancel()
            self.runs_watch_task = asyncio.create_task(self._poll_runs_loop(sub_id))
            return True

        if operation == "view.runs.unwatch":
            if self.runs_watch_task is not None:
                self.runs_watch_task.cancel()
                self.runs_watch_task = None
            await self.send_response(request_id, "accepted", receipt={"subscribed": False})
            return True

        if operation == "view.run.watch":
            if not run_id:
                await self.send_response(request_id, "notFound", error="missing runId")
                return True
            sub_id = str(payload.get("subscriptionId") or f"run:{run_id}")
            snap, _ = await asyncio.to_thread(self._build_and_track_snapshot, run_id)
            if snap is None:
                _log_err(f"run not found or unreadable: {run_id}")
                await self.send_response(request_id, "notFound", error=f"run not found: {run_id}")
                return True
            rev = int(snap["revision"])
            await self.send_response(request_id, "accepted", revision=rev, receipt={"subscribed": True})
            await self.send_event(sub_id, "run_snapshot", snap, run_id=run_id, revision=rev)
            old_task = self.run_watch_tasks.pop(run_id, None)
            if old_task is not None:
                old_task.cancel()
            self.run_watch_tasks[run_id] = asyncio.create_task(self._poll_single_run_loop(run_id, sub_id))
            return True

        if operation == "view.run.unwatch":
            target_id = run_id
            sub_id = str(payload.get("subscriptionId") or "")
            if not target_id and sub_id.startswith("run:"):
                target_id = sub_id[4:]
            if target_id:
                task = self.run_watch_tasks.pop(target_id, None)
                if task is not None:
                    task.cancel()
            await self.send_response(request_id, "accepted", receipt={"subscribed": False})
            return True

        if operation == "view.page":
            if not run_id:
                await self.send_response(request_id, "notFound", error="missing runId")
                return True
            snap = self.run_snapshots.get(run_id)
            if snap is None:
                snap, _ = await asyncio.to_thread(self._build_and_track_snapshot, run_id)
            if snap is None:
                await self.send_response(request_id, "notFound", error=f"run not found: {run_id}")
                return True
            kind = str(payload.get("kind") or "steps")
            raw_cursor = payload.get("cursor")
            cursor = int(raw_cursor) if isinstance(raw_cursor, int) and not isinstance(raw_cursor, bool) else 0
            receipt = page_receipt(snap, kind, cursor)
            await self.send_response(
                request_id,
                "accepted",
                revision=int(snap["revision"]),
                receipt=receipt,
            )
            return True

        await self.send_response(request_id, "rejected", error="acpx runs are read-only here")
        return True

    async def run(self) -> None:
        hello = {
            "schema": SCHEMA_CLIENT,
            "type": "hello",
            "connectionId": self.conn_id,
            "packageVersion": self.piw_version,
        }
        if not await self.send_msg(hello):
            self._stop_all_tasks()
            return

        try:
            while not self.closed:
                line_bytes = await self.reader.readline()
                if not line_bytes:
                    break
                if len(line_bytes) > MAX_LINE_BYTES:
                    break
                try:
                    text = line_bytes.decode("utf-8")
                    req = json.loads(text)
                except Exception:
                    break
                if not isinstance(req, dict):
                    break
                ok = await self.handle_request(req)
                if not ok:
                    break
        except asyncio.CancelledError:
            pass
        except Exception:
            pass
        finally:
            self.closed = True
            self._stop_all_tasks()
            try:
                self.writer.close()
                await self.writer.wait_closed()
            except Exception:
                pass


async def start_adapter_server(
    sock_path: Path, runs_dir: Path, piw_version: str
) -> tuple[asyncio.AbstractServer, set[asyncio.Task[None]]]:
    prepare_socket_path(sock_path)
    active_tasks: set[asyncio.Task[None]] = set()

    def client_connected(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        session = ClientSession(reader, writer, runs_dir, piw_version)
        task = asyncio.create_task(session.run())
        active_tasks.add(task)
        task.add_done_callback(active_tasks.discard)

    server = await asyncio.start_unix_server(client_connected, path=str(sock_path), limit=MAX_LINE_BYTES * 2)
    return server, active_tasks


async def shutdown_server(
    server: asyncio.AbstractServer, active_tasks: set[asyncio.Task[None]], sock_path: Path
) -> None:
    server.close()
    try:
        await server.wait_closed()
    except Exception:
        pass
    for t in list(active_tasks):
        t.cancel()
    if active_tasks:
        await asyncio.gather(*active_tasks, return_exceptions=True)
    try:
        sock_path.unlink()
    except FileNotFoundError:
        pass
    except OSError:
        pass


async def cmd_serve(args: argparse.Namespace) -> int:
    sock_path = Path(args.socket) if args.socket else Path(f"/tmp/acpx-piw-{os.getuid()}.sock")
    runs_dir = Path(args.runs_dir).expanduser()
    piw_version = detect_piw_version(args.piw, args.piw_version)

    server, active_tasks = await start_adapter_server(sock_path, runs_dir, piw_version)
    stop_event = asyncio.Event()
    loop = asyncio.get_running_loop()

    for sig in (signal.SIGINT, signal.SIGTERM):
        try:
            loop.add_signal_handler(sig, stop_event.set)
        except NotImplementedError:
            pass

    try:
        await stop_event.wait()
    finally:
        await shutdown_server(server, active_tasks, sock_path)
    return 0


async def cmd_open(args: argparse.Namespace) -> int:
    if args.once and not args.run_id:
        _log_err("--once requires RUN_ID")
        return 2

    sock_path = Path(f"/tmp/acpx-piw-{os.getpid()}.sock")
    runs_dir = Path(args.runs_dir).expanduser()
    piw_version = detect_piw_version(args.piw, args.piw_version)

    server, active_tasks = await start_adapter_server(sock_path, runs_dir, piw_version)
    proc: asyncio.subprocess.Process | None = None
    loop = asyncio.get_running_loop()

    def forward_sig(sig: int) -> None:
        if proc is not None and proc.returncode is None:
            try:
                proc.send_signal(sig)
            except ProcessLookupError:
                pass

    for sig in (signal.SIGINT, signal.SIGTERM):
        try:
            loop.add_signal_handler(sig, forward_sig, sig)
        except NotImplementedError:
            pass

    try:
        cmd = [args.piw]
        if args.run_id:
            cmd.append(args.run_id)
        if args.once:
            cmd.append("--once")

        env = os.environ.copy()
        env["PIW_SOCKET"] = str(sock_path)
        env["PIW_NO_AUTOSTART"] = "1"

        proc = await asyncio.create_subprocess_exec(*cmd, env=env)
        rc = await proc.wait()
        return int(rc)
    finally:
        await shutdown_server(server, active_tasks, sock_path)


def _assert_canonical_and_sized(obj: Any, label: str) -> str:
    s1 = canon(obj)
    b1 = s1.encode("utf-8")
    if len(b1) >= MAX_LINE_BYTES:
        raise AssertionError(f"{label}: message exceeds 1 MiB ({len(b1)} bytes)")
    parsed = json.loads(s1)
    s2 = canon(parsed)
    if s1.encode("utf-8") != s2.encode("utf-8"):
        raise AssertionError(f"{label}: non-canonical roundtrip")
    return s1


async def cmd_selftest(args: argparse.Namespace) -> int:
    try:
        # 1. Check float formatting & canonical JSON rules
        float_cases: list[tuple[Any, str]] = [
            (1.0, "1"),
            (-0.0, "0"),
            (0.0, "0"),
            (1e-5, "0.00001"),
            (1e21, "1e+21"),
            (1.5e-7, "1.5e-7"),
            (float("nan"), "null"),
            (float("inf"), "null"),
            (float("-inf"), "null"),
            (True, "true"),
            (False, "false"),
            (None, "null"),
        ]
        for val, expected in float_cases:
            got = canon(val)
            if got != expected:
                raise AssertionError(f"canon({val!r}) == {got!r}, expected {expected!r}")

        # Check UTF-16 key sorting: U+E000 (0xE000) vs U+10000 (0xD800 0xDC00)
        # In UTF-16 code units, 0xD800 < 0xE000 even though U+10000 > U+E000 in code point order.
        utf16_obj = {"\ue000": 1, "\U00010000": 2}
        if canon(utf16_obj) != '{"\U00010000":2,"\ue000":1}':
            raise AssertionError(f"UTF-16 key sort failed: {canon(utf16_obj)}")

        runs_dir = Path(args.runs_dir).expanduser()
        rows = build_runs_list(runs_dir)
        runs_event = {
            "schema": SCHEMA_CLIENT,
            "type": "event",
            "subscriptionId": "runs",
            "event": "runs",
            "payload": {
                "schema": SCHEMA_RUN_LIST_PAGE,
                "revision": "1",
                "start": 0,
                "total": len(rows),
                "items": rows,
            },
        }
        _assert_canonical_and_sized(runs_event, "runs_event")

        newest_run_id: str | None = None
        for rdir in _list_run_dirs(runs_dir):
            snap = run_snapshot(rdir, revision=1)
            if snap is None:
                raise AssertionError(f"failed to build snapshot for {rdir.name}")
            ev = {
                "schema": SCHEMA_CLIENT,
                "type": "event",
                "subscriptionId": f"run:{snap['runId']}",
                "event": "run_snapshot",
                "runId": snap["runId"],
                "revision": 1,
                "payload": snap,
            }
            s = _assert_canonical_and_sized(ev, f"snapshot:{snap['runId']}")
            steps_cnt = len(snap["state"]["steps"])
            entries_cnt = len(snap["session"]["entryPage"]["items"])
            if steps_cnt > 0:
                rcpt = page_receipt(snap, "steps", steps_cnt - 1)
                _assert_canonical_and_sized(rcpt, f"page_steps:{snap['runId']}")
                tr_rcpt = page_receipt(snap, "trace_at_step", steps_cnt - 1)
                _assert_canonical_and_sized(tr_rcpt, f"page_trace:{snap['runId']}")
            print(
                f"{snap['runId']} status={snap['manifest']['status']} "
                f"steps={steps_cnt} entries={entries_cnt} bytes={len(s.encode('utf-8'))}"
            )
            if newest_run_id is None:
                newest_run_id = str(snap["runId"])

        if rows and newest_run_id is None:
            newest_run_id = str(rows[0]["manifest"]["runId"])

        piw_bin_path = shutil.which(args.piw)
        if piw_bin_path and newest_run_id:
            piw_version = detect_piw_version(piw_bin_path, args.piw_version)
            sock_path = Path(f"/tmp/acpx-piw-selftest-{os.getpid()}.sock")
            server, active_tasks = await start_adapter_server(sock_path, runs_dir, piw_version)
            try:
                env = os.environ.copy()
                env["PIW_SOCKET"] = str(sock_path)
                env["PIW_NO_AUTOSTART"] = "1"
                proc = await asyncio.create_subprocess_exec(
                    piw_bin_path,
                    newest_run_id,
                    "--once",
                    stdout=asyncio.subprocess.PIPE,
                    stderr=asyncio.subprocess.PIPE,
                    env=env,
                )
                stdout_b, stderr_b = await proc.communicate()
                if proc.returncode != 0:
                    err_txt = stderr_b.decode("utf-8", errors="replace").strip()
                    raise AssertionError(f"piw --once exited with {proc.returncode}: {err_txt}")
                if not stdout_b:
                    raise AssertionError("piw --once produced empty stdout")
            finally:
                await shutdown_server(server, active_tasks, sock_path)

        print("selftest ok")
        return 0
    except Exception as exc:
        _log_err(f"selftest failed: {exc}")
        return 1


def build_arg_parser() -> argparse.ArgumentParser:
    common = argparse.ArgumentParser(add_help=False)
    default_runs = os.environ.get("ACPX_RUNS_DIR") or os.path.expanduser("~/.acpx/flows/runs")
    common.add_argument("--runs-dir", default=default_runs, help="Directory containing acpx flow runs")
    common.add_argument("--piw", default="piw", help="Path to piw binary")
    common.add_argument("--piw-version", default=None, help="Override packageVersion sent in hello")

    parser = argparse.ArgumentParser(
        prog="acpx-piw.py",
        description="Adapter bridging acpx flow runs to piw",
        parents=[common],
    )
    sub = parser.add_subparsers(dest="subcommand", required=True)

    p_serve = sub.add_parser("serve", parents=[common], help="Serve protocol on a Unix socket")
    p_serve.add_argument("--socket", default=None, help="Unix socket path")

    p_open = sub.add_parser("open", parents=[common], help="Start in-process server and launch piw")
    p_open.add_argument("run_id", nargs="?", default=None, metavar="RUN_ID", help="Optional run ID to open")
    p_open.add_argument("--once", action="store_true", help="Pass --once to piw (requires RUN_ID)")

    sub.add_parser("selftest", parents=[common], help="Validate snapshots, canonical JSON, and headless piw")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_arg_parser()
    args = parser.parse_args(argv)

    if args.subcommand == "serve":
        return asyncio.run(cmd_serve(args))
    if args.subcommand == "open":
        return asyncio.run(cmd_open(args))
    if args.subcommand == "selftest":
        return asyncio.run(cmd_selftest(args))
    return 1


if __name__ == "__main__":
    sys.exit(main())
