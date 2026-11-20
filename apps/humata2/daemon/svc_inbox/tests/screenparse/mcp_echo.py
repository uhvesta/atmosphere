"""A stdio MCP server with one tool, `echo`, so a recorded agent shows its
MCP tool permission prompt. Newline-delimited JSON-RPC 2.0; no network."""
import json
import sys


def reply(request_id, result):
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": request_id, "result": result}) + "\n")
    sys.stdout.flush()


TOOL = {
    "name": "echo",
    "description": "Returns the text it is given.",
    "inputSchema": {
        "type": "object",
        "properties": {"text": {"type": "string"}},
        "required": ["text"],
    },
}

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        message = json.loads(line)
    except json.JSONDecodeError:
        continue
    method = message.get("method")
    request_id = message.get("id")
    if request_id is None:
        continue
    if method == "initialize":
        version = message.get("params", {}).get("protocolVersion", "2025-06-18")
        reply(request_id, {
            "protocolVersion": version,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "screenparse", "version": "1.0.0"},
        })
    elif method == "tools/list":
        reply(request_id, {"tools": [TOOL]})
    elif method == "tools/call":
        text = message.get("params", {}).get("arguments", {}).get("text", "")
        reply(request_id, {"content": [{"type": "text", "text": text}]})
    else:
        sys.stdout.write(json.dumps({
            "jsonrpc": "2.0",
            "id": request_id,
            "error": {"code": -32601, "message": f"no method {method}"},
        }) + "\n")
        sys.stdout.flush()
