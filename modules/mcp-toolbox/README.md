# mcp-toolbox

一个**用 MCP（Model Context Protocol）声明的常驻服务**模块：给 agent 提供本机小工具，并顺带当仓里 MCP 链路的演示与回归载体。

它不是模块工具（`tools`），而是**常驻服务**（`services`）：核心经 `residents` 能力按声明拉起 `tools/server.py`，
握手后发现操作，再用长驻 stdio 会话转发每一次调用。服务停掉，操作就从模型工具面里消失。

## 声明

```yaml
services:
  toolbox:
    adapter: mcp
    command: python tools/server.py
    desc: 本机小工具箱：时间 / UUID / SHA-256 / 进程内便签
    enabled: false
```

`enabled: false` 是缺省：没经用户同意不起外部常驻进程。要用先启用、再启动。

## 操作

| 操作 | 入参 | 回执 |
| --- | --- | --- |
| `now` | — | `{"iso","epoch_ms"}` 当前 UTC 时间 |
| `uuid` | — | `{"uuid"}` 随机 UUIDv4 |
| `sha256` | `{"text"}` | `{"sha256","bytes"}` 十六进制摘要 |
| `remember` | `{"key","value"}` | `{"ok","key","count"}`；value 只存服务进程内存 |
| `recall` | `{"key"}` | `{"key","found","value"}` |

服务**只用标准库**、不联网、不写盘；便签状态只活在服务进程里，重启即清空。

## 启用与启动

CLI：

```
resident enable mcp-toolbox toolbox
resident start  mcp-toolbox toolbox
```

网页：在「模块与服务」面板里对 `mcp-toolbox.toolbox` 点启用、启动。服务清单与状态也由 `resident list` 查看。

**顺序很重要**：单 agent 会话的工具面在**建工作时**装配一次，所以要先启动服务、再建工作；
反过来建的会话看不到新启动的服务操作（重建该会话才会补上）。

## 演示与回归

- 真机演示（真实模型）：`node demo/run-demo-mcp.mjs`（先起 `node start.js -webUI`）。
- 确定性回归：`tests/cross-platform/e2e/` 的 `mcp` 场景用脚本模型 + 真进程跑同一条链路，进 CI。

## 手工验证

不经过核心也能单独跑这个服务（按行 JSON-RPC）：

```bash
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"sha256","arguments":{"text":"solomni"}}}' | python tools/server.py
```
