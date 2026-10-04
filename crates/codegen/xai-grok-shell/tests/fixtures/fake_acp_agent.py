#!/usr/bin/env python3
"""Tiny ACP stdio agent. Echoes pid, hostname, and the command name it was given."""

import json
import os
import socket
import sys

command = sys.argv[1] if len(sys.argv) > 1 else "fake"
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
        sid = f"s-{pid}-{len(sessions) + 1}"
        sessions[sid] = True
        result(req_id, {"sessionId": sid, **identity()})
    elif method in ("session/load", "session/resume"):
        sid = params.get("sessionId") or params.get("session_id")
        result(
            req_id,
            {"sessionId": sid, "known": sid in sessions, **identity()},
        )
    elif method == "session/prompt":
        sid = params.get("sessionId") or params.get("session_id")
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
