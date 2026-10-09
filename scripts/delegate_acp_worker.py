#!/usr/bin/env python3
"""ACP stdio worker used by the cross-machine delegation demo.

Modes (argv[1], or WORKER_MODE when argv is just ``-c``):

  hostname  Reply with this machine's hostname.
  delegate  Shell out to leader_delegate.py so worker B answers.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys


def mode_name() -> str:
    if os.environ.get("WORKER_MODE"):
        return os.environ["WORKER_MODE"]
    if len(sys.argv) > 1 and sys.argv[1] != "-c":
        return sys.argv[1]
    return "hostname"


def emit(obj: dict) -> None:
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def result(req_id, payload: dict) -> None:
    emit({"jsonrpc": "2.0", "id": req_id, "result": payload})


def chunk(session_id: str, text: str) -> None:
    emit(
        {
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": session_id,
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": text},
                },
            },
        }
    )


def answer_for(prompt: str) -> str:
    mode = mode_name()
    if mode == "delegate":
        script = os.environ["DELEGATE_SCRIPT"]
        sock = os.environ["DELEGATE_SOCKET"]
        target = os.environ["DELEGATE_TARGET"]
        proc = subprocess.run(
            [
                sys.executable,
                script,
                "--leader-socket",
                sock,
                "--agent-cmd",
                target,
                "--prompt",
                prompt,
            ],
            capture_output=True,
            text=True,
        )
        body = proc.stdout.strip()
        if proc.returncode != 0:
            err = proc.stderr.strip()
            return f"delegate failed ({proc.returncode}): {err}\n{body}".rstrip()
        return f"worker-a delegated; worker-b said: {body}"
    if mode != "hostname":
        return f"unknown worker mode {mode}"
    host = subprocess.check_output(["hostname"], text=True).strip()
    return f"hostname={host}"


def main() -> None:
    sessions: dict[str, bool] = {}
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
            result(
                req_id,
                {
                    "protocolVersion": 1,
                    "agentCapabilities": {"loadSession": True},
                    "authMethods": [],
                },
            )
        elif method == "session/new":
            sid = f"s-{os.getpid()}-{len(sessions) + 1}"
            sessions[sid] = True
            result(req_id, {"sessionId": sid})
        elif method in ("session/load", "session/resume"):
            sid = params.get("sessionId") or params.get("session_id")
            result(req_id, {"sessionId": sid})
        elif method == "session/prompt":
            sid = params.get("sessionId") or params.get("session_id") or ""
            text = ""
            blocks = params.get("prompt") or []
            if isinstance(blocks, list):
                for block in blocks:
                    if isinstance(block, dict) and isinstance(block.get("text"), str):
                        text += block["text"]
            chunk(sid, answer_for(text))
            result(req_id, {"stopReason": "end_turn"})
        elif req_id is not None:
            result(req_id, {"ok": True})


if __name__ == "__main__":
    main()
