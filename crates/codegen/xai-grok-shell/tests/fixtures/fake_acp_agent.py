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
pid = os.getpid()
host = socket.gethostname()
init_count = 0
sessions = {}


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
                "agentCapabilities": {"loadSession": True},
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
        result(req_id, {"sessionId": sid, **identity()})
        if command == "fake-roster-title":
            session_info(sid, "Pong Game")
    elif method in ("session/load", "session/resume"):
        sid = params.get("sessionId") or params.get("session_id")
        result(
            req_id,
            {"sessionId": sid, "known": sid in sessions, **identity()},
        )
    elif method == "session/prompt":
        sid = params.get("sessionId") or params.get("session_id")
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
        result(req_id, {"stopReason": "end_turn", **identity()})
    elif req_id is not None:
        result(req_id, {"ok": True, **identity()})
