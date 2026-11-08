# workspace（工作区）

> 定位：沙箱寻址与共享区版本化。
> 效力：规范｜核对：2026-11-04｜依据：`src/capabilities/workspace/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：工作区与沙箱寻址（含越界判定）、建工作与上传、文件视图、根会话的外部工作区绑定投影，以及**共享区版本化工作区**（内容寻址的提交记录、各 agent 的拉取基线、文件级三方比较、三方视图与显式冲突解决、按提交点还原）。
- 不管：模块清单与 `module.yaml`（在 `modules`，本能力只消费它给的模块事实）、不执行工具（`tools`）、不写会话流水（`session`）、不决定谁能用哪些工具（`tools` 的角色表）。

## 入站契约与状态归属

- 入站面：`api::Workspace`。
  - 目录与视图：**`bind`**（把根会话的工作区绑定刷成 `.work-root` 布局投影；`None` = 产品内布局）/ `prepare` / **`discard`**（撤销一次准备：只删本产品建出的东西——空沙箱 / 空 `work/`、布局投影与版本库 `.work`；建会话失败时用）/ `work_has` / `files` / `usage` / `roots`（共享区根、会话树根 `session_root` 与各 agent 私沙根）。
  - 共享区版本化：`work_pull` / `work_commit` / `work_sync` / `work_commit_user` / `work_status` / `work_versions` / `work_resolve` / `work_lost` / `work_restore_lost` / `work_recover` / `work_gc` / `work_restore` / `work_head` / `work_archive` / `work_restore_archive` / `work_delete` / `work_archives`。
  - **提交历史视图**：`work_log(work, limit, before?)` 给当前 head、新→旧的分页提交与每条 `on_head`（是否在当前 head 祖先链）；`work_commit_view` 给提交详情与改动清单（对象大小 / 媒体提示）；`work_commit_file` 给该提交里某个文件的受限预览（路径必须真在这条提交的树里，`src/capabilities/workspace/domain/workspace.rs`）。这些是只读视图，不进动作表。
  - **提交点还原**：`work_restore(work, commit)` 只把主副本物化到目标提交并更新 head，**非破坏性**——目标之后的提交仍保留在历史里，可再次回滚。
  - **删除是硬回退**：`work_delete(work, keep, agents, keep_marks)` 沿当前 head 的祖先链按行锚选点，把主副本、每个受影响沙箱与逐路径基线一起落到该点；沙箱里不一致的文件先进 `lost/<agent>/delete-<点>/`，未在 `keep_marks` 里的档案引用（`refs/archive/*`）一并作废并返回 `pruned`。`work_archives` 供编排层在确认前报出会作废的数量。
  - **三方视图与冲突解决**：`work_versions` 只看（文本预览有上限；图片 / 视频 / 二进制只给指纹、大小与类型提示；带主副本提交 id），`work_resolve` 显式选边——`take=upstream` 用主副本覆盖沙箱，`take=local` 保留本地并把基线推进到主副本；不做自动合并。
  - **冲突解决的安全边界**：
    - `take=local` 必须带 `expect_head`（调用方看到的主副本提交 id，`src/capabilities/workspace/api.rs`）；与当前 head 不符就整次拒绝，不把旧合并结果登记到新版本。
    - `take=local` 在「本地与基线一致（没改过）」时整次拒绝；本地缺失 / 没有基线只警告、不硬拒（回执必须列出风险，`src/capabilities/workspace/api.rs`）。
    - `take=upstream` 覆盖或删除本地文件前，先把本地版本流式备份到 `.work/lost/<agent>/<rel>`；任一备份失败则整次拒绝、不覆盖。
    - `work_lost` 列出本 agent 的备份与当前沙箱 hash；恢复必须带 `expect_current`（文件不存在传空串，`src/capabilities/workspace/api.rs`），不匹配就拒绝。恢复不消费备份。
    - 备份只保留「同一 agent + 同一路径」的最新一份；`lost/` 不进对象库、不被 `work_gc` 清理。
  - **通用资产契约**：共享区里的一切都是**字节资产**，文本 / 图片 / 视频 / 二进制没有存储语义上的区别。
    - 每份资产以全文 SHA-256 为 `hash`、全文字节数为 `size`；文本 / 二进制只是**呈现判定**。
    - `work_versions` 的 `VersionView` 带 `head_commit`（主副本提交 id）与 `media_type`（按扩展名的提示，未知 = None）；`FileVersion` 带 `text` / `truncated` / `too_large`。文本预览最多 `VERSION_TEXT_LIMIT`（16 KiB）；已知媒体扩展名（图片 / 视频 / 音频 / PDF / 压缩包）不读内容、不给文本；其它类型按内容判 NUL / UTF-8。**任一方 `truncated` 或 `too_large` 时回执要明确警告「预览不完整、不能据此合并」，并指引用 `read` 读全文。**
    - 单文件进入共享区的上限是 `MAX_ASSET_BYTES`（64 MiB）。`work_commit` 与 `work_commit_user`（上传）超限**整次拒绝**并点名路径、大小、上限；`work_pull` 对既有超限对象仍流式复制；`work_versions` 对既有超限文件只给 `hash/size/类型`，不预览。共享区**不做存储级分块**，超大文件另见 `tests/gaps.yaml` 的 `work.large-asset-transfer`。
    - 内容 IO 一律分块流式（`WorkStore` 的 `stat_under` / `scan_under` / `stat_object` / `scan_object` / `ingest_under` / `copy_object_to_under`，`src/capabilities/workspace/detail/fs_workstore.rs`）：pull 不整份读对象，commit 边读边算指纹边写对象，status 只算指纹，versions 只取元数据与受限预览，resolve / 恢复也走流式复制。
    - 冲突按文件级三方比较：文本可人工合并后用 `take=local`；图片 / 视频 / 二进制不能合并，只能 `take=upstream` 或 `take=local`（`src/capabilities/workspace/api.rs`）；不做自动合并。
    - 上传入口在 base64 解码前按上限检查，Web 请求体也有上界；内置 `read` / `write` 仍是文本通道，二进制由模块工具或用户投喂产生。
  - **外部工作区**：根会话可把共享区根绑到产品目录之外（绑定在会话 meta，布局投影 `.work-root`）。首次绑定 / 旧会话首次启用时自动把工作区已有文件导入成初始提交（忽略 `.git` / `.svn` / `.hg` / `node_modules` / `target` / `__pycache__` / `.venv` / `venv`，单文件 64 MiB、总量 256 MiB / 2000 文件为界，超出只导入可导入部分），agent 创建后即可 `work_pull`。工作区磁盘是外部内容的真相：`work_pull` 只拉与 head 一致的路径，`D != head` 的路径进回执的 `external`；`work_commit` 写盘前核对同一条路径的磁盘指纹，被外部改过就整次拒绝；`work_status` 也列出 `external`；`work_sync` 读磁盘把差异采纳成 `author=external` 的提交；回档物化只删**当前 head 树里目标树没有**的路径，未跟踪文件永不自动删。多个会话绑同一外部根时共用一个工作区写锁（`.home/locks/`，不写用户目录）。本机档与虚拟机档都支持外部工作区：虚拟机档把**会话树根**（`session/<顶层工作名>`，含各 agent 私沙与 `.work`）与**外部工作区根**各挂一条稳定挂载，guest 内共享区对 agent 只读、写共享区走宿主侧 `work_commit`；Windows 的 UNC / 网络路径只在**虚拟机档**可用：绑定到共享下的具体目录（不能是共享根本身），凭据用当前登录用户，路径桥把它编成 guest 内的 `/solomni-host/unc/<server>/<share>/…`；本机档明确拒绝网络工作区（Windows 容器围栏访问不了私有网络与回环，文件围栏还会递归改共享的权限），映射到网络共享的盘符同样按网络路径拒。外部根与产品根的互不包含按**落点身份**（卷 + 文件 id）判，同一物理目录的 UNC / 盘符两种写法不会被当成两处，跨会话工作区锁也用它做键。建会话失败时 `discard` 连版本库 `.work` 一起清（会话从未落盘，用户文件不动）。
  - **写事务与恢复**：提交 / 同步 / 物化 / 丢弃先写 `.work/` 的 `pending.json`，再改主副本、提交记录、head 与基线；任一步失败由下一次操作幂等重放。元数据与对象用同目录临时文件 + rename；`.work/next-id` 只增不复用；`.work/lock` 是跨进程写闸；`.work/lost/<agent>/<rel>` 是 `take=upstream` 覆盖 / 删除前的本地版本备份。
  - **拉取进度**：`work_pull` 逐条写入沙箱与内存基线，最后原子写回 index；中途失败会先落盘已完成部分的基线，并在错误里点名失败路径与已更新数量。进程在 copy 与 index 写之间被杀仍有窗口，重拉会收敛；严格崩溃一致性走后续 pending 日志。
  - **备份与恢复**：`work_lost(work, agent, path?)` 只列该 agent 的 `lost/`；`work_restore_lost(work, agent, path, expect_current)` 按当前沙箱 hash 做 compare-and-swap 后把备份复制回同路径。两者都只读写本 agent 的沙箱与版本库，不碰共享主副本。
  - **对象回收**：`work_gc(work, apply)` 以「所有提交树 + 所有 agent 基线」为可达集合；`apply=false` 只报数量和字节，`apply=true` 才删不可达对象并清对象目录里的临时文件。入口是隐藏维护命令 `solomni --work-gc <工作名>`（默认 dry-run，`--yes` 才删）；不做后台静默清理。
  - 提交记录带 `CommitAnchor(agent, line)`，回档据此按行精确定位。
  - 出站端口 `Workdirs` / **`WorkStore`**（版本库落盘：流式资产原语 + 跨根复制（备份 / 恢复）+ 内容寻址对象 + 提交记录 + 拉取基线 + 待恢复事务 + 提交号 + 跨进程锁 + 对象回收）**只由 `service.rs` 持有**（R12）。
- 模块事实（含 `has_userdata`）由 `modules` 提供：沙箱把它收成 `modules_with_userdata` 一起注入，围栏派生据此决定派不派这条落点——domain 层不读盘。
- 沙箱还带**会话 id**（`Sandbox.session`）：围栏把它当容器身份里的会话租约，配合 agent 与受保护根集合派生 SID（见 [docs/kernel/README.md](../kernel/README.md) 的「身份按 reach、生命周期按会话」）；授权按会话持有，会话结束统一回收。
- 虚拟机档的沙箱还带**路径翻译器**（`Sandbox.path_codec`，本机档为 None = 恒等）：系统工具寻址、模块工具在 guest 里执行、以及用户 `@` 引用的改写共用同一座桥，三处给的是同一种写法（见 [docs/tools/tools-and-roles.md](../tools/tools-and-roles.md) 的「路径模型」）。沙箱清单 `Sandboxes` 另存同一份 `codec`：协作的 `@` 改写发生在**名单确认之前**（那时 `list` 还是空的），由 `Sandboxes::ref_roots()` 取用。
- 状态归属：共享区工作区（提交记录、拉取基线、待恢复事务、锁）由本单元持有；会话与沙箱清单由调用方传入。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`permission`、`prompt`
- 谁在用我：`collab`、`conductor`、`provision`（`guest-runtime` 的沙箱清单）、`session`、`tools`

## 改动本单元时必须同步

- 用例测试在 `src/tests/workspace.rs`；运行包与执行档位的契约不在本单元（见 [docs/vm/README.md](../vm/README.md)）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
