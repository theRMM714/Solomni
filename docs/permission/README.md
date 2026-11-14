# permission（权限）

> 定位：独立于会话的一类能力——回答「谁能在本机、在哪些路径上做什么、要不要问」。
> 效力：规范｜核对：2026-11-13｜依据：`src/capabilities/permission/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：权限声明（全局默认、会话 ceiling、子会话 narrowing）的解析与求交；单次调用的 reach × effect × decision 判定；共享区路径范围；四档 profile 预设。
- 不管：不认会话、不碰文件系统、不持端口、不持有状态。
- 不管：落盘的声明在登记处（`settings.yaml` 的 `sandbox.defaults` 与 `sandbox.policy`）与会话 `meta.yaml`；生效态每次现算。

## 入站契约与状态归属

- 入站面：`api`——重导出权限类型、四档预设与校验入口（`hit` / `normalize` / `validate_*` 等）。
- 出站端口：无（纯领域，不自持端口）。
- 状态归属：不持有状态；落盘的声明在登记处（`settings.yaml` 的 `sandbox.defaults` 与 `sandbox.policy`）与会话 `meta.yaml`，生效态每次现算。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：无（纯领域，不自持端口）
- 谁在用我：`registry`（全局默认与硬黑名单）、`session`（meta 的 ceiling / narrowing）、`workspace`（落点判定）、`conductor`（解析与装配）、`tools`（工具声明与围栏派生）

## 一、三个维度

- **Reach（能到哪）**
  - private：自己的私有沙箱；
  - shared-read：读共享区主副本；
  - shared-write：写共享区（只有 work_commit 与写工具后的自动提交这条路径使用，外部工具进程永远不直接写主副本）；
  - module-read：读成员模块目录；
  - module-write：写模块自己的 userdata（只给模块工具进程）；
  - host-read：用户显式授权的主机只读根；
  - network：出站网络。
- **Effect（这次操作属于哪类）**
  - read：read / list / search / work_pull / work_status / work_versions；
  - write：write / edit / patch / delete / work_commit / work_resolve / work_lost 的恢复；
  - exec：模块工具 / shell；
  - none：不碰文件、不需要权限闸门的动作。
- **Decision（怎么处置）**
  - allow：直接执行；
  - ask：进统一裁决队；没有可回答前端 = 拒绝；
  - deny：不执行，如实回执。
- secret 不是 reach：模块只声明自己有哪些隐秘字段，值只由系统持有并在起模块工具 / 服务进程时注入；agent 与 shell 都不直接触碰它。

## 二、四档 profile

| id | 标签 | reach | 读 | 写 | exec sandbox | exec external |
| --- | --- | --- | --- | --- | --- | --- |
| view | 仅可查看 | private + shared-read + module-read | allow | deny | deny | deny |
| workspace | 工作区内修改 | view + shared-write + module-write | allow | allow | allow | deny |
| external | 外部可达（逐次确认） | workspace + host-read + network | allow | allow | allow | ask |
| all_reach | 全可达（自动放行） | 同 external | allow | allow | allow | allow |

- shared-write 只给 work_commit（含写工具落地后的自动提交）；它不会把共享区放进任何外部进程的 rw。
- module-write 只给模块工具写自己的 userdata；shell 不声明它，所以全可达也写不进模块 userdata。
- 私有沙箱不受共享区路径范围约束，但 profile 的 write decision 仍然作用于它（仅可查看档禁写）。
- 最高档按 reach 命名，不叫「完全权限」；它仍受全局 network / host_read_roots / deny / 围栏强制等级上限。

## 三、单次调用判定

- 输入：会话生效权限（profile 的 reach / decision）、工具声明（effect / 最大 reach / 默认 decision / 是否接 scope）、本次参数。

1. granted = 会话 reach ∩ 工具最大 reach；
2. 工具声明 scope 时：requested = sandbox（只有 private）或 external（private + shared-read + module-read + host-read + network）；若 requested 不是 granted 的子集 → 拒绝（不是问）；granted = granted ∩ requested；
3. decision = 会话逐工具覆盖 → profile 的 effect/scope 决定（四档都显式给出；工具声明里的默认只作缺省说明，不压过用户选的 profile）；
4. deny 不执行；ask 有前端进裁决队、无前端拒绝；allow 执行；
5. 执行层按 granted 派生围栏 / 做路径判定。

- 卡片上的 allow_turn 只表示「本轮剩余 ask 都不再问」；它不改 reach、不改 profile、不落盘。后续调用如果 reach 不够，仍然拒绝。
- **shell 的 scope** 是本次调用的 reach 请求，不是第二个权限档：
  - sandbox（缺省）：只 private + 基础 / 项目工具链只读；net=false；workspace 及以上 allow，view 不发放；不碰共享 / 模块 / 主机只读根。
  - external：在 sandbox 之上加 shared-read / module-read / host-read / network；external 档 ask、all_reach 档 allow；共享区 / 模块目录仍只读，module-write 不声明，模块 secrets 不注入。
  - sandbox 与 external 都要求围栏真强制（`allow_degraded=false`），装不上就拒绝；命令被围栏拒绝时回执提示 external，**不自动升级**。
  - Windows 上容器身份 = `会话租约 + agent + 受保护根集合`（reach 指纹，不含工具链、不含 net）：同一会话里不同 scope 的调用根集合不同 → 不同 SID（按次收窄因此真实成立）；同一 reach 的后续调用复用 SID 与 ACL。授权**按会话持有**，会话改配 / 删除时撤销；一次性调用（构建 / 独立模块工具）各自唯一租约、跑完即撤。
  - **外部工作区授权粒度三态**（绑定在根会话 meta 的 `workspace.grant`，子会话沿 `parent` 链继承最近一份）：`session`（缺省）逐会话授/撤共享区；`workspace` 该工作区一次性机器级只读（跨会话零写，代价是本产品任何会话的容器都能读，不限本会话）；`none` 不向容器授共享区，且生效权限里把 `shared-read` / `shared-write` 整个拿掉（`PermissionSet::without_shared`），内置文件工具与 shell 拿到的是同一份事实。粒度只影响围栏与共享区读范围，不改变文件归属。
  - **工具链与运行包不是 reach**：`.tools/` 与 `runtimes/<包>` 是可信执行基座（只读、无秘密、本来就要执行），一次性授给**本产品的能力 SID**（`S-1-15-3-*`，不是 `ALL APPLICATION PACKAGES`——那会把读权限下放给本机所有 AppContainer 应用，见 [kernel/README.md](../kernel/README.md) 的「授权主体分两个」）、按机器级归属记台账（`scope=machine`、无归属，跨会话复用、启动对账不回收），不参与按 reach 的容器身份与按次授权；共享区 / 模块目录 / 用户只读根才按 reach 收窄。产品根上的一条 `(OI)(CI)` 只读属性同属机器级授权（授给同一个能力 SID、跨会话复用、启动对账不回收），也不属于 reach：容器 cwd 走视图盘写法（产品根映射成实例独占的盘符），产品根之外的祖先不授。**机器级一次性预授**（`--setup=fence`，或首次授权成功后自动一次）把产品根 `(OI)(CI)` 只读属性、`.tools/` 与 `runtimes/` 递归只读一次写掉；之后每个新会话只剩按 reach 的容器 SID 授权，不再重复写这批 ACE。

## 四、围栏派生

- `FenceSpec::for_call` 按本次 granted 派生：private 进 rw；module-write 只把该模块的 userdata 放进 rw；shared-read / module-read 进 ro_tree；host-read 进 ro；network 与全局开关求交。shared-write 永不进 rw。shell 的 per-call 围栏另加项目内**基础工具链根**（只读）；模块工具的放行仍走它自己的命令解释器基线。
- `FenceSpec::for_session` 只作**会话 base**（提供主机只读根与全局断网事实）再派生按次围栏；撤销走会话租约（`release_by_lease`），不再按「会话并集」反推落点。
- VM guest 的 per-call 断网由 guest helper 按策略里的 net 做 `unshare(CLONE_NEWNET)`；失败 fail-closed，不静默降级。

## 五、根 ceiling + 子会话逐层收窄

- 根会话持有 ceiling：profile、reach 子集、共享区范围、逐工具决定。
- 每个子会话只有一层 narrowing；effective = 全局默认 ∩ 根 ceiling ∩ 沿祖先链的每一层 narrowing。
- 子只能换更低档，reach / 共享区范围 / decision 只能更窄更严；要放宽必须改根 ceiling（`src/capabilities/permission/domain/permission.rs`）。
- 路径范围只作用于共享区：work_pull / work_commit / 读共享；私有沙箱永远不受它的路径范围限制。
- 共享区范围用三态：all / none / 路径数组。字段 None = 不覆盖，Some(none) = 显式无权限，两者必须分开（`src/capabilities/permission/domain/permission.rs`）。
- 父级编辑只更新 ceiling，再与子 narrowing 求交，不覆盖子收窄；父级收窄到无交集时子会话无权限，并如实提示。
- 档位切换是根级动作：整棵 work tree 有会话在生成时拒绝（先停再切）；切换在下一次权限判定生效，已经在执行的调用不追溯。
- Windows 的 ACL 是持久的：容器身份按 `会话 + agent + 可达根集合` 派生；同一 reach 的调用复用授权，会话结束统一回收（`release_by_lease`）。档位收窄后不会再读到别的 scope 留下的外部根（这正是并发重叠场景的判据）。

## 六、工具级确认的引擎路径

- 工具循环在执行一次调用前先过权限闸门。ask 命中时：
  - 有确认通道才问；没有可回答前端时直接拒绝，不阻塞、不放行；
  - 有通道时，这次调用作为一关进裁决队，与核心各关卡共用同一条队；
  - 卡片选项 id 是契约：allow / deny / allow_turn；
  - 停止 = 整队作废，等待方按拒绝解开；
  - 卡与回答随会话落盘；重启后不重问已答的。

## 七、安全验收

- 行为矩阵：四档 profile × 七个 reach × 四种 effect × 三种 call scope 的处置逐格穷举（`decision_matrix_is_total_across_profile_effect_scope_and_reach`，`src/tests/permission.rs`）。
- 代数性质：`Decision` 取严的交换 / 结合 / 幂等 / 吸收，`ReachSet` 求交的交换 / 结合 / 幂等与子集等价，共享区三态的子集蕴含与交集逐路径等价（`decision_and_reach_algebra_hold_for_every_combination`、`shared_scope_intersection_and_subset_agree_on_every_path`）。
- 收窄不放大：子会话的档位、reach、共享区、decision、拒绝名单与逐工具决定逐维断言（`narrowing_never_widens_profile_reach_scope_or_decision`）。
- CI 的 `--fence-live` 门禁把安全探针的环境跳过记 `quality-fail`，不许当成通过（`tests/gates/steps.js` 的 `securitySkipsIn`）；`--strict-env` 没带 `--fence-live` 同样失败。
- 围栏路径的根内判定在规范化失败时拒绝含上级段的路径（`inside_root_rejects_parent_escape_even_when_canonicalize_fails`，`src/kernel/detail/confine/windows/tests.rs`）。

## 改动本单元时必须同步

- 改语义 → 本页 + `REGISTRY_SPEC.md`（settings 的 sandbox 段）+ `docs/session/session-model.md`（meta 的会话级 sandbox）。
- 改落点判定 → `docs/tools/tools-and-roles.md` 的路径模型 + 平台围栏探针。
- 改 `FenceSpec` 字段 → 三平台后端与跨平台字面量门禁（`src/kernel/contracts/fence.rs`）。
- 改 profile / 工具声明 → `systools/tools.yaml`、`docs/tools/tools-and-roles.md`、Web 配置页与冒烟。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
