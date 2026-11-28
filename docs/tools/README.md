# tools（工具与围栏）

> 定位：系统工具总表与一次工具执行的围栏。
> 效力：规范｜核对：2026-11-28｜依据：`src/units/business/tools/api.rs`
> 系统侧的门户是 [SYSTOOL.md](../../SYSTOOL.md)（入口与真相源表）；本目录是它的细则。本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：系统工具总表与名字表（`systools/tools.yaml`、`systools/names.yaml`）的解码与「按表校验调用」的判据；目录与读盘归 `assets`（`systools/` 的唯一主人）；工具面由 `agentloop` 单元的唯一装配机（src/units/business/agentloop/domain/assemble.rs）按角色现拼。
- 管：内置文件工具（read / list / write / edit / patch / delete / grep）的执行编排与纯规则、工具参数契约、补丁通道。
- 管：执行类系统工具 `shell` 的参数 / 超时规则与派发；host 的拉起与围栏机制是 kernel 共享的 `ProcessRunner` / `confine`，VM 走 guest。
- 管：模块工具进程的执行编排；拉起与围栏机制是 kernel 共享的 `ProcessRunner` / `confine`。
- 管：围栏可达范围的派生（`src/units/business/tools/domain/fence.rs` 的 `for_call` / `for_session` / `standalone`），以及**一席的工具运行环境**（`src/units/business/tools/domain/env.rs` 的 `member_env`：围栏计划、模块变量、PATH 入口与工具链根）与宿主只读根的规整（`host_read_roots`：逐条去空白、丢掉空项，空白与空串不算根）。
- 不管：不放领域语义（角色是会话的身份，不是业务概念）；不选模型；不碰会话流水。
- 不管：不决定用户**授权**哪些主机只读根——名单来自设置里的 `sandbox.policy.host_read_roots`，本单元只把声明规整成路径并写进围栏计划。
- 不管：沙箱怎么来（在 `workspace`，`api::seat_benches` 是唯一构造点）；包事实以「包根 + PATH 入口」两个值进来，本单元不认识包计划类型。

## 入站契约与状态归属

- 入站面：`api::Tools`（总表 / 保留名 / 自检）+ `api::ToolExec`（跑模块工具与系统工具、释放围栏）+ `api::member_env` / `api::host_read_roots`（一席的工具运行环境，纯派生）。
- 出站端口：`SysIo` 与 kernel 共享的 `ProcessRunner` / `FenceHost` / `FenceGate` 由 `src/units/business/tools/service.rs` 持有（R12）；`SystoolsSource` 只在装配时读一次，不持有。
- 模块工具的一次执行经 `--fence-run` 守门进程完成（命令与参数分开、环境白名单、超时杀树），协议见 [kernel 的「工具进程与守门协议」](../kernel/README.md)。
- `shell` 是执行类系统工具：host 走同一个 `ProcessRunner`（Unix `sh`、Windows PowerShell），VM 走 guest helper；授予固定（私有沙箱 rw、模块目录与主机只读根 ro），profile 只决定出站网络。Windows 上每次宿主调用都用按次容器 profile（W1）把 ACL 收窄做实（shell、模块工具、独立调用、构建同一口径）；细节见 [tools-and-roles.md](tools-and-roles.md) 的「执行类系统工具：shell」。
- 状态归属：工具面与围栏描述符按调用现算；本单元不持会话状态，工具表只读。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`catalog`、`kernel`、`llm`、`prompt`、`workspace`
- 谁在用我：`agentloop`、`collab`、`conversation`、`entry`、`lifecycle`、`presentation`、`proxy`、`runtime`、`transcript`、`turn`、`turn_state`、`work`

## 改动本单元时必须同步

- 工具表与流程段与角色的同步见 [tools-and-roles.md](tools-and-roles.md) 与 `systools/`。
- 围栏机制的同步细则（平台探针、台账、ACE、解释器基线）与围栏落点（必要 / 可选、授不上的行为、平台差异）都在 [kernel](../kernel/README.md) 的「一、围栏落点：必要与可选」与「二、工具进程与守门协议」。
- `FencePlan.net` 的平台落点以 kernel 的「二、工具进程与守门协议」为准；tools 只派生**计划**，不解释平台能力。
- 授权面按**注入的事实**派生：
  - 模块私有区（`data/modules/<id>/userdata`）由 `content` 在**模块载入时确保存在**（产品唯一的自动写盘，幂等）；建不了如实标注、不阻断加载。
  - domain 不读盘：私有区落点作为事实随沙箱注入（`Sandbox.module_data`）。
  - 事实为否（建不了）时不进 `rw`；`standalone` 的缺省工作目录退回模块根。
  - `prepare_fence` 跳过不存在的落点，不判整次失败。
- 新增 / 修改系统工具 → 改 `systools/tools.yaml` 与（需要时）`systools/loops.yaml` 的角色位声明；尚未实现的方向记在 `systool_gaps.yaml`（仓库根；不替代测试缺口账）。
- 新增 / 修改产品命令（命令名、别名、分组、参数声明、确认门槛、执行者）→ 改 `systools/commands.yaml`；写法、校验与补全由参数声明派生（`src/units/business/tools/domain/op.rs`），执行分派仍写在入口与终端层，表与分派的对齐由 `src/tests/commands.rs` 锁死。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
| [`tools-and-roles.md`](tools-and-roles.md) | 系统工具、流程段与角色（身份）与「谁能用哪些工具」 |
