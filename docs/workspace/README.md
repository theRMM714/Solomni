# workspace（工作区）

> 沙箱寻址与共享区版本化。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：工作区与沙箱寻址（含越界判定）、建工作与上传、文件视图，以及**共享区版本化工作区**（内容寻址的提交记录、各 agent 的拉取基线、文件级三方比较、三方视图与显式冲突解决、按提交点还原）。

**不管**：模块清单与 `module.yaml`（在 `modules`，本能力只消费它给的模块事实）、不执行工具（`tools`）、不写会话流水（`session`）、不决定谁能用哪些工具（`tools` 的角色表）。

## 二、入站契约与状态归属

`api::Workspace`：

- 目录与视图：`prepare` / **`discard`**（撤销一次准备：只删空目录，建会话失败时用）/ `work_has` / `files` / `usage` / `roots`。
- 共享区版本化：`work_pull` / `work_commit` / `work_commit_user` / `work_status` / `work_versions` / `work_resolve` / `work_recover` / `work_gc` / `work_restore` / `work_head` / `work_rewind_to` / `work_restore_point` / `work_discard_after`。
- **三方视图与冲突解决**：`work_versions` 只看（文本预览有上限；图片/视频/二进制只给指纹、大小与类型提示），`work_resolve` 显式选边——`take=upstream` 用主副本覆盖沙箱，`take=local` 保留本地并把基线推进到主副本；不做自动合并。
- **通用资产契约**：共享区里的一切都是**字节资产**，文本 / 图片 / 视频 / 二进制没有存储语义上的区别。
  - 每份资产以全文 SHA-256 为 `hash`、全文字节数为 `size`；文本 / 二进制只是**呈现判定**。
  - `work_versions` 的 `VersionView` 带 `media_type`（按扩展名的提示，未知 = None）；`FileVersion` 带 `text` / `truncated` / `too_large`。文本预览最多 `VERSION_TEXT_LIMIT`（16 KiB）；已知媒体扩展名（图片 / 视频 / 音频 / PDF / 压缩包）不读内容、不给文本；其它类型按内容判 NUL / UTF-8。
  - 单文件进入共享区的上限是 `MAX_ASSET_BYTES`（64 MiB）。`work_commit` 与 `work_commit_user`（上传）超限**整次拒绝**并点名路径、大小、上限；`work_pull` 对既有超限对象仍流式复制；`work_versions` 对既有超限文件只给 `hash/size/类型`，不预览。共享区**不做存储级分块**，超大文件另见 `tests/gaps.yaml` 的 `work.large-asset-transfer`。
  - 内容 IO 一律分块流式（`WorkStore` 的 `stat_under` / `scan_under` / `stat_object` / `scan_object` / `ingest_under` / `copy_object_to_under`）：pull 不整份读对象，commit 边读边算指纹边写对象，status 只算指纹，versions 只取元数据与受限预览，resolve / 恢复也走流式复制。
  - 冲突按文件级三方比较：文本可人工合并后用 `take=local`；图片 / 视频 / 二进制不能合并，只能 `take=upstream` 或 `take=local`；不做自动合并。
  - 上传入口在 base64 解码前按上限检查，Web 请求体也有上界；内置 `read` / `write` 仍是文本通道，二进制由模块工具或用户投喂产生。
- **写事务与恢复**：提交 / 物化 / 丢弃先写 `.work/pending.json`，再改主副本、提交记录、head 与基线；任一步失败由下一次操作幂等重放。元数据与对象用同目录临时文件 + rename；`.work/next-id` 只增不复用；`.work/lock` 是跨进程写闸。
- **对象回收**：`work_gc(work, apply)` 以"所有提交树 + 所有 agent 基线"为可达集合；`apply=false` 只报数量和字节，`apply=true` 才删不可达对象并清对象目录里的临时文件。入口是隐藏维护命令 `solomni --work-gc <工作名>`（默认 dry-run，`--yes` 才删）；不做后台静默清理。
- 提交记录带 `CommitAnchor(agent, line)`，回档据此按行精确定位。
- 出站端口 `Workdirs` / **`WorkStore`**（版本库落盘：流式资产原语 + 内容寻址对象 + 提交记录 + 拉取基线 + 待恢复事务 + 提交号 + 跨进程锁 + 对象回收）**只由 `service.rs` 持有**（R12）。

模块事实（含 `has_userdata`）由 `modules` 提供：沙箱把它收成 `modules_with_userdata` 一起注入，围栏派生据此决定派不派这条落点——domain 层不读盘。

沙箱还带**会话 id**（`Sandbox.session`）：围栏把它当**会话租约**（`FenceSpec.lease`）带进授权台账，同名 agent 的多个并发会话释放时互不误撤（机制侧见 [docs/kernel/README.md](../kernel/README.md) 的「释放按会话租约」）。

虚拟机档的沙箱还带**路径翻译器**（`Sandbox.path_codec`，本机档为 None = 恒等）：系统工具寻址、模块工具在 guest 里
执行、以及用户 `@` 引用的改写共用同一座桥，三处给的是同一种写法（见 [docs/tools/tools-and-roles.md](../tools/tools-and-roles.md) 的「路径模型」）。
沙箱清单 `Sandboxes` 另存同一份 `codec`：协作的 `@` 改写发生在**名单确认之前**（那时 `list` 还是空的），
由 `Sandboxes::ref_roots()` 取用。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`permission`、`prompt`
- 谁在用我：`collab`、`conductor`、`provision`（`guest-runtime` 的沙箱清单）、`session`、`tools`

## 四、改动本单元时必须同步

- 用例测试在 `src/tests/workspace.rs`；运行包与执行档位的契约不在本单元（见 [docs/vm/README.md](../vm/README.md)）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |