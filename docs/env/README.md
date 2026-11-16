# env（环境事实）

> 定位：环境事实的唯一派生者——权限面、沙箱清单、一席的围栏 / 进程环境 / 工具链根与 guest 租约，全部由入参算出。
> 效力：规范｜核对：2026-11-16｜依据：`src/capabilities/env/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：权限面——`api::permissions` 先套根会话 ceiling、再逐层 narrowing，最后并上全局硬黑名单；共享区授权为 `None` 时收掉共享读写的 reach。
- 管：沙箱清单——`api::sandboxes` 按 agent 名单组装，每席只挂自己的模块目录，缺沙箱路径就地报错。
- 管：一席的环境——`api::member` 派生围栏、模块变量、运行时可执行文件与工具链根。
- 管：全局策略——`api::network_allowed` 读设置里的网络开关，`api::host_read_roots` 把用户填的宿主只读根逐条去空白、丢掉空项后转成路径；两处都只有这一个出处。
- 不管：事实取得——`SessionMeta` / `WorkRoots` / `Roster` / `PackagePlan` / `AppSettings` 都由调用方先取好，本单元无端口、无 IO。
- 不管：落盘与进程——不写文件、不启停 guest、不碰围栏授权台账（`tools` 的 `release_by_lease`）。
- 不管：机制侧怎么用——围栏与模块变量由调用方装进工具面（`src/capabilities/conductor/service/env.rs` 的 `build_single`）。

## 入站契约与状态归属

- 入站面：`api::member` / `api::sandboxes` / `api::permissions` / `api::workspace_grant` / `api::network_allowed` / `api::host_read_roots`，以及事实结构 `api::MemberFacts` / `api::MemberEnv` / `api::SandboxesFacts`；别的业务与测试只经 `api` 取用（R1）。
- 状态归属：**不持任何状态**——历史、工作区根、模块名册、包计划与 registry 设置都是入参；派生结果随调用返回，本单元没有字段、没有缓存、没有后台任务。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`session`（`SessionMeta` / `AgentMeta` / `WorkspaceGrant`）、`workspace`（`Sandbox` / `Sandboxes` / `WorkRoots`）、`modules`（`Roster`）、`packages`（`PackagePlan`）、`permission`（`PermissionSet`）、`registry`（`AppSettings`）、`kernel`（`FenceSpec` / `Tier`、`ports::PathCodec`）
- 谁在用我：`conductor`（`src/capabilities/conductor/service/env.rs` 取事实后转调六个派生函数）、`collab`（`src/capabilities/collab/service/collab.rs` 的会话设置取网络开关与宿主只读根）、`cli` / `web`（入口装配取网络开关与宿主只读根条数）、`src/tests/env.rs`

## 一、事实进、结果出

- 围栏只有一处算法：`FenceSpec::for_session` 先按沙箱与本席的宿主读根建底，再把包根并进 `ro_tree` 后排序去重；packages 侧不再各自拼围栏。
- 工具链根按档位分流：只有 `Tier::Host` 才带 `base_roots`，`Tier::Vm` 只留包根，同样排序去重——`src/capabilities/env/domain/env.rs` 的 `member` 是唯一判定点。
- 权限链的收口在顺序：根 ceiling → 逐层 narrowing → 硬黑名单 → 视需要 `without_shared`；换顺序就会放宽本该收紧的面。
- 全局策略也只有一处算法：网络开关直接取 `sandbox.policy.network`；宿主只读根逐条 `trim`、丢掉空串、其余转 `PathBuf`——空白项与空串不算根，判定点只有 `src/capabilities/env/domain/env.rs` 的 `network_allowed` / `host_read_roots`。

## 改动本单元时必须同步

- `src/capabilities/conductor/service/env.rs` → 取事实与转调都在这里（`guest_scope` / `sandboxes` / `permissions_for` / `workspace_grant` / `tools_env` 取一席环境）：事实从哪来变了必须同改，`env` 的入站签名保持不变。
- `src/capabilities/env/api.rs` → 新增入站项要同步 [`unit-map.md`](unit-map.md) 与本页「入站契约」。
- `src/tests/env.rs` → 五条用例钉住权限链、共享区、租约、一席环境与模块挂载：改派生规则必须同改断言。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
