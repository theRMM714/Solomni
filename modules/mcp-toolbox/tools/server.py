# 目的：mcp-toolbox 的 MCP stdio 服务——按行 JSON-RPC，提供本机小工具与只活在本进程里的便签。
# 管：initialize 握手、notifications/initialized、ping、tools/list、tools/call。
# 不管：进程与围栏（走守门进程的长驻会话）；协议适配（在核心的 residents/detail/mcp.rs）。
# 约束：只用标准库、不联网、不写盘；便签只存内存，服务停掉就没了。
import hashlib
import json
import sys
import time
import uuid as uuid_mod

PROTOCOL_VERSION = "2024-11-05"

# 便签：只活在本进程内存里（常驻服务重启即清空）；最多记 MAX_NOTES 条，超出按写入先后淘汰最旧的。
MAX_NOTES = 256
NOTES = {}

# 按行协议统一 UTF-8：核心起进程时会带 PYTHONIOENCODING，这里兜住手工直跑的场合。
for _stream in (sys.stdin, sys.stdout):
    if hasattr(_stream, "reconfigure"):
        try:
            _stream.reconfigure(encoding="utf-8")
        except ValueError:
            pass


def send(obj):
    sys.stdout.write(json.dumps(obj, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def reply(mid, result):
    send({"jsonrpc": "2.0", "id": mid, "result": result})


def fail(mid, code, message):
    send({"jsonrpc": "2.0", "id": mid, "error": {"code": code, "message": message}})


def text_result(text):
    return {"content": [{"type": "text", "text": text}]}


def tool_error(message):
    return {"content": [{"type": "text", "text": message}], "isError": True}


def as_text(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True)


def arg(args, name, required=True):
    value = args.get(name)
    if value is None:
        if required:
            raise ValueError("缺少必填参数 " + name)
        return ""
    return str(value)


def op_now(args):
    now = time.time()
    return as_text({
        "iso": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(now)),
        "epoch_ms": int(now * 1000),
    })


def op_uuid(args):
    return as_text({"uuid": str(uuid_mod.uuid4())})


def op_sha256(args):
    data = arg(args, "text").encode("utf-8")
    return as_text({"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)})


def op_remember(args):
    key = arg(args, "key")
    value = arg(args, "value", required=False)
    NOTES[key] = value
    # 先写先出：长驻进程的内存有上界，不靠调用方自觉。
    while len(NOTES) > MAX_NOTES:
        NOTES.pop(next(iter(NOTES)))
    return as_text({"ok": True, "key": key, "count": len(NOTES)})


def op_recall(args):
    key = arg(args, "key")
    if key not in NOTES:
        return as_text({"key": key, "found": False})
    return as_text({"key": key, "found": True, "value": NOTES[key]})


def schema(properties, required):
    return {"type": "object", "properties": properties, "required": required}


# 操作表：名字 → 说明 → 参数契约 → 执行体。
TOOLS = [
    ("now", "取当前 UTC 时间（ISO 8601 与 epoch 毫秒）。", schema({}, []), op_now),
    ("uuid", "生成一个随机 UUIDv4。", schema({}, []), op_uuid),
    ("sha256", "计算一段文本的 SHA-256 摘要。",
     schema({"text": {"type": "string", "description": "要摘要的文本"}}, ["text"]), op_sha256),
    ("remember", "把一条 key → value 记在本服务进程的内存里（服务停止或重启即消失）。",
     schema({"key": {"type": "string"}, "value": {"type": "string"}}, ["key"]), op_remember),
    ("recall", "取回本服务进程内存里记住的 value。",
     schema({"key": {"type": "string"}}, ["key"]), op_recall),
]


def tool_list():
    return [
        {"name": name, "description": desc, "inputSchema": sch}
        for name, desc, sch, _ in TOOLS
    ]


def tool_call(name, args):
    for tool_name, _, _, handler in TOOLS:
        if tool_name == name:
            try:
                return text_result(handler(args))
            except ValueError as e:
                return tool_error(str(e))
    return tool_error("没有操作 " + name)


def handle(msg):
    method = msg.get("method")
    mid = msg.get("id")
    if method == "initialize":
        reply(mid, {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "serverInfo": {"name": "mcp-toolbox", "version": "1"},
        })
    elif method == "notifications/initialized":
        pass
    elif method == "ping":
        reply(mid, {})
    elif method == "tools/list":
        reply(mid, {"tools": tool_list()})
    elif method == "tools/call":
        params = msg.get("params") or {}
        args = params.get("arguments")
        if not isinstance(args, dict):
            args = {}
        name = params.get("name")
        if not isinstance(name, str) or name == "":
            reply(mid, tool_error("缺少操作名"))
            return
        reply(mid, tool_call(name, args))
    elif mid is not None:
        fail(mid, -32601, "没有这个方法：" + str(method))


def main():
    # 按行读；只认 JSON 对象。非法行如实报错，不猜、不停机。
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError as e:
            send({"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": "JSON 非法：" + str(e)}})
            continue
        if isinstance(msg, dict):
            handle(msg)
        else:
            # 批量（JSON 数组）这条适配器不用：如实拒绝，不静默丢。
            send({
                "jsonrpc": "2.0",
                "id": None,
                "error": {"code": -32600, "message": "不支持批量请求（一次只发一个 JSON 对象）"},
            })


if __name__ == "__main__":
    main()
