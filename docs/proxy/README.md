# proxy（核心代理工具）

> 定位：核心代理（`core_proxy`）的工具面——六个代理工具的载荷与回执、授权判据、幂等账与成员侧执行者；宿主契约 `ProxyHost` 由它声明。
> 效力：规范｜核对：2026-11-16｜依据：`src/capabilities/conversation/proxy/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：**六工具的声明与入参形状**（`src/capabilities/conversation/proxy/domain/proxy.rs`）：`catalog_agents` / `create_session` / `send_session_message` / `observe_session` / `control_session` / `read_session_messages` 的 `*Args`、`CatalogScope` / `ObserveView` / `ControlAction` 的解析、`resolve_new_session` / `relay` / `observe_request` / `control_request` / `messages_request` / `render_catalog`；拒绝文案是逐字契约。
- 管：**授权判据**（`Grant` / `authorize`）：没有授予、已过期、越出工具 / 模型 / 会话范围各自一句固定文案——"不能自行授权"是这段代码的性质，不是调用方的约定。
- 管：**执行面与幂等账**（`src/capabilities/conversation/proxy/service/proxy.rs` 的 `ProxyTools::call`）：非代理工具与总表里没有的工具如实拒绝 → schema 校验与默认值 → 参数形状 → 幂等账（键 = 工具名 + `request_id`）→ 授权 → 交宿主；`create_session` / `send_session_message` / `control_session` 进幂等账，`observe_session` / `read_session_messages` 只读不进。
- 管：**成员侧执行者** `ProxyHandler`（`kernel::ports::ToolHandler`）：代理会话那一回合把这六个工具接到 `ProxyTools` 上，其余工具不认领。
- 管：**宿主契约** `ProxyHost`（`src/capabilities/conversation/proxy/api.rs`）：`catalog` / `create_session` / `send` / `observe` / `control` / `messages` 六个动作；方法注释即约束（只读动作不推进、建会话不留半成品、失败如实回报）。
- 不管：真实会话动作、命令队列与生成——生产宿主是 `runtime` 的 `ProxyBridge`（`src/capabilities/runtime/service/bridge.rs`），装配在 `src/capabilities/runtime/service/handle.rs` 的 `inject_proxy_handler`。
- 不管：会话本体与运行态（`turn`）、转录（`session`）、工作区（`workspace`）；授予读会话的 `meta.delegation`，本单元不另存一份。
- 不管：工具声明、角色面与提示词——`systools/tools.yaml`、`systools/roles.yaml` 与 `prompts/roles/core_proxy.yaml` 是它们的唯一落点。

## 入站契约与状态归属

- 入站面：`api` 转出的值类型（`ProxyCall` / `Grant` / `NewSession` / `Relayed` / `Snapshot` / `MessagesPage` / `ControlState` / `Catalog` …）、同名纯规则函数、执行面 `ProxyTools` 与 `ProxyHandler`，以及**端口反转**的 `ProxyHost`（本单元声明，`runtime` 实现）。
- 状态归属：只有内存里的幂等账（`ProxyTools` 的 `done`，键 = 工具名 / `request_id`），随代理会话那一回合的 `ProxyHandler` 存在与消亡，**不落盘**；其它事实一律经 `ProxyHost` 取（`src/capabilities/conversation/proxy/api.rs` 的六个方法）。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`permission`（`ProfileId`）、`registry`（`validate_name` / `AgentMeta`）、`session`（`OptionView`）；`tools` 的 `ToolBook` / `ToolOutcome` 与 `kernel::ports::ToolHandler`。
- 谁在用我：`runtime`（`src/capabilities/runtime/service/handle.rs` 装配 `ProxyHandler`；`src/capabilities/runtime/service/action.rs` 与 `src/capabilities/runtime/service/{action,bridge}.rs` 用值类型与 `ProxyHost`）、测试。
- **不引用 `runtime`**：依赖只向下，`proxy` 在 `runtime` 之下（`layers.yaml`）。

## 一、一次工具调用的顺序

- 认领：`is_proxy_tool` 只认六个名字；不是代理工具 → `不是代理工具：{名字}`。工具总表里没有 → `工具总表里没有这个工具：{名字}`。
- 参数：声明里的 schema 先校验（`arg_fault_text`），再补默认值；JSON 不合法 → `{名字} 的参数不是合法 JSON：{原因}`；形状不对 → `{名字} 的参数形状不对：{原因}`。
- 幂等：写动作先查账（键 = 工具名 + `request_id`），命中就原样回上次的回执——重试不会建出第二个会话。
- 授权：`authorize` 在**碰宿主之前**判；拒绝时不产生任何外部动作（`FakeProxyHost` 的日志为空是这条的测试判据）。
- 宿主：只有通过了上面四步才调 `ProxyHost`；`send_session_message` 逐目标转达并回 `{sent, failed}`，失败不清成功项。

## 改动本单元时必须同步

- `src/capabilities/conversation/proxy/domain/proxy.rs` 的拒绝文案、回执形状与 `*Args` → `src/tests/runtime/proxy.rs`、`src/tests/proxy.rs` 一起改（前者锁执行面与桥，后者锁三个请求校验器）。
- `src/capabilities/conversation/proxy/api.rs` 的 `ProxyHost` → 生产宿主 `ProxyBridge`（`src/capabilities/runtime/service/bridge.rs`）与替身 `FakeProxyHost`（`src/tests/doubles/proxy.rs`）一起改；端口矩阵在 [docs/testing/doubles.md](../testing/doubles.md)。
- `ProxyHandler` 的形状 → 装配点 `src/capabilities/runtime/service/handle.rs` 的 `inject_proxy_handler` 一起改。
- 工具名或角色面 → `systools/tools.yaml`、`systools/roles.yaml` 与 [docs/tools/tools-and-roles.md](../tools/tools-and-roles.md) 一起改。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
