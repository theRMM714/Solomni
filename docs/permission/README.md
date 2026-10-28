# permission（权限）

> 独立于会话的一类能力：回答「谁能在本机、在哪些路径上做什么、要不要问」。
> 本目录是该单元的唯一细则入口：本页 → unit-map.md（逐文件职责，机器比对）。
> 分层与依赖方向见 ARCHITECTURE.md 的 §一，业务边界判据见 §九；测试规范见 TESTING.md。

## 一、管什么 / 不管什么

**管**：权限声明（全局默认、会话 ceiling、子会话 narrowing）的解析与求交；单次调用的 reach × effect × decision 判定；共享区路径范围；四档 profile 预设。

**不管**：不认会话、不碰文件系统、不持端口、不持有状态。落盘的声明在登记处（settings.yaml 的 sandbox.defaults 与 sandbox.policy）与会话 meta.yaml；生效态每次现算。

## 二、三个维度

**Reach（能到哪）**

- private：自己的私有沙箱；
- shared-read：读共享区主副本；
- shared-write：写共享区（只有 work_commit 这条路径使用，外部工具进程永远不直接写主副本）；
- module-read：读成员模块目录；
- module-write：写模块自己的 userdata（只给模块工具进程）；
- host-read：用户显式授权的主机只读根；
- network：出站网络。

**Effect（这次操作属于哪类）**

- read：read / list / search / work_pull / work_status / work_versions；
- write：write / edit / patch / delete / work_commit / work_resolve / work_lost 的恢复；
- exec：模块工具 / 未来的 shell；
- none：不碰文件、不需要权限闸门的动作。

**Decision（怎么处置）**

- allow：直接执行；
- ask：进统一裁决队；没有可回答前端 = 拒绝；
- deny：不执行，如实回执。

secret 不是 reach：模块只声明自己有哪些隐秘字段，值只由系统持有并在起模块工具 / 服务进程时注入；agent 与 shell 都不直接触碰它。

## 三、四档 profile

| id | 标签 | reach | 读 | 写 | exec sandbox | exec external |
| --- | --- | --- | --- | --- | --- | --- |
| view | 仅可查看 | private + shared-read + module-read | allow | deny | deny | deny |
| workspace | 工作区内修改 | view + shared-write + module-write | allow | allow | allow | deny |
| external | 外部可达（逐次确认） | workspace + host-read + network | allow | allow | allow | ask |
| all_reach | 全可达（自动放行） | 同 external | allow | allow | allow | allow |

- shared-write 只给 work_commit；它不会把共享区放进任何外部进程的 rw。
- module-write 只给模块工具写自己的 userdata；shell 不声明它，所以全可达也写不进模块 userdata。
- 私有沙箱不受共享区路径范围约束，但 profile 的 write decision 仍然作用于它（仅可查看档禁写）。
- 最高档按 reach 命名，不叫「完全权限」；它仍受全局 network / host_read_roots / deny / 围栏强制等级上限。

## 四、单次调用判定

输入：会话生效权限（profile 的 reach / decision）、工具声明（effect / 最大 reach / 默认 decision / 是否接 scope）、本次参数。

1. granted = 会话 reach ∩ 工具最大 reach；
2. 工具声明 scope 时：
   requested = sandbox（只有 private）或 external（shared-read + module-read + host-read + network）；
   若 requested 不是 granted 的子集 → 拒绝（不是问）；
   granted = granted ∩ requested；
3. decision = 会话逐工具覆盖 → profile 的 effect/scope 决定（四档都显式给出；工具声明里的默认只作缺省说明，不压过用户选的 profile）；
4. deny 不执行；ask 有前端进裁决队、无前端拒绝；allow 执行；
5. 执行层按 granted 派生围栏 / 做路径判定。

卡片上的 allow_turn 只表示「本轮剩余 ask 都不再问」；它不改 reach、不改 profile、不落盘。后续调用如果 reach 不够，仍然拒绝。

## 五、围栏派生

FenceSpec::for_call 按本次 granted 派生：private 进 rw；module-write 只把该模块的 userdata 放进 rw；shared-read / module-read 进 ro_tree；host-read 进 ro；network 与全局开关求交。shared-write 永不进 rw。

FenceSpec::for_session 用会话 profile 的 reach 并集，用于撤销 Windows ACL 等持久授权与生命周期；它不用于单次执行。

VM guest 的 per-call 断网由 guest helper 按策略里的 net 做 unshare(CLONE_NEWNET)；失败 fail-closed，不静默降级。

## 六、根 ceiling + 子会话逐层收窄

- 根会话持有 ceiling：profile、reach 子集、共享区范围、逐工具决定；
- 每个子会话只有一层 narrowing；effective = 全局默认 ∩ 根 ceiling ∩ 沿祖先链的每一层 narrowing；
- 子只能换更低档，reach / 共享区范围 / decision 只能更窄更严；要放宽必须改根 ceiling；
- 路径范围只作用于共享区：work_pull / work_commit / 读共享；私有沙箱永远不受它的路径范围限制；
- 共享区范围用三态：all / none / 路径数组。字段 None = 不覆盖，Some(none) = 显式无权限，两者必须分开；
- 父级编辑只更新 ceiling，再与子 narrowing 求交，不覆盖子收窄；父级收窄到无交集时子会话无权限，并如实提示；
- 档位切换是根级动作：整棵 work tree 有会话在生成时拒绝（先停再切）；切换在下一次权限判定生效，已经在执行的调用不追溯。

## 七、工具级确认的引擎路径

工具循环在执行一次调用前先过权限闸门。ask 命中时：

- 有确认通道才问；没有可回答前端时直接拒绝，不阻塞、不放行；
- 有通道时，这次调用作为一关进裁决队，与核心各关卡共用同一条队；
- 卡片选项 id 是契约：allow / deny / allow_turn；
- 停止 = 整队作废，等待方按拒绝解开；
- 卡与回答随会话落盘；重启后不重问已答的。

## 八、依赖图位置

- 经 ::api 用到：无（纯领域，不自持端口）；
- 谁在用我：registry（全局默认与硬黑名单）、session（meta 的 ceiling / narrowing）、workspace（落点判定）、conductor（解析与装配）、tools（工具声明与围栏派生）。

## 九、改动本单元时必须同步

- 改语义 → 本页 + REGISTRY_SPEC.md（settings 的 sandbox 段）+ docs/session/session-model.md（meta 的会话级 sandbox）；
- 改落点判定 → docs/tools/tools-and-roles.md 的路径模型 + 平台围栏探针；
- 改 FenceSpec 字段 → 三平台后端与跨平台字面量门禁；
- 改 profile / 工具声明 → systools/tools.yaml、docs/tools/tools-and-roles.md、Web 配置页与冒烟。

## 本目录

| 文件 | 内容 |
| --- | --- |
| unit-map.md | 逐文件职责（T0 与磁盘双向比对） |
