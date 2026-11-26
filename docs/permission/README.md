# permission（权限）

> 定位：独立于会话的一类纯领域规则——只回答「这个会话能不能出站」。
> 效力：规范｜核对：2026-11-25｜依据：`src/units/pure/permission/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：网络档位（`offline` / `online`）的解析、全局默认、根 ceiling 与子会话 narrowing 的合并；生效态 `PermissionSet` 只承载这一个开关。
- 不管：文件可达范围。沙箱、模块目录、主机只读根、工具链都是**固定授予**，不参与判定（见「二、固定授予」）。
- 不管：共享区路径归属。它是 workspace 的 `SharedScope`（见「三、共享区归属」）。
- 不管：不认会话、不碰文件系统、不持端口、不持有状态。落盘的声明在登记处（`settings.yaml` 的 `sandbox.defaults`）与会话 `meta.yaml`（`sandbox.ceiling` / `sandbox.narrowing`），生效态每次现算。

## 入站契约与状态归属

- 入站面：`api`——重导出 `PermissionSet` / `PermissionDefaults` / `PermissionLayer` / `SessionSandbox` / `ProfileId` / `ProfileSpec`。
- 出站端口：无（纯领域，不自持端口）。
- 状态归属：不持有状态；生效态由 `turn` 的 `env::permissions` 沿权限链现算，装配进 `workspace` 的 `Sandbox.permissions`。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：无（纯领域，不依赖别的单元）
- 谁在用我：`catalog`、`proxy`、`runtime`、`session_model`、`turn`、`work`、`workspace`

## 一、只有网络是权限

| id | 标签 | 出站 |
| --- | --- | --- |
| `offline` | 离线（默认） | 否 |
| `online` | 联网 | 是，且仍受全局 `sandbox.policy.network` 约束 |

- 两档只差一个开关：`PermissionSet { profile }`，`allows_network()` 是它唯一的行为。
- 全局 `sandbox.policy.network` 是硬上限：`online` 档在它关闭时仍然不能出站（`FencePlan.net = global_net && permissions.allows_network()`）。
- 档位名不描述「能读哪里」——没有那样的档位，也没有逐工具决定。
- `FencePlan.net` 还有第三态 `Local { ports }`：它**不是档位**，而是一次调用的显式例外——`shell` 的 `local_ports` 让容器里的 `127.0.0.2:<端口>` 经中继到达宿主的 `127.0.0.1:<端口>`，清单外连不上，产品自己监听的端口一律拒（判据是「监听者落在本产品进程树里，或它跑的就是本产品这个可执行文件」（用户另起的实例也算），不靠端口表：`src/kernel/detail/container/windows/relay.rs`）。
- 只有 Windows 后端把 `Local` 落得下去（`Confine::local_relay` → `true`）；其它平台给了 `local_ports` 就在守门进程里拒绝执行，**不**降级成回环全通（那等于把宿主回环上的产品端口一起送出去，`src/kernel/detail/confine/job.rs`）。

## 二、固定授予（不经权限判定）

- **私有沙箱恒可读写**：这是 agent 的工作副本，写它不需要任何档位。
- **模块目录恒只读**；模块自己的私有区（`data/modules/<id>/userdata`）只给模块工具进程写；模块工具写**调用方 agent 的沙箱**（契约见 [MODULE_SPEC.md](../../MODULE_SPEC.md)）。
- **主机只读根**（`sandbox.policy.host_read_roots`）恒只读挂进围栏。
- **项目工具链与运行包**恒只读（可信执行基座，机器级一次性预授，与档位无关，见 [kernel/README.md](../kernel/README.md) 的「授权主体分两个」）。
- 因此「读模块源码」「读写共享区」「执行落点在哪」都不是权限项：前者由工具面（工具面 / 模块声明）决定，后者恒为私有沙箱（见「四」）。

## 三、共享区归属（在 workspace，不在本单元）

- 共享区是**主副本**：任何外部进程都读不到、写不到它；agent 只写沙箱，改动经 `work_pull` / `work_commit` / `work_push`。
- 「这次会话能读 / 能提交共享区的哪些路径」是 workspace 的 `SharedScope`（三态 all / none / 路径数组 + 黑名单），字段在会话 `meta.yaml` 的 `shared`，全局默认在 `settings.yaml` 的 `sandbox.scope`。
- 它约束的是**内置文件工具与工作工具**的路径归属（谁能拉、谁能提交），不是围栏里的 ACL——因此没有任何按会话的 ACL 授予与对账（依据见 [docs/workspace/README.md](../workspace/README.md)）。
- 语义与校验在本单元之外：`src/units/business/workspace/domain/scope.rs`。

## 四、执行落点恒为私有沙箱

- shell 的每次调用都用同一套授予：cwd = 私有沙箱，私有沙箱 rw；模块目录、基础 / 项目工具链、主机只读根 ro；`net` = 全局开关与档位求交（显式给 `local_ports` 时改成只放行那几个本机回环端口，见「一」的第三态）。**调用参数里没有范围旋钮**，也没有第二条「外部落点」的路。
- 要够到沙箱外的东西只有一条路：先 `work_pull` 拉进沙箱；共享区主副本永不进围栏，模块 `userdata` 不可写，模块 secrets 不注入。
- 围栏装不上就拒绝（`allow_degraded=false`），不回退到无围栏执行。
- 没有工具级确认：权限侧不产生裁决卡，也没有「本轮放行」这类开关；需要人定夺的是核心自己的关卡（建会话、验收、定版）与工具层的围栏裁决（授不上必要落点时问一次「无围栏跑 / 放弃」，见 [tools-and-roles.md](../tools/tools-and-roles.md)）。

## 五、围栏派生

- `FencePlan::for_call(sandbox, module, base)`：`home` = 私有沙箱（隐含一条写授予）；`grants` = 主机只读根（`Grant::read`）+ 全部模块目录（只读子树）+ 该模块私有区（`sb.module_data` 里的写子树，私有区建不出来时不授）；`net` = 全局开关与档位求交。不再有按 reach 的分支。
- `FencePlan::for_session(sandbox, host_roots, global_net)`：同一套固定并集，只作会话 base；撤销走会话租约（`release_by_lease`）。
- VM guest 的 per-call 断网由 guest helper 按策略里的 net 做 `unshare(CLONE_NEWNET)`；失败 fail-closed。
- Windows 的容器身份 = `会话租约 + agent`（**不含可达根集合**）：同一 `(agent, shell)` 的调用复用 SID 与 ACL，档位切换不需要重写 DACL。

## 六、根 ceiling + 子会话逐层收窄

- 根写 `sandbox.ceiling`，子写 `sandbox.narrowing`；effective = 全局默认 → 根 ceiling → 沿祖先链每层 narrowing。
- 子只能换更低档：`apply_narrowing` 用 `rank()` 比较，比父宽即报错（`src/units/pure/permission/domain/permission/base.rs`）。
- 共享区的收窄与档位无关，在 workspace 的 `SharedEdit` 里独立校验（`src/units/business/workspace/domain/scope.rs`）：子只能更窄，父编辑只改 ceiling、不覆盖子收窄。
- 档位切换是根级动作：整棵 work tree 有会话在生成时拒绝（先停再切）；切换从下一次求合并起生效。

## 七、安全验收

- 只有两档，且两档只差网络（`profiles_are_two_tiers_that_differ_only_in_network`，`src/tests/permission.rs`）。
- 根可换档、子只能收窄、空层不改生效态（`permission_set_root_and_narrowing_only_touch_the_tier`，`src/tests/permission.rs`）。
- 围栏的固定并集：私有沙箱（`home`）+ 该模块私有区可写、模块目录只读子树、`net` 与全局开关求交（`fence_spec_grants_fixed_surfaces`）。
- 共享区三态与黑名单：序列化、交集、收窄不放大（`shared_scope_narrowing_never_widens`、`shared_scope_serde_and_merge_edges`，`src/tests/permission.rs`）。
- 工作工具的路径过滤跟着读范围走（`work_status_filters_paths_outside_read_scope`、`work_lost_is_scoped_by_read_permission`、`work_versions_and_resolve_are_scoped_by_read_permission`）；`write=none` 时提交 / 推送被拒（`no_write_scope_refuses_commit_and_push`）。
- 工具面只由工具面决定，档位不裁剪（`profile_does_not_change_the_tool_face`，`src/tests/tools/shell.rs`）。
- CI 的 `--fence-live` 门禁把安全探针的环境跳过记 `quality-fail`，不许当成通过（`tests/gates/steps.js` 的 `securitySkipsIn`）；`--strict-env` 没带 `--fence-live` 同样失败。
- 围栏路径的根内判定在规范化失败时拒绝含上级段的路径（`inside_root_rejects_parent_escape_even_when_canonicalize_fails`，`src/kernel/detail/confine/windows/tests.rs`）。

## 改动本单元时必须同步

- 改语义 → 本页 + `REGISTRY_SPEC.md`（settings 的 sandbox 段）+ `docs/session/session-model.md`（meta 的会话级 sandbox）。
- 改共享区归属 → `docs/workspace/README.md`、`docs/tools/tools-and-roles.md` 的路径模型。
- 改 `FencePlan` / `Grant` / `NetPolicy` 字段 → 三平台后端（`trait Confine` 的四个实现）与跨平台字面量门禁（`src/kernel/contracts/fence.rs`）。
- 改档位 → `systools/tools.yaml`、`docs/tools/tools-and-roles.md`、Web 配置页与冒烟。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |