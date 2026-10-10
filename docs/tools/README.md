# tools（工具与围栏）

> 系统工具、角色表与一次工具执行的围栏。
> 系统侧的门户是 [SYSTOOL.md](../../SYSTOOL.md)（入口与真相源表）；本目录是它的细则。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：系统工具总表与角色表（`systools/`）、内置文件工具（read/write/edit/patch/search）的执行编排与纯规则、工具参数契约、补丁通道、外部工具进程的执行编排（**拉起与围栏机制**是 kernel 共享的 `ProcessRunner` / `confine`）、**围栏策略**（派哪些可达范围、`fence_read` 怎么叠加——策略在本能力，机制在 kernel）。

**不管**：不放领域语义（角色是系统的身份，不是业务概念）；不选模型；不碰会话流水。

## 二、入站契约与状态归属

`api::Tools`（按角色发放工具面 / 总表 / 自检）+ `api::ToolExec`（跑模块与内置工具、释放围栏）。出站端口 `SysIo` / `SystoolsSource` 由 `service/mod.rs` 持有（R12）；`ProcessRunner` / `FenceHost` 是 kernel 共享的机制端口。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`llm`、`prompt`、`workspace`
- 谁在用我：`collab`、`conductor`、`session`、`slate`

## 四、改动本单元时必须同步

- 工具表 / 角色表的同步见 [tools-and-roles.md](tools-and-roles.md) 与 `systools/`；围栏机制的同步细则（平台探针、台账、ACE、解释器基线、真机验收）已随机制迁入 [kernel](../kernel/README.md)。
- 授权面按**注入的事实**派生：`<module>/userdata/` 由 workspace 在**载入时确保存在**（产品唯一的自动写盘，幂等；建不了如实标注、不阻断加载）并随沙箱注入（domain 不读盘）；事实为否（建不了）时不进 `rw`，`standalone` 的缺省工作目录退回模块根。`prepare_fence` 也会跳过不存在的落点、不判整次失败。
- 核心代理系统工具的待实现规划清单：`systool_gaps.yaml`（仓库根；不是当前工具表，也不替代测试缺口账）。

- 围栏落点（必要 / 可选）、必要落点授不上的行为、平台差异与真机验收的细则，已随机制迁入 [kernel](../kernel/README.md) 的「围栏落点：必要与可选」。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
| [`tools-and-roles.md`](tools-and-roles.md) | 系统工具、角色（身份）与「谁能用哪些工具」 |