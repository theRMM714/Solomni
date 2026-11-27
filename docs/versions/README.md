# versions（版本化工作区）

> 定位：共享区版本化——提交记录、三条线、`work_*` 用例与十一个工具的处理器。
> 效力：规范｜核对：2026-11-27｜依据：`src/units/business/versions/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；目录在哪儿问 [`docs/workspace`](../workspace/README.md)；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：**共享区版本化**——内容寻址的提交记录、三条线（存档线自动快照 + 各 agent 的工作线 + 主线）、推送时的文件级三方合并、三方视图与显式冲突解决、按提交点还原、留档 / 删除 / 回收、过滤名单的维护。
- 管：**成员侧十一个工作工具**的处理器与回执：`work_pull` / `work_commit` / `work_push` / `work_sync` / `work_status` / `work_ignore` / `work_versions` / `work_diff` / `work_resolve` / `work_reset` / `work_lost` 由 `src/units/business/versions/service/work_tools.rs` 的 `WorkHandler` 认领，回执文本由 `src/units/business/versions/domain/work_tools.rs` 渲染；`work_handlers` 按工具面发放的名单现取执行者。
- 管：**写门禁的版本快照** `work_gate_info(work, agent, paths)`——只回答三条事实：路径是否受管（在主线 head 树或该 agent 工作线 head 树里）、沙箱当前版本的整份 SHA-256、基线版本（工作线头优先，否则主线头）。它不加锁、不比对读账本；门禁与上下文账本在 `tools` 侧合成（`src/units/business/versions/service.rs`、`src/units/business/tools/api.rs`）。自动存档线不算受管——否则脚本随手建的 scratch 也会被纳入。
- 不管：**目录在哪儿**——布局、沙箱、根、外部绑定投影全在 `workspace`；本单元只经 `api::Workspace` 问「共享区根 / 会话树根 / 各 agent 私沙根」与「这会话是不是外部工作区、锁文件放哪」。
- 不管：建工作（`work`）、模块清单（`modules`）、工具面归属与授权（`tools` / `agentloop` 的流程段与角色）、会话流水（`session`）。

## 入站契约与状态归属

- 入站面：`api::Versions`（版本化用例全集）、`api::WorkHandler` / `api::work_handlers(&Arc<dyn Versions>, &Sandbox, &[String])`（`turn` 装一席工具面时按 `allowed` 现取），以及共享 DTO（`FileVersion` / `ReadFact` / `Change` / `CommitDetail` / `ChangeView` / `GateFile` 等）。
- 出站端口：`api` 之外的 `ports::WorkStore`（版本库落盘的流式原语、内容寻址对象、提交记录、拉取基线、待恢复事务、提交号、跨进程锁、对象回收）**只由 `service.rs` 持有**（R12）；布局侧经 `workspace::api::Workspace`。
- 状态归属：提交记录、拉取基线、待恢复事务、提交号与跨进程锁由本单元持有；「目录在哪」与「外部根是谁」由 `workspace` 持有；会话与沙箱清单由调用方传入。
- **三条线**：写工具之后的自动提交只进 `refs/agents/<agent>/saves`（存档线，整棵沙箱快照），不动共享区也不动主线；`work_commit` 只把显式路径落进该 agent 的工作线 `refs/agents/<agent>`（父链在自己的线上，本地提交不做别人基线的准入）；`work_push` 才把工作线与主线做三方合并（base = 两条线的**分叉点**提交树，工作线头落后于已拉取路径时用拉取基线补录这些路径；mine = 工作线头、theirs = 主线头），干净就 squash 成**主线一条提交**（`refs/main`）、物化共享区、把工作线快进到它，冲突整次拒绝并点名路径；净改动为零时如实回执空推送、不推进工作线（`src/units/business/versions/service/commit.rs` 的 `plan_push`，`work_status` 与它同源）。所以「共享区当前内容」只由 `work_push` 与 `work_sync`（用户在产品之外改了工作区磁盘文件时把它记成 `author=external` 的主线提交）决定，自动快照只服务于回档 / 删除 / 留档 / 恢复。
- **状态相对工作线看**：`work_status` 给主线当前提交、你的工作线头与**待推送的净改动项数**（另报领先的空提交条数与会不会冲突），逐条状态只落在五态——已提交（与主线一致）/ 已提交（未推送）/ 提交后又改了 / 未提交新增 / 未提交删除——另加「主线有、还没拉取」与「与主线一致（没改）」两个提示态；对照物是**工作线**而不是主线，所以「已提交」不代表共享区已变（`src/units/business/versions/domain/workstore/status.rs` 的 `work_state_of`）。
- **通用资产契约**：共享区里的一切都是**字节资产**，文本 / 图片 / 视频 / 二进制没有存储语义上的区别：每份资产以全文 SHA-256 为 `hash`、全文字节数为 `size`，文本 / 二进制只是**呈现判定**；`work_versions` 的 `VersionView` 带 `head_commit` 与 `media_type`，`FileVersion` 带 `text` / `truncated` / `too_large`；文本预览最多 `VERSION_TEXT_LIMIT`（16 KiB），已知媒体扩展名不读内容、不给文本；任一方 `truncated` 或 `too_large` 时回执要明确警告「预览不完整、不能据此合并」并指引用 `read` 读全文；单文件进共享区上限 `MAX_ASSET_BYTES`（64 MiB），超限**整次拒绝**并点名路径、大小、上限，而 `work_pull` 对既有超限对象仍流式复制（`src/units/business/versions/api.rs`、`src/units/business/versions/domain/asset.rs`）。
- **内容 IO 一律分块流式**（`WorkStore` 的 `stat_under` / `scan_under` / `stat_object` / `scan_object` / `ingest_under` / `copy_object_to_under`）：pull 不整份读对象，commit 边读边算指纹边写对象，status 只算指纹，versions 只取元数据与受限预览，resolve / 恢复也走流式复制（`src/units/business/versions/detail/fs_workstore.rs`）。
- **冲突解决的安全边界**：`take=local` 必须带 `expect_head`（调用方看到的主副本提交 id），与当前 head 不符整次拒绝；本地与基线一致（没改过）时整次拒绝；本地缺失 / 没有基线只警告不硬拒。`take=upstream` 覆盖或删除本地文件前先把本地版本流式备份到 `.work/lost/<agent>/@resolve/<rel>`，任一备份失败则整次拒绝、不覆盖。不做自动合并（`src/units/business/versions/service/resolve.rs`、`src/units/business/versions/domain/workstore/resolve.rs`）。
- **写事务与恢复**：提交 / 推送 / 同步 / 物化 / 丢弃先写 `.work/` 的 `pending.json`，再改主副本、提交记录、head 与基线；任一步失败由下一次操作幂等重放。元数据与对象用同目录临时文件 + rename；`.work/next-id` 只增不复用；`.work/lock` 是跨进程写闸；`.work/lost/<agent>/@<来源>/<rel>` 是覆盖 / 删除前的本地版本备份（`src/units/business/versions/service.rs`）。
- **外部工作区**：工作区磁盘是外部内容的真相——`work_pull` 只拉与 head 一致的路径，`D != head` 的路径进回执的 `external`；`work_push` 写盘前核对同一条路径的磁盘指纹，被外部改过就整次拒绝；`work_status` 也列出 `external`；`work_sync` 读磁盘把差异采纳成 `author=external` 的提交；回档物化只删**当前 head 树里目标树没有**的路径，未跟踪文件永不自动删。多个会话绑同一外部根时共用一个工作区写锁（锁文件路径问 `workspace`，锁在这里取）。
- **过滤名单只有一份**：版本库目录里的 `.workignore`（语法同 `.gitignore`）。**内容面**（首次导入、`work_status` 的未跟踪清单、`work_commit` / `work_sync` 的文件夹展开）与**检索面**（内置 `grep` 的默认剪枝）都按这一份名单判「哪些路径算工作区内容」；`.gitignore` 只参与检索面。显式点名单个文件仍然生效（同 `git add -f`），已跟踪文件不会因新规则消失；list / add / remove 是唯一维护入口（`src/units/business/versions/service/ignore.rs`、`src/units/business/workbench/domain/filter.rs`）。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`workspace`（只问「在哪儿」：根、外部根、写闸路径）
- 谁在用我：`agentloop`、`runtime`、`work`、`tools`、`lifecycle`、`system`、`provision`、`maintenance`、`turn`、`turn_state`

## 改动本单元时必须同步

- `docs/workspace/README.md` → 版本化口径搬走后两页不许各说一半；「目录在哪」的判据只写在 workspace。
- `tests/specs/versions.yaml` → 用例声明的文件清单与用例名（版本化搬来时一并搬）。
- `docs/tools/tools-and-roles.md` → `work_*` 十一个工具的回执与授权口径变了要同步。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
