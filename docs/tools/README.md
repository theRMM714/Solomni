# tools（工具与围栏）

> 系统工具、角色表与一次工具执行的围栏。
> 系统侧的门户是 [SYSTOOL.md](../../SYSTOOL.md)（入口与真相源表）；本目录是它的细则。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：

- 系统工具总表、角色表与名字表（`systools/` 三份），以及「按角色发放工具面 / 按表校验调用」的唯一判据。
- 内置文件工具（read / list / write / edit / patch / delete / search）的执行编排与纯规则、工具参数契约、补丁通道。
- 执行类系统工具 `shell` 的参数 / scope 规则与派发；host 的拉起与围栏机制是 kernel 共享的 `ProcessRunner` / `confine`，VM 走 guest。
- 模块工具进程的执行编排；拉起与围栏机制是 kernel 共享的 `ProcessRunner` / `confine`。
- 围栏可达范围的派生（`domain/fence.rs` 的 `for_call` / `for_session` / `standalone`）。

**不管**：

- 不放领域语义（角色是系统的身份，不是业务概念）；不选模型；不碰会话流水。
- 不决定用户授权的主机只读根（`sandbox.policy.host_read_roots`）：由调用方（`conductor` / `collab`）叠加。

## 二、入站契约与状态归属

`api::Tools`（按角色发放工具面 / 总表 / 自检）+ `api::ToolExec`（跑模块工具与系统工具、释放围栏）。出站端口 `SysIo` 与 kernel 共享的 `ProcessRunner` / `FenceHost` 由 `service/mod.rs` 持有（R12）；`SystoolsSource` 只在装配时读一次，不持有。

模块工具的一次执行经 `--fence-run` 守门进程完成（命令与参数分开、环境白名单、超时杀树），协议在 [kernel 的「工具进程与守门协议」](../kernel/README.md)。

`shell` 是执行类系统工具：host 走同一个 `ProcessRunner`（Unix `sh`、Windows PowerShell），VM 走 guest helper；sandbox / external 由 `scope` 请求、profile 封顶。Windows 上每次宿主调用都用按次容器 profile（W1）把 ACL 收窄做实（shell、模块工具、独立调用、构建同一口径）；细节见 [tools-and-roles.md](tools-and-roles.md) 的「执行类系统工具：shell」。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`llm`、`modules`、`prompt`、`workspace`
- 谁在用我：`collab`、`conductor`、`session`、`slate`

## 四、改动本单元时必须同步

- 工具表 / 角色表的同步见 [tools-and-roles.md](tools-and-roles.md) 与 `systools/`。
- 围栏机制的同步细则（平台探针、台账、ACE、解释器基线）与围栏落点（必要 / 可选、授不上的行为、平台差异）都在 [kernel](../kernel/README.md) 的「围栏落点：必要与可选」与「工具进程与守门协议」。
- `FenceSpec.net` 的平台落点以 kernel 的「工具进程与守门协议」为准；tools 只派生描述符，不解释平台能力。
- 授权面按**注入的事实**派生：
  - `<module>/userdata/` 由 modules 在**扫描载入时确保存在**（产品唯一的自动写盘，幂等）；建不了如实标注、不阻断加载。
  - domain 不读盘：`has_userdata` 作为事实随沙箱注入。
  - 事实为否（建不了）时不进 `rw`；`standalone` 的缺省工作目录退回模块根。
  - `prepare_fence` 跳过不存在的落点，不判整次失败。
- 新增 / 修改系统工具 → 改 `systools/tools.yaml` 与（需要时）`systools/roles.yaml`；尚未实现的方向记在 `systool_gaps.yaml`（仓库根；不替代测试缺口账）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
| [`tools-and-roles.md`](tools-and-roles.md) | 系统工具、角色（身份）与「谁能用哪些工具」 |