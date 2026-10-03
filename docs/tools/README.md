# tools（工具与围栏）

> 系统工具、角色表与一次工具执行的围栏。
> 系统侧的门户是 [SYSTOOL.md](../../SYSTOOL.md)（工具、角色、路径模型、回报与验收）。
> 本目录是该单元的唯一细则入口：本页 → [`module-map.md`](module-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：系统工具总表与角色表（`systools/`）、内置文件工具（read/write/edit/patch/search）的执行编排与纯规则、工具参数契约、补丁通道、外部工具进程的执行、**围栏策略与平台实现**、授权记录与撤销。

**不管**：不放领域语义（角色是系统的身份，不是业务概念）；不选模型；不碰会话流水。

## 二、入站契约与状态归属

`api::Tools`（按角色发放工具面 / 总表 / 自检）+ `api::ToolExec`（跑模块与内置工具、释放围栏）。出站端口 `SysIo` / `ToolRunner` / `FenceHost` / `SystoolsSource` **只由 `service/mod.rs` 持有**（R12）。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`llm`、`prompt`、`workspace`
- 谁在用我：`collab`、`conductor`、`session`、`slate`

## 四、改动本单元时必须同步

- 平台专属代码本地不编译——`FenceSpec` 字面量必须写全字段（跨平台字面量门禁）；改围栏 → `tests/<平台>/` 探针；改工具表 → `systools/` 与本目录 `tools-and-roles.md`。
- 核心代理系统工具的待实现规划清单：`systool_gaps.yaml`（仓库根；不是当前工具表，也不替代测试缺口账）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`module-map.md`](module-map.md) | 逐文件职责（T0 与磁盘双向比对） |
| [`tools-and-roles.md`](tools-and-roles.md) | 系统工具、角色（身份）与「谁能用哪些工具」 |