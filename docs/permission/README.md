# permission（权限）

> 独立于会话的一类能力：回答「谁能在本机、在哪些路径上做什么」。
> 本目录是该单元的唯一细则入口：本页 → [`module-map.md`](module-map.md)（逐文件职责，机器比对）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：决定粒度（`ask` / `full`）、工作区路径的读写白名单与黑名单、模块目录写授权；
把**声明**解析成**生效态**，并提供唯一一处路径判定。

**不管**：不认会话、不碰文件系统、不持端口、不持有状态——落盘的声明在登记处（`settings.yaml` 的全局默认）
与会话 `meta.yaml`（逐 agent 覆盖），生效态每次现算。

## 二、入站契约与状态归属

`api`：`Permissions` / `PermissionsOverride` / `Granularity` 与 `validate` / `validate_override`。
**状态不属于本能力**：全局默认归登记处，逐 agent 覆盖归会话 `meta`；本能力只有纯规则。

## 三、语义（唯一一份）

- 默认：整棵工作区**可读、可提交**；
- **白名单一出现就取代默认**（配了 `allow_read` 就失去默认的整棵可读，配了 `allow_write` 就失去默认提交）；
- **黑名单只做减法，不让默认失效**；两者重叠时**黑大于白**；
- `.` 代表整棵工作区；匹配按**路径组件**做（`web` 不匹配 `website`）；
- **模块目录默认只读**；`module_write` 命中该模块才整块可写；`<module>/userdata/` 恒可写；
- `granularity`：`full` = 任何工具都不设确认；`ask` = `ask` 表里的工具调用**在执行前**先让用户回答。

### 工具级确认的引擎路径（`ask`）

工具循环在执行一次调用前查这一席的生效权限：命中 `ask` 就
① 把调用（工具 / 模块 / 参数）登记进 `kernel::ApprovalRegistry`（与 `JobRegistry` 同级的跨线程机制）、
② 推一张确认卡并阻塞工作线程，
③ 前端回答（动作表 `approve_tool`，**不经生成命令队列**，与「停止」同一条直路）把答案写进放行表并唤醒。
答案有三种：`yes`（放行这一次）、`no`（不执行，回一条"用户拒绝"的工具结果）、
`full`（放行这一次，且**本轮不再问**——只持续到 AI 停下输出，**不改落盘设置**，下一条消息恢复按 `ask` 表询问）。

**回答路径**：Web 的确认卡给「放行 / 拒绝 / 本轮都不再问」三个按钮；页面在等待确认期间**刷新**也能重建卡片
（`/api/state` 的快照带上 `pending_approval`）。CLI 把生成放后台线程、主线程就地提示
`yes / no / full`（`presentation::cli` 的交互循环），所以终端同样能回答。

> **目标契约（实施中）**：核心的各关卡已先走裁决通道的卡片 + 选项（一次回答命令带卡片 id + 选项 id，
> 见 [docs/session/session-model.md](../session/session-model.md) 的「请用户裁决：一条通道，消息 + 选项」）；
> **本节的工具确认尚未并入**。并入后它与核心的各关卡共用一条队列、一张卡（消息 + 选项）、一条回答命令；
> 选项 id 仍是 `allow` / `deny` / `full`（文案归呈现层），但 `kernel::ApprovalRegistry`
> 与动作 `approve_tool` 取消，`/api/state` 的快照只剩一份 `pending`。

白名单 / 黑名单是**呈现层词汇**：解析后下游只见一个正向判定（`read_ok` / `write_ok`）。
平台围栏是正向 allow-list（Landlock 没有 deny 规则），所以嵌套黑名单只在核心层兑现。

## 四、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：无（纯领域，不自持端口）；
- 谁在用我：`registry`（全局默认）、`session`（逐 agent 覆盖）、`workspace`（落点判定）、
  `tools`（围栏派生）、`conductor`（解析与装配）。

## 五、改动本单元时必须同步

- 改语义 → 本页 + [`REGISTRY_SPEC.md`](../../REGISTRY_SPEC.md)（settings/agents 字段）+ [docs/session/session-model.md](../session/session-model.md)（meta）；
- 改落点判定 → [docs/tools/tools-and-roles.md](../tools/tools-and-roles.md) 的路径模型 + 平台围栏探针；
- 改 `FenceSpec` 字段 → 三平台后端与跨平台字面量门禁。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`module-map.md`](module-map.md) | 逐文件职责（T0 与磁盘双向比对） |
