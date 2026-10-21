#!/usr/bin/env python3
"""Mock MCP stdio server for end-to-end testing of the NanoHermes agent.

Exposes:
  - tools/list     : a single echo tool
  - resources/list : two resources (a file:// and a custom schema)
  - resources/read : returns text content for known URIs
  - prompts/list   : two prompts
  - prompts/get    : renders a prompt with optional arguments

Speaks JSON-RPC 2.0 with LSP-style Content-Length framing over stdio.
"""
import json
import sys


RESOURCES = [
    {
        "uri": "file:///mock/notes.txt",
        "name": "notes",
        "description": "A mock notes resource",
        "mimeType": "text/plain",
    },
    {
        "uri": "mock://config",
        "name": "config",
        "description": "A mock config resource",
        "mimeType": "text/plain",
    },
]

RESOURCE_CONTENTS = {
    "file:///mock/notes.txt": "hello from the notes resource\nline two\n",
    "mock://config": "key=value\nanswer=42\n",
}

PROMPTS = [
    {"name": "summarize", "description": "Summarize a passage.", "arguments": []},
    {
        "name": "greet",
        "description": "Greet someone.",
        "arguments": [{"name": "name", "description": "Who to greet"}],
    },
]


def handle(method, params):
    if method == "initialize":
        return {
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "resources": {},
                "prompts": {},
                "tools": {},
            },
            "serverInfo": {"name": "mock-mcp", "version": "1.0.0"},
        }
    if method == "notifications/initialized":
        return None  # notification -> no result
    if method == "tools/list":
        return {
            "tools": [
                {
                    "name": "echo",
                    "description": "Echo the input back.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"text": {"type": "string"}},
                        "required": ["text"],
                    },
                }
            ]
        }
    if method == "tools/call":
        name = params.get("name")
        args = params.get("arguments", {})
        if name == "echo":
            return {"content": [{"type": "text", "text": args.get("text", "")}]}
        return {"isError": True, "content": [{"type": "text", "text": "unknown tool"}]}
    if method == "resources/list":
        return {"resources": RESOURCES}
    if method == "resources/read":
        uri = params.get("uri", "")
        text = RESOURCE_CONTENTS.get(uri)
        if text is None:
            return {
                "isError": True,
                "content": [{"type": "text", "text": f"no such resource: {uri}"}],
            }
        return {"contents": [{"type": "text", "text": text, "uri": uri}]}
    if method == "prompts/list":
        return {"prompts": PROMPTS}
    if method == "prompts/get":
        name = params.get("name")
        args = params.get("arguments", {}) or {}
        if name == "summarize":
            text = "Please summarize the following text concisely."
        elif name == "greet":
            who = args.get("name", "there")
            text = f"Hello, {who}! Nice to meet you."
        else:
            return {
                "isError": True,
                "content": [{"type": "text", "text": f"unknown prompt: {name}"}],
            }
        return {
            "messages": [
                {"role": "user", "content": {"type": "text", "text": text}},
            ]
        }
    return {"isError": True, "content": [{"type": "text", "text": f"unknown method: {method}"}]}


def read_message():
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None  # EOF
        line = line.strip()
        if not line:
            break
        line = line.decode("utf-8")
        if ":" in line:
            k, v = line.split(":", 1)
            headers[k.strip().lower()] = v.strip()
    length = int(headers.get("content-length", "0"))
    if length == 0:
        return None
    body = sys.stdin.buffer.read(length).decode("utf-8")
    return json.loads(body)


def send(msg):
    body = json.dumps(msg).encode("utf-8")
    frame = f"Content-Length: {len(body)}\r\n\r\n".encode("utf-8")
    sys.stdout.buffer.write(frame + body)
    sys.stdout.buffer.flush()


def main():
    while True:
        msg = read_message()
        if msg is None:
            break
        mid = msg.get("id")
        method = msg.get("method")
        params = msg.get("params", {})
        result = handle(method, params)
        # Notifications (no id) get no response.
        if mid is not None:
            if isinstance(result, dict) and result.get("isError"):
                send({"jsonrpc": "2.0", "id": mid, "error": {"code": -1, "message": result["content"][0]["text"]}})
            else:
                send({"jsonrpc": "2.0", "id": mid, "result": result})


if __name__ == "__main__":
    main()
