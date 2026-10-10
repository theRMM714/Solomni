# 目的：测试用的最小 MCP stdio 服务——按行 JSON-RPC，只提供 echo 一个工具。
# 约束：自包含、不联网、随手可删；只被 src/tests/mcp.rs 的真实端到端用例拉起。
import json
import sys


def send(obj):
    sys.stdout.write(json.dumps(obj, ensure_ascii=False) + "\n")
    sys.stdout.flush()


for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    method = msg.get("method")
    mid = msg.get("id")
    if method == "initialize":
        send({
            "jsonrpc": "2.0",
            "id": mid,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {"name": "echo", "version": "0"},
            },
        })
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        send({
            "jsonrpc": "2.0",
            "id": mid,
            "result": {
                "tools": [{
                    "name": "echo",
                    "description": "把参数原样回显",
                    "inputSchema": {"type": "object"},
                }],
            },
        })
    elif method == "tools/call":
        args = msg.get("params", {}).get("arguments", {})
        send({
            "jsonrpc": "2.0",
            "id": mid,
            "result": {"content": [{"type": "text", "text": json.dumps(args, ensure_ascii=False)}]},
        })
    elif mid is not None:
        send({"jsonrpc": "2.0", "id": mid, "error": {"code": -32601, "message": "没有这个方法"}})
