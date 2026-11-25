# tools（工具与围栏）

> 定位：系统工具总表与一次工具执行的围栏。
> 效力：规范｜核对：2026-11-04｜依据：`src/units/tools/api.rs`
> 系统侧的门户是 [SYSTOOL.md](../../SYSTOOL.md)（入口与真相源表）；本目录是它的细则。本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：系统工具总表与名字表（`systools/` 两份），以及「按表校验调用」的判据；席位面由会话能力的唯一装配机（src/units/agentloop/domain/assemble.rs）按席位现拼。
- 管：内置文件工具（read / list / write / edit / patch / delete / grep）的执行编排与纯规则、工具参数契约、补丁通道。
- 管：执行类系统工具 `shell` 的参数 / 超时规则与派发；host 的拉起与围栏机制是 kernel 共享的 `ProcessRunner` / `confine`，VM 走 guest。
- 管：模块工具进程的执行编排；拉起与围栏机制是 kernel 共享的 `ProcessRunner` / `confine`。
- 管：围栏可达范围的派生（`src/units/tools/domain/fence.rs` 的 `for_call` / `for_session` / `standalone`）。
- 不管：不放领域语义（席位是会话的身份，不是业务概念）；不选模型；不碰会话流水。
- 不管：不决定用户授权的主机只读根（`sandbox.policy.host_read_roots`）——由调用方（`runtime` / `collab`）叠加。

## 入站契约与状态归属

- 入站面：`api::Tools`（总表 / 保留名 / 自检）+ `api::ToolExec`（跑模块工具与系统工具、释放围栏）。
- 出站端口：`SysIo` 与 kernel 共享的 `ProcessRunner` / `FenceHost` 由 `src/units/tools/service.rs` 持有（R12）；`SystoolsSource` 只在装配时读一次，不持有。
- 模块工具的一次执行经 `--fence-run` 守门进程完成（命令与参数分开、环境白名单、超时杀树），协议见 [kernel 的「工具进程与守门协议」](../kernel/README.md)。
- `shell` 是执行类系统工具：host 走同一个 `ProcessRunner`（Unix `sh`、Windows PowerShell），VM 走 guest helper；授予固定（私有沙箱 rw、模块目录与主机只读根 ro），profile 只决定出站网络。Windows 上每次宿主调用都用按次容器 profile（W1）把 ACL 收窄做实（shell、模块工具、独立调用、构建同一口径）；细节见 [tools-and-roles.md](tools-and-roles.md) 的「执行类系统工具：shell」。
- 状态归属：工具面与围栏描述符按调用现算；本单元不持会话状态，工具表只读。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`llm`、`modules`、`prompt`、`workspace`
- 谁在用我：`collab`、`runtime`、`session`、`slate`

## 改动本单元时必须同步

- 工具表与机制席位的同步见 [tools-and-roles.md](tools-and-roles.md) 与 `systools/`。
- 围栏机制的同步细则（平台探针、台账、ACE、解释器基线）与围栏落点（必要 / 可选、授不上的行为、平台差异）都在 [kernel](../kernel/README.md) 的「一、围栏落点：必要与可选」与「二、工具进程与守门协议」。
- `FencePlan.net` 的平台落点以 kernel 的「二、工具进程与守门协议」为准；tools 只派生**计划**，不解释平台能力。
- 授权面按**注入的事实**派生：
  - `<module>/userdata/` 由 modules 在**扫描载入时确保存在**（产品唯一的自动写盘，幂等）；建不了如实标注、不阻断加载。
  - domain 不读盘：`has_userdata` 作为事实随沙箱注入。
  - 事实为否（建不了）时不进 `rw`；`standalone` 的缺省工作目录退回模块根。
  - `prepare_fence` 跳过不存在的落点，不判整次失败。
- 新增 / 修改系统工具 → 改 `systools/tools.yaml` 与（需要时）`src/units/session_model/domain/mechanism.rs` 的席位声明；尚未实现的方向记在 `systool_gaps.yaml`（仓库根；不替代测试缺口账）。
- 新增 / 修改产品命令（命令名、别名、分组、参数声明、确认门槛、执行者）→ 改 `systools/commands.yaml`；写法、校验与补全由参数声明派生（`src/units/tools/domain/op.rs`），执行分派仍写在入口与终端层，表与分派的对齐由 `src/tests/commands.rs` 锁死。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
| [`tools-and-roles.md`](tools-and-roles.md) | 系统工具、机制席位（身份）与「谁能用哪些工具」 |
