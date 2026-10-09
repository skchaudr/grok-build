#!/usr/bin/env python3
"""Open a session on a named leader worker, send one prompt, print the answer.

The worker name is the leader's ``--agent-cmd`` string: that exact command is
the backend identity. Stock ``grok -p`` rejects ``--agent-cmd``, and
``grok leader`` has no prompt command, so an agent shells out to this script.

Wire format: 4-byte big-endian length, then one JSON ``ClientMessage``.
"""

from __future__ import annotations

import argparse
import json
import socket
import sys
import time


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--leader-socket", required=True)
    parser.add_argument("--agent-cmd", required=True, help="worker identity (exact command)")
    parser.add_argument("--prompt", required=True)
    parser.add_argument("--timeout", type=float, default=60.0)
    args = parser.parse_args()
    if not args.agent_cmd.strip():
        raise SystemExit("--agent-cmd must name a worker command")

    deadline = time.monotonic() + args.timeout
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.settimeout(args.timeout)
    try:
        sock.connect(args.leader_socket)
        send(
            sock,
            {
                "type": "register",
                "client_type": "leader-delegate",
                "mode": "stdio",
                "capabilities": {"agent_cmd": args.agent_cmd},
            },
        )
        registered = recv_until(
            sock, deadline, lambda m: m.get("type") in ("registered", "error")
        )
        if registered.get("type") == "error":
            raise SystemExit(f"register failed: {registered.get('message')}")
        if not registered.get("ready", True):
            ready = recv_until(
                sock, deadline, lambda m: m.get("type") in ("leader_ready", "error")
            )
            if ready.get("type") == "error":
                raise SystemExit(f"leader not ready: {ready.get('message')}")

        acp(sock, deadline, 1, "initialize", {"protocolVersion": 1})
        created, _ = acp(
            sock,
            deadline,
            2,
            "session/new",
            {"cwd": "/tmp", "mcpServers": []},
        )
        session_id = (created.get("result") or {}).get("sessionId")
        if not session_id:
            raise SystemExit(f"session/new returned no sessionId: {created}")
        _, text = acp(
            sock,
            deadline,
            3,
            "session/prompt",
            {
                "sessionId": session_id,
                "prompt": [{"type": "text", "text": args.prompt}],
            },
            collect_text=True,
        )
    except socket.timeout:
        raise SystemExit(f"timed out after {args.timeout}s waiting for the worker")
    finally:
        sock.close()

    if not text.strip():
        raise SystemExit("worker finished without an agent message")
    sys.stdout.write(text)
    if not text.endswith("\n"):
        sys.stdout.write("\n")


def acp(sock, deadline, req_id, method, params, collect_text=False):
    send(
        sock,
        {
            "type": "acp",
            "payload": json.dumps(
                {"jsonrpc": "2.0", "id": req_id, "method": method, "params": params}
            ),
        },
    )
    chunks: list[str] = []

    def match(msg: dict) -> bool:
        if msg.get("type") == "error":
            raise SystemExit(f"leader error: {msg.get('message')}")
        if msg.get("type") != "acp":
            return False
        payload = decode_payload(msg.get("payload"))
        if collect_text:
            found = chunk_text(payload)
            if found:
                chunks.append(found)
        if payload.get("id") != req_id:
            return False
        if "error" in payload:
            raise SystemExit(f"{method} failed: {payload['error']}")
        msg["__payload"] = payload
        return True

    msg = recv_until(sock, deadline, match)
    return msg["__payload"], "".join(chunks)


def chunk_text(payload: dict) -> str:
    if payload.get("method") != "session/update":
        return ""
    update = (payload.get("params") or {}).get("update") or {}
    if update.get("sessionUpdate") != "agent_message_chunk":
        return ""
    content = update.get("content")
    if isinstance(content, dict):
        text = content.get("text")
        return text if isinstance(text, str) else ""
    if isinstance(content, list):
        parts = []
        for item in content:
            if isinstance(item, dict) and isinstance(item.get("text"), str):
                parts.append(item["text"])
        return "".join(parts)
    return ""


def decode_payload(payload) -> dict:
    if isinstance(payload, str):
        return json.loads(payload)
    if isinstance(payload, dict):
        return payload
    raise SystemExit(f"unexpected ACP payload: {payload!r}")


def recv_until(sock, deadline, pred) -> dict:
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise socket.timeout
        sock.settimeout(remaining)
        msg = recv(sock)
        if pred(msg):
            return msg


def send(sock, obj) -> None:
    data = json.dumps(obj, separators=(",", ":")).encode()
    sock.sendall(len(data).to_bytes(4, "big") + data)


def recv(sock) -> dict:
    header = recvall(sock, 4)
    size = int.from_bytes(header, "big")
    if size > 32 * 1024 * 1024:
        raise SystemExit(f"leader frame is too large ({size} bytes)")
    return json.loads(recvall(sock, size))


def recvall(sock, size: int) -> bytes:
    buf = bytearray()
    while len(buf) < size:
        chunk = sock.recv(size - len(buf))
        if not chunk:
            raise SystemExit("leader closed the connection")
        buf += chunk
    return bytes(buf)


if __name__ == "__main__":
    main()
