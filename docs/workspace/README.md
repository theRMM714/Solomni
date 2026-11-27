# workspace（工作区）

> 定位：沙箱寻址、共享区版本化与成员侧工作工具的处理器。
> 效力：规范｜核对：2026-11-25｜依据：`src/units/business/workspace/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：工作区与沙箱寻址（含越界判定）、按会话铺局（`ensure_layout` / `discard_layout`）、上传、文件视图、根会话的外部工作区绑定投影，以及**共享区版本化工作区**（内容寻址的提交记录、三条线（存档线自动快照 + 各 agent 的工作线 + 主线）、推送时的文件级三方合并、三方视图与显式冲突解决、按提交点还原）。
- 管：**沙箱清单的唯一构造点**（`src/units/business/workspace/domain/seats.rs` 的 `seat_sandboxes`）：调用方只交「每席要什么」（agent 名、模块目录、生效权限面、工作台声明链），本单元算出每席的私有根、模块目录、模块私有区与共享区归属；缺私有根就地报「工作区没有给出 agent <名> 的沙箱路径」。代理会话与普通会话走同一条路。
- 管：**共享区归属的求值**（`src/units/business/workspace/domain/scope.rs` 的 `scope_from`）：全局默认 → 根覆盖 → 逐层收窄 → 全局硬黑名单，顺序错了就会放宽本该收紧的面；会话只在 `SessionMeta.workspace.params` 里存自己那层声明（`edit_of` / `edit_params` 是读写它的唯一入口）。
- 管：**成员侧工作工具处理器**——`work_pull` / `work_commit` / `work_push` / `work_sync` / `work_status` / `work_ignore` / `work_versions` / `work_diff` / `work_resolve` / `work_reset` / `work_lost` 十一个工具名由 `src/units/business/workspace/service/work_tools.rs` 的 `WorkHandler` 认领，回执文本由 `src/units/business/workspace/domain/work_tools.rs` 渲染；`work_handlers` 按工具面发放的名单现取执行者。
- 不管：**建工作**——工作名、通道、guest 与两段事务都在 `work`（`prepare_work` / `commit_work` / `abort_work`），本单元只按已有会话把目录铺出来，不产生会话也不决定工作名；不管模块清单与 `module.yaml`（在 `modules`，本单元只消费它给的模块事实）、不执行工具（`tools`）、不写会话流水（`session`）、不决定谁能用哪些工具（`tools` 的总表与会话能力的流程段与角色）。

## 入站契约与状态归属

- 入站面：`api::Workspace`；装配侧另有 `api::work_handlers(&Arc<dyn Workspace + Send + Sync>, &Sandbox, &[String]) -> Vec<Arc<dyn ToolHandler>>` 与 `api::WorkHandler`（`turn` 装一席工具面时按 `allowed` 现取；工作名、agent 名与生效权限都从沙箱来）；`api::seat_sandboxes` / `api::scope_from`（沙箱清单与共享区归属的唯一派生点，入参只收每席计划与会话声明链）。
  - 目录与视图：**`bind`**（把根会话的工作区绑定刷成 `.work-root` 布局投影；`None` = 产品内布局）/ `ensure_layout`（按会话铺目录：会话目录、版本库 `.work` 与过滤种子、`work/`、各 agent 沙箱）/ **`discard_layout`**（撤销一次铺局：只删本产品建出的东西——空沙箱 / 空 `work/`、布局投影与版本库 `.work`；建会话失败时用）/ `work_has` / `files` / `usage` / `roots`（共享区根、会话树根 `session_root` 与各 agent 私沙根）。`usage` 在内存里按 `session + agents` 缓存 2 秒（前端 `@` 菜单与删除确认会短时间重复取；写路径不做逐写失效），统计时**不跟随**目录链接 / 联接——链接本身按一个文件计，避免重复计数与成环（`src/units/business/workspace/detail/fs_workspace.rs`）。
  - 共享区版本化：`work_pull` / `work_commit` / `work_push` / `work_sync` / `work_commit_user` / `work_status` / `work_ignore` / `work_versions` / `work_diff` / `work_resolve` / `work_reset` / `work_lost` / `work_restore_lost` / `work_recover` / `work_gc` / `work_restore` / `work_head` / `work_archive` / `work_restore_archive` / `work_delete` / `work_archives`。
  - **提交历史视图**：`work_log(work, limit, before?)` 给当前 head、新→旧的分页主线提交与每条 `on_head`（是否在当前 head 祖先链），另给 `work` 列表——各 agent 工作线上的本地提交（新→旧，`pushed` 按主线提交记下的 `sources` 判：squash 之后原工作线提交已不在主线祖先链上，只有来源集合认得出它被推送过）；`work_commit_view` 给提交详情与改动清单（对象大小 / 媒体提示）；`work_commit_file` 给该提交里某个文件的受限预览（路径必须真在这条提交的树里，`src/units/business/workspace/domain/workspace.rs`）。这些是只读视图，不进动作表。
  - **提交点还原**：`work_restore(work, commit)` 只把主副本物化到目标提交并更新 head，**非破坏性**——目标之后的提交仍保留在历史里，可再次回滚。
  - **三条线**：写工具之后的自动提交只进 `refs/agents/<agent>/saves`（存档线，整棵沙箱快照），不动共享区也不动主线；`work_commit` 只把显式路径落进该 agent 的工作线 `refs/agents/<agent>`（父链在自己的线上，本地提交不做别人基线的准入）；`work_push` 才把工作线与主线做三方合并（base = 两条线的**分叉点**提交树，工作线头落后于已拉取路径时用拉取基线补录这些路径；mine = 工作线头、theirs = 主线头），干净就 squash 成**主线一条提交**（`refs/main`）、物化共享区、把工作线快进到它，冲突整次拒绝并点名路径；净改动为零时如实回执空推送、不推进工作线（判定见 `src/units/business/workspace/service/commit.rs` 的 `plan_push`，`work_status` 与它同源）。所以「共享区当前内容」只由 `work_push` 与 `work_sync`（用户在产品之外改了工作区磁盘文件时把它记成 `author=external` 的主线提交）决定，自动快照只服务于回档 / 删除 / 留档 / 恢复（`src/units/business/workspace/service/commit.rs`、`src/units/business/workspace/service/sync.rs`）。
  - **写门禁的版本快照**：`work_gate_info(work, agent, paths)` 只回答三条事实——路径是否受管（在主线 head 树或该 agent 工作线 head 树里）、沙箱当前版本的整份 SHA-256、基线版本（工作线头优先，否则主线头）。它不加锁、不比对读账本；门禁与上下文账本在 `tools` 侧合成（`src/units/business/workspace/service.rs`、`src/units/business/tools/api.rs`）。自动存档线不算受管——否则脚本随手建的 scratch 也会被纳入。
  - **状态相对工作线看**：`work_status` 给主线当前提交、你的工作线头与**待推送的净改动项数**（另报领先的空提交条数与会不会冲突），逐条状态只落在五态——已提交（与主线一致）/ 已提交（未推送）/ 提交后又改了 / 未提交新增 / 未提交删除——另加「主线有、还没拉取」与「与主线一致（没改）」两个提示态；对照物是**工作线**而不是主线，所以「已提交」不代表共享区已变（`src/units/business/workspace/domain/workstore/status.rs` 的 `work_state_of`）。净改动为零但工作线仍领先时只说「内容已与主线一致、领先 N 条空提交」，不再显示未推送条数。工作线与主线不一致时回执另列**最近几条工作线提交**（id / 父提交 / 说明 / 是否已进主线），`work_reset to=` 与「撤回某条提交」都从这里取父提交（`src/units/business/workspace/service/history.rs` 的 `work_status`、`src/units/business/workspace/domain/work_tools.rs` 的 `render_status`）。
   - **看改动与本地回退**：`work_diff`（默认工作线头 → 沙箱；`from` / `to` 可给提交号）只比文本，带 3 行上下文的 hunk；二进制与超过单文件上限的只报路径、大小与类型，不冒充文本（`src/units/business/workspace/domain/workstore/diff.rs`）。`work_reset` 的 `paths` 形态把沙箱路径恢复成基线版本（工作线头优先，没提交过的取拉取基线；覆盖前备份进 `lost/@reset-paths/`，基线里也没有的从沙箱删），`to` 形态把工作线移回自己的祖先提交——`keep_changes=true` 只移引用（soft），否则按目标树重铺工作线跟踪的路径（目标树 ∪ 当前头树；拉过但没提交的文件原样保留），覆盖前备份进 `lost/@reset-<目标>/`。共享区不受影响，要让别人看到回退得重新 `work_commit` + `work_push`；回执按前后工作线头（before/after）说清这次动没动线，`paths` 形态不动线（`src/units/business/workspace/service/reset.rs`）。
  - **删除是硬回退**：`work_delete(work, keep, agents, keep_marks)` 沿当前 head 的祖先链按行锚选点，把主副本落到该点的主线树、每个受影响沙箱落到它自己存档线上 `anchor.line ≤ 点` 的最后一个快照（该 agent 没有存档则清空到主线树）；工作线与存档线各自截断；沙箱里不一致的文件先进 `lost/<agent>/@delete-<点>/`，未在 `keep_marks` 里的档案引用（`refs/archive/*`）一并作废并返回 `pruned`。`work_archives` 供编排层在确认前报出会作废的数量。
  - **三方视图与冲突解决**：`work_versions` 只看（文本预览有上限；图片 / 视频 / 二进制只给指纹、大小与类型提示；带主副本提交 id），`work_resolve` 显式选边——`take=upstream` 用主副本覆盖沙箱，`take=local` 保留本地并把基线推进到主副本；不做自动合并。
  - **冲突解决的安全边界**：
    - `take=local` 必须带 `expect_head`（调用方看到的主副本提交 id，`src/units/business/workspace/api.rs`）；与当前 head 不符就整次拒绝，不把旧合并结果登记到新版本。
    - `take=local` 在「本地与基线一致（没改过）」时整次拒绝；本地缺失 / 没有基线只警告、不硬拒（回执必须列出风险，`src/units/business/workspace/api.rs`）。
    - `take=upstream` 覆盖或删除本地文件前，先把本地版本流式备份到 `.work/lost/<agent>/@resolve/<rel>`（`@<来源>/` 之后才是原路径）；任一备份失败则整次拒绝、不覆盖。
    - `work_lost` 列出本 agent 的备份：每条报**原路径**、备份位置 `@<来源>/<原路径>`、来源与当前沙箱 hash；`path` 既可传原路径也可传备份位置，同一路径有多份备份时必须传备份位置点名一份。恢复必须带 `expect_current`（文件不存在传空串，`src/units/business/workspace/api.rs`），不匹配就拒绝；恢复写回**原路径**，不消费备份（`src/units/business/workspace/service/lifecycle.rs`）。
    - 备份只保留「同一 agent + 同一路径」的最新一份；`lost/` 不进对象库、不被 `work_gc` 清理。
  - **通用资产契约**：共享区里的一切都是**字节资产**，文本 / 图片 / 视频 / 二进制没有存储语义上的区别。
    - 每份资产以全文 SHA-256 为 `hash`、全文字节数为 `size`；文本 / 二进制只是**呈现判定**。
    - `work_versions` 的 `VersionView` 带 `head_commit`（主副本提交 id）与 `media_type`（按扩展名的提示，未知 = None）；`FileVersion` 带 `text` / `truncated` / `too_large`。文本预览最多 `VERSION_TEXT_LIMIT`（16 KiB）；已知媒体扩展名（图片 / 视频 / 音频 / PDF / 压缩包）不读内容、不给文本；其它类型按内容判 NUL / UTF-8。**任一方 `truncated` 或 `too_large` 时回执要明确警告「预览不完整、不能据此合并」，并指引用 `read` 读全文。**
    - 单文件进入共享区的上限是 `MAX_ASSET_BYTES`（64 MiB）。`work_commit` 与 `work_commit_user`（上传）超限**整次拒绝**并点名路径、大小、上限；`work_pull` 对既有超限对象仍流式复制；`work_versions` 对既有超限文件只给 `hash/size/类型`，不预览。共享区**不做存储级分块**，超大文件另见 `tests/gaps.yaml` 的 `work.large-asset-transfer`。
    - 内容 IO 一律分块流式（`WorkStore` 的 `stat_under` / `scan_under` / `stat_object` / `scan_object` / `ingest_under` / `copy_object_to_under`，`src/units/business/workspace/detail/fs_workstore.rs`）：pull 不整份读对象，commit 边读边算指纹边写对象，status 只算指纹，versions 只取元数据与受限预览，resolve / 恢复也走流式复制。
    - 冲突按文件级三方比较：文本可人工合并后用 `take=local`；图片 / 视频 / 二进制不能合并，只能 `take=upstream` 或 `take=local`（`src/units/business/workspace/api.rs`）；不做自动合并。
    - 上传入口在 base64 解码前按上限检查，Web 请求体也有上界；内置 `read` / `write` 仍是文本通道，二进制由模块工具或用户投喂产生。
  - **外部工作区**：根会话可把共享区根绑到产品目录之外（绑定在会话 meta，布局投影 `.work-root`）。CLI 启动的会话默认绑**启动目录**（在哪个目录启动就绑哪个，`--no-workspace` 关闭；`ambient` 与 `resolve` 共用一条阶梯，但启动目录不探可写、判定不过就回退产品内 `work/` 并打印原因）。首次绑定 / 旧会话首次启用时自动把工作区已有文件导入成初始提交（按版本库目录里的 `.workignore` 剪枝，初始名单是 `.git` / `.svn` / `.hg` / `node_modules` / `target` / `__pycache__` / `.venv` / `venv`，可经 `work_ignore` 调整；单文件 64 MiB、总量 256 MiB / 2000 文件为界，超出只导入可导入部分），agent 创建后即可 `work_pull`。工作区磁盘是外部内容的真相：`work_pull` 只拉与 head 一致的路径，`D != head` 的路径进回执的 `external`；`work_push` 写盘前核对同一条路径的磁盘指纹，被外部改过就整次拒绝；`work_status` 也列出 `external`；`work_sync` 读磁盘把差异采纳成 `author=external` 的提交；回档物化只删**当前 head 树里目标树没有**的路径，未跟踪文件永不自动删。多个会话绑同一外部根时共用一个工作区写锁（`data/state/locks/`，不写用户目录）。本机档与虚拟机档都支持外部工作区：虚拟机档把**会话树根**（`data/session/<顶层工作名>`，含各 agent 私沙与 `.work`）与**外部工作区根**各挂一条稳定挂载，guest 内共享区对 agent 只读、写共享区走宿主侧 `work_push`；Windows 的 UNC / 网络路径只在**虚拟机档**可用：绑定到共享下的具体目录（不能是共享根本身），凭据用当前登录用户，路径桥把它编成 guest 内的 `/solomni-host/unc/<server>/<share>/…`；本机档明确拒绝网络工作区（Windows 容器围栏访问不了私有网络与回环，文件围栏还会递归改共享的权限），映射到网络共享的盘符同样按网络路径拒。外部根与产品根的互不包含按**落点身份**（卷 + 文件 id）判，同一物理目录的 UNC / 盘符两种写法不会被当成两处，跨会话工作区锁也用它做键。建会话失败时 `discard_layout` 连版本库 `.work` 一起清（会话从未落盘，用户文件不动）。
  - **过滤名单只有一份**：版本库目录里的 `.workignore`（语法同 `.gitignore`，由 `ignore` 解析）。**内容面**（首次导入、`work_status` 的未跟踪清单、`work_commit` / `work_sync` 的文件夹展开）与**检索面**（内置 `grep` 的默认剪枝）都按这一份名单判「哪些路径算工作区内容」；`.gitignore` 只参与检索面（它表达「不进 git」，不是「不该读」）。显式点名单个文件仍然生效（同 `git add -f`），已跟踪文件不会因新规则消失；新建会话时播种默认名单，已存在不覆盖；list / add / remove 是唯一维护入口（`src/units/business/workspace/domain/path_filter.rs`、`src/units/business/workspace/service/ignore.rs`）。
  - **写事务与恢复**：提交 / 推送 / 同步 / 物化 / 丢弃先写 `.work/` 的 `pending.json`，再改主副本、提交记录、head 与基线；任一步失败由下一次操作幂等重放。元数据与对象用同目录临时文件 + rename；`.work/next-id` 只增不复用；`.work/lock` 是跨进程写闸；`.work/lost/<agent>/@<来源>/<rel>` 是覆盖 / 删除前的本地版本备份。
  - **拉取进度**：`work_pull` 逐条写入沙箱与内存基线，最后原子写回 index；中途失败会先落盘已完成部分的基线，并在错误里点名失败路径与已更新数量。进程在 copy 与 index 写之间被杀仍有窗口，重拉会收敛；严格崩溃一致性走后续 pending 日志。
  - **备份与恢复**：`work_lost(work, agent, path?)` 只列该 agent 的 `lost/`（存储形态 `lost/<agent>/@<来源>/<原路径>`，来源是 `resolve` / `reset-paths` / `reset-<目标>` / `delete-<点>` / `delete-empty` / `archive-<存档号>` / `rewind-<点>` / `restore-<存档号>`；没有 `@` 段的旧备份整条当原路径）；`work_restore_lost(work, agent, path, expect_current)` 按当前沙箱 hash 做 compare-and-swap 后把备份复制回**原路径**。两者都只读写本 agent 的沙箱与版本库，不碰共享主副本。
  - **对象回收**：`work_gc(work, apply)` 以「所有提交树 + 所有 agent 基线」为可达集合；`apply=false` 只报数量和字节，`apply=true` 才删不可达对象并清对象目录里的临时文件。入口是隐藏维护命令 `solomni --work-gc <工作名>`（默认 dry-run，`--yes` 才删）；不做后台静默清理。
  - 提交记录带 `CommitAnchor(agent, session, line)`（会话不可省：同名 agent 在不同会话里行号都从 1 数，缺会话会串），回档据此按行精确定位；**没有锚的提交不参与选点**（外部同步与用户投喂都带锚，无锚 = 来历不明，不能被当成"留档点之前"）。留档引用 `refs/archive/<id>` 记 `target`（留档前的主线头）、`heads`（各 agent 自己的工作线头）、`saves`（各 agent 存档线头）与 `since`（留档时刻的下一个提交号）：恢复到档案头、工作线与存档线各回各的头，然后丢弃 `since` 及之后的提交，留档之后长出来的分叉一并作废。
  - 出站端口 `Workdirs` / **`WorkStore`**（版本库落盘：流式资产原语 + 跨根复制（备份 / 恢复）+ 内容寻址对象 + 提交记录 + 拉取基线 + 待恢复事务 + 提交号 + 跨进程锁 + 对象回收）**只由 `service.rs` 持有**（R12）。
- 模块事实由 `modules` 提供：沙箱把「模块 id → 私有区」收成 `module_data`（只有私有区建得出来的模块才进表，`Module.private` 为 `Some`），围栏派生据此决定授不授写权限、`standalone` 缺省工作目录退到哪——domain 层不读盘。
- 沙箱还带**会话 id**（`Sandbox.session`）：围栏把它当容器身份里的会话租约，配合 agent 与受保护根集合派生 SID（见 [docs/kernel/README.md](../kernel/README.md) 的「身份按会话与 agent、生命周期按会话」）；授权按会话持有，会话结束统一回收。
- 虚拟机档的沙箱还带**路径翻译器**（`Sandbox.path_codec`，本机档为 None = 恒等）：系统工具寻址、模块工具在 guest 里执行、以及用户 `@` 引用的改写共用同一座桥，三处给的是同一种写法（见 [docs/tools/tools-and-roles.md](../tools/tools-and-roles.md) 的「路径模型」）。沙箱清单 `Sandboxes` 另存同一份 `codec`：协作的 `@` 改写发生在**名单确认之前**（那时 `list` 还是空的），由 `Sandboxes::ref_roots()` 取用。
- 状态归属：共享区工作区（提交记录、拉取基线、待恢复事务、锁）由本单元持有；会话与沙箱清单由调用方传入。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`exec_model`、`kernel`、`permission`、`prompt`、`session_model`（只读 `WorkspaceDecl` 这一张声明，不认识会话元信息）
- 谁在用我：`agentloop`、`catalog`、`collab`、`entry`、`lifecycle`、`maintenance`、`presentation`、`provision`、`runtime`、`session_model`、`system`、`tools`、`turn`、`turn_state`、`work`

## 改动本单元时必须同步

- 用例测试在 `src/tests/workspace/` 与 `src/tests/access.rs`（沙箱清单与共享区归属的派生）；运行包与执行档位的契约不在本单元（见 [docs/vm/README.md](../vm/README.md)）。
- 改 `seat_sandboxes` 的入参形状 → `src/units/business/turn/turn/service/facts.rs`（唯一取事实处）与 `src/units/business/turn/turn/service/rebuild.rs`（代理会话），两处都是同一份计划。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |