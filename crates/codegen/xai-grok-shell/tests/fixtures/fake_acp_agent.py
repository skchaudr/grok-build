#!/usr/bin/env python3
"""Tiny ACP stdio agent. Echoes pid, hostname, and the command name it was given."""

import json
import os
import socket
import sys
import time

command = sys.argv[1] if len(sys.argv) > 1 else "fake"
hold_release = (
    sys.argv[2]
    if command in ("fake-roster-hold", "fake-roster-hold-new") and len(sys.argv) > 2
    else None
)
# fake-roster-persist <dir>: session ids survive process death as empty files.
# fake-roster-noload: advertises loadSession false and rejects unknown prompts.
persist_dir = (
    sys.argv[2] if command == "fake-roster-persist" and len(sys.argv) > 2 else None
)
load_session_cap = command != "fake-roster-noload"
pid = os.getpid()
host = socket.gethostname()
init_count = 0
sessions = {}
loaded_via_load = set()


def emit(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def result(req_id, payload):
    emit({"jsonrpc": "2.0", "id": req_id, "result": payload})


def identity():
    return {"pid": pid, "hostname": host, "command": command}


def session_info(sid, title):
    emit(
        {
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": sid,
                "update": {"sessionUpdate": "session_info_update", "title": title},
            },
        }
    )


def wait_release():
    while not os.path.exists(hold_release):
        time.sleep(0.02)
    return open(hold_release, encoding="utf-8").read().strip()


def prompt_summary(prompt):
    if not isinstance(prompt, list):
        return "blocks="
    parts = []
    for block in prompt:
        if not isinstance(block, dict):
            continue
        kind = block.get("type")
        if kind == "image":
            data = block.get("data") or ""
            parts.append(f"image:{block.get('mimeType')}:{len(data)}")
        else:
            parts.append(str(kind))
    return "blocks=" + ",".join(parts)


def unknown_session(req_id):
    emit(
        {
            "jsonrpc": "2.0",
            "id": req_id,
            "error": {
                "code": -32602,
                "message": "Invalid params",
                "data": "unknown session id",
            },
        }
    )


def persist_path(sid):
    if not persist_dir or not sid:
        return None
    return os.path.join(persist_dir, sid)


for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    try:
        msg = json.loads(raw)
    except json.JSONDecodeError:
        continue
    method = msg.get("method")
    req_id = msg.get("id")
    params = msg.get("params") or {}
    if method == "initialize":
        init_count += 1
        result(
            req_id,
            {
                "protocolVersion": 1,
                "agentCapabilities": {"loadSession": load_session_cap},
                "authMethods": [],
                "meta": {**identity(), "initCount": init_count},
            },
        )
    elif method == "session/new":
        if command == "fake-roster-hold-new":
            kind = wait_release()
            if kind == "error":
                emit(
                    {
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "error": {"code": -32000, "message": "session/new failed"},
                    }
                )
                continue
        sid = f"s-{pid}-{len(sessions) + 1}"
        sessions[sid] = True
        if persist_dir:
            os.makedirs(persist_dir, exist_ok=True)
            open(persist_path(sid), "w", encoding="utf-8").close()
        result(req_id, {"sessionId": sid, **identity()})
        if command == "fake-roster-title":
            session_info(sid, "Pong Game")
    elif method in ("session/load", "session/resume"):
        sid = params.get("sessionId") or params.get("session_id")
        if command == "fake-roster-persist":
            path = persist_path(sid)
            on_disk = path is not None and os.path.isfile(path)
            if sid in sessions or on_disk:
                sessions[sid] = True
                loaded_via_load.add(sid)
                result(req_id, {"sessionId": sid, "known": True, **identity()})
            else:
                unknown_session(req_id)
            continue
        result(
            req_id,
            {"sessionId": sid, "known": sid in sessions, **identity()},
        )
    elif method == "session/prompt":
        sid = params.get("sessionId") or params.get("session_id")
        if command in ("fake-roster-persist", "fake-roster-noload") and sid not in sessions:
            unknown_session(req_id)
            continue
        if hold_release:
            kind = wait_release()
            if kind == "error":
                emit(
                    {
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "error": {"code": -32000, "message": "prompt failed"},
                    }
                )
                continue
        text = f"pid={pid} hostname={host} command={command}"
        if command == "fake-echo-prompt":
            text = prompt_summary(params.get("prompt"))
        emit(
            {
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": sid,
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": text},
                    },
                },
            }
        )
        payload = {"stopReason": "end_turn", **identity()}
        if command == "fake-roster-persist":
            payload["loaded"] = sid in loaded_via_load
        result(req_id, payload)
    elif req_id is not None:
        result(req_id, {"ok": True, **identity()})
