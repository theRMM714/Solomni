# 上下文新鲜度与全局版本化 · 施工计划

> 临时施工文档，不对外。三个 PR 顺序合并（PR1 已合并）：每完成一个，删掉对应章节并把状态改成"已合并"；三个全部合并后删除本文件，并移除 `tests/gates/docs/root-router.js` 里 `plan.md` 的白名单条目。

## 0. 状态

| # | 分支 | 内容 | 状态 |
| --- | --- | --- | --- |
| 1 | `task/ws-refs` | 存储层：refs、增量提交、祖先遍历 GC | 已合并（#53） |
| 2 | `task/context-freshness` | 写路径提交、读事实、过期判定、用户开关 | 已合并（#54） |
| 3 | `task/archive-checkpoint` | 留档全局检查点、恢复剪叉、缓存读数 | 已完成，待合并 |

顺序依赖：2 需要 1 的 refs 与增量提交；3 需要 2 的读事实。每个 PR 从最新 `main` 切，前一个合并后才开下一个。

## 1. 通用约定

### 1.1 不变量

- **I1** 一次读取发生时，文件状态等于某条提交。
- **I2** 提交准入：待提交的**每条路径**的基线 == `refs/main`，否则要求先 pull（不自动 pull、不降级）。
- **I3** 发送视图逐字节确定（遍历顺序、时间、浮点格式都不许抖）。
- **I4** 留档零删除：只新增 ref，不删提交、不改历史。
- **I5** 恢复零删除：剪叉前先钉 `refs/live/<id>`。
- **I6** GC 从 refs 遍历祖先，不再枚举全部提交。

### 1.2 术语（实现与文档用同一组词）

| 词 | 含义 |
| --- | --- |
| 工作树 | 一个检出目录 + 一个 HEAD：主副本，或某个 agent 沙箱 |
| 根提交 | 带完整 `tree` 的提交；增量提交只有 `changes` |
| 档案 | 留档产生的全局检查点，含主副本与全部受影响沙箱的旧 HEAD |
| 剪叉 | 恢复时把分叉状态收回；只动指针，不删提交 |
| 过期 | 某条读取的 blob 之后被改过；纯派生，不落状态 |
| 提交准入 | 分支 `base` 等于 `refs/main` 才允许提交 |

### 1.3 每个 PR 的流程与判据

1. 从最新 `main` 切分支 → 空提交 → push → Draft PR（正文=方案）。
2. 一个阶段一个提交；每次 push 更新 PR 的清单与测试证据。
3. 完成判据：`node dev.js fmt` / `clippy` 干净；`node run-tests.js` 全绿；纯文档改动 `node run-tests.js --docs` 全绿；`git status` 干净。
4. 不写兼容层；旧 `.work` 不做解析迁移（见 2.5）。

## 2. PR2 写路径提交与过期判定

### 2.1 目标

每次工具写后提交；读事实落工具行；发送前按 refs 链重算过期并替换占位符。开关默认关（只记录、不呈现）。

### 2.2 改动文件

| 文件 | 改动 |
| --- | --- |
| `src/capabilities/collab/service/tool_loop.rs` | 工具执行边界统一提交；批次末 `adopt_sandbox` 扫沙箱采纳 shell / 模块改动 |
| `src/capabilities/tools/domain/systool.rs` | `Observations` 暴露 `touched` 路径集合；read 产出 `ReadFact` |
| `src/capabilities/tools/service/systool.rs` | read 前确保状态已提交；记录 blob |
| `src/capabilities/workspace/domain/base.rs`（新增） | `BaseRef`、`CommitRefusal`、`stale_paths` 准入纯函数 |
| `src/capabilities/workspace/service.rs` | `work_commit_auto`、`work_adopt_sandbox`（按 `status_of` 认新增 / 修改 / 删除，只收快进）、逐路径基线落 `refs/agents/<agent>` |
| `src/capabilities/workspace/domain/freshness.rs`（新增） | 过期判定纯函数 |
| `src/capabilities/session/domain/events.rs` | 工具结果 payload 增 `read` 事实 |
| `src/capabilities/collab/service/driver.rs` | 发送视图 = `send_view`（过期读取换占位符）；读台账登记 |
| `src/capabilities/session/domain/session.rs` | 读台账 `ReadAt` 与 `freshness_view` 纯函数 |
| `src/capabilities/session/domain/history.rs` | `SessionMeta.context_freshness`（落盘） |

### 2.3 算法

**写路径**：工具调用后取 `Observations.take_written()`（这次调用真正写过的路径）调 `work_commit_auto`，无差异 = `Ok(None)`；批次里跑过 shell / 模块工具时，批次末调 `work_adopt_sandbox` 扫一遍沙箱（基线 = `status_of(index, head, local)`）：`LocalOnly` / `LocalModified` / `LocalDeleted` 一次提交（`source=external`、锚 = 批次行），`BothPresent` / `Conflict` / 超大文件逐条 `held` 并在该工具回执里点名。只读批次不扫。`work_pull` 末尾把拉到的路径基线与 `source=pull` 提交一起落。显式 `work_commit` 保留逐路径冲突判定；**只有自动提交**走 fast-forward。

**准入（已定口径）**：基线是显式事实 `BaseRef { oid }`，落在 `refs/agents/<agent>` 的 `heads: path -> oid`；没有记录 = 从未拉取，不伪造历史。两个落点——工具执行前（写前准入）与提交时（CAS 复核），都在写共享区之前判定，失败不产出 `Pending`（否则 `recover_locked` 会重放）。自动提交**不 pull**：基线过期即 `CommitRefusal::Stale` 硬失败，由模型/用户显式 `work_pull` 后再写。新文件（主线里没有、也没拉过）放行，否则模型写不出第一份文件。

**读事实**：`ReadFact { worktree, path, from, to, blob }` 随工具结果行落盘；`blob` 是读到的整份正文指纹（对象由写后自动提交落库），只有整份读到的 `read` 才留事实。

**判定**：取共享区当前提交树里同路径的 blob；与读事实的指纹相同即新鲜；路径不在树里即过期；不同则把读过的行窗口在新正文里按内容找——找不到即过期。时间戳、遍历顺序不进判定。

**呈现**：发送前一次性派生（同回合批量）；占位符文本 `[已过期：<path> 第<from>-<to>行，读取后文件已修改；需要时重新 read]`。已进摘要的旧读不再进发送视图，自然无需替换。

### 2.4 测试

- 同回合并发工具，提交顺序 = 调用顺序；
- 崩溃后 sweep 自愈；fast-forward 拒绝有明确错误；
- 文件头插一行不让整份读失效；块内改一行只失效相交结果；
- shell 改文件也能判过期；两次 `assemble` 逐字节相同。

### 2.5 进度

- [x] 2A 基线事实与准入：`domain/base.rs`、`refs/agents/<agent>` 逐路径基线（pull / 提交后写实）、`work_commit_auto`（CAS → 提交 → 基线推进）、类型化拒绝；
- [x] 2B 接线：`MemberTools.workspace`（`CollabSession` 注入，生产必为 `Some`）、`tool_loop::commit_writes` 在写工具落地后按 `Observations.take_written()` 自动提交（锚 = 工具调用行）、`work_commit_auto` 返回 `Option`（无改动不算失败）、六处临时 `allow(dead_code)` 已删；提示词与 `docs/session/session-model.md`、`docs/permission/README.md`、`systools/tools.yaml` 同步为"自动提交 + 基线过期只留在沙箱"；
- [x] 2B′ 批次末扫沙箱：`work_adopt_sandbox`（`AdoptReport { commit, changed, held }`，一次提交带新增 / 修改 / 删除，只收快进）、`tool_loop::adopt_sandbox`（只在批次跑过 shell / 模块工具时触发，回执挂最后一个跑成的调用）、提交带 `source=external` 与 `(agent, line)` 锚；外部工作区不采纳（真相是磁盘，走 `work_sync`）；`commit_locked` 的来源与锚改为调用方给；
- [x] 2C `ReadFact` 与 `freshness`：`domain/readfact.rs`（事实随工具行落盘，`ToolCallView.read`）、`domain/freshness.rs`（按内容找连续行窗口）、`work_stale_reads`（指纹相同即新鲜，不同才比行区间；对象缺失按过期）；整份读到的 `read` 才留事实；
- [x] 2E 用户开关：`set_context_freshness` 动作（写工作根 meta、整棵子树共享、生成中拒绝）+ 状态栏「工具」菜单勾选项；切换后子树从内存丢掉、按新开关重建；API 契约测试 `set_context_freshness_action_flips_the_work_switch_and_reports_it`
- [x] 2D 发送视图占位符与 `meta.context_freshness`：事实落转录、读台账按正文匹配（压缩移位不失配）、`freshness_view` 纯函数派生、单/代理会话装配与重建都装回开关与台账、开关默认关（成本由用户按钮控制）；`docs/session/session-model.md` 的「过期读取的占位符」为口径。

### 2.6 收尾

- [ ] PR 合并后删除本章
- [x] 更新 `docs/session/` 的发送视图说明（压缩与过期占位符同属"改发送视图、不动转录"）

## 3. PR3 留档检查点与恢复

### 3.1 目标

留档 = 全局检查点 + ref 分叉；恢复 = 切回兄弟枝，剪叉前先钉 `live`。

### 3.2 改动文件

| 文件 | 改动 |
| --- | --- |
| `src/capabilities/workspace/api.rs` | `ArchiveReport`；`work_archive` / `work_restore_archive` 两个面 |
| `src/capabilities/workspace/service.rs` | `archive_point` 抽出复用、`lay_down_tree`（先备份进 lost 再铺树）、`reset_agent_point`、档案引用与恢复 |
| `src/capabilities/session/domain/rewind.rs` | `RewindMark` 扩为 `ArchiveMark`（`shared` / `heads` / `label`）、`archive_label` |
| `src/capabilities/conductor/service/rewind.rs` | 留档 / 恢复编排：子会话折叠计划、agent 名单并集、标记落盘、展开子会话 |
| `src/capabilities/conductor/api/handle.rs` | `settle_tree` 屏障（作废裁决队 + 取消生成 + 轮询交回，30 秒拒绝） |
| `src/capabilities/session/domain/events.rs` | `ContextUsage` 增 `cache_read` / `cache_creation`，usage 事件与快照透出 |
| `src/capabilities/llm/detail/http_stream.rs` | `usage_of` 解析 OpenAI `prompt_tokens_details.cached_tokens` 与 Anthropic 两个 cache 字段 |
| `src/presentation/web/assets/app.js` | 折叠分界显示 `label`；上下文浮层显示缓存读 / 写；留档 / 恢复文案写明文件一起回滚 |

### 3.3 算法

**留档**：

1. 屏障（`handle.settle_tree`）：整棵子树在跑的回合先作废裁决队、取消生成，再轮询到核心交回会话；30 秒仍未收尾 = 拒绝，文件与转录都还没动。
2. 先算各子会话的折叠点（回合对齐，不落盘）→ `keep_by_agent` → 受影响 agent 名单 = 主 / 子会话名单并集。
3. `refs/archive/<mark>` 先写：`target` = 留档前主线头，`heads` = 每个受影响 agent → 同一个头（没有 per-agent HEAD）。
4. 每个受影响沙箱：不一致的文件备份进 `lost/<agent>/archive-<mark>/`，删掉，铺目标树；index / 逐路径基线对齐到目标点。
5. 主副本 `Pending::Restore` + `materialize(target)`。
6. 失败撤销档案引用。
7. **最后**写转录标记（主会话与各子会话同一个 `mark` / `shared` / `heads` / `label`），再重建会话对象。

**恢复**：

1. 屏障。
2. 读 `refs/archive/<mark>`、校验目标提交还在、外部冲突检查。
3. `pin_head`：当前线钉成 `refs/live/<id>`（零拷贝，提交一条不删）。
4. 主副本与 `heads` 里的每个沙箱物化回档案头（现状先进 `lost/<agent>/restore-<mark>/`）。
5. 移除 `refs/archive/<mark>`；转录在该标记处截断，子会话按同一 `mark` 展开。

**跨会话**：受影响名单从主 / 子会话的 `meta.agents` 并集得出；在跑的子会话在屏障里一起停掉；被回滚会话的上下文靠过期标记自然失效。

**开关**：`meta.context_freshness` 默认 false（2E 已接用户按钮）。`cache_read` / `cache_creation` 由适配器解析后进 `ContextUsage`，缺失 = None（不伪造成 0），状态栏浮层显示"缓存读 / 写"。

### 3.4 测试

- [x] 三个 agent 一起留档后沙箱同点、未提交内容进 lost 可读回（`work_archive_moves_every_sandbox_to_one_point_and_keeps_uncommitted_in_lost`）；
- [x] 恢复后全部回退，且 `refs/live` 仍可再回滚（`work_restore_archive_rewinds_every_sandbox_and_pins_the_line_it_left`）；
- [x] 在途回合被停稳再留档（`archive_stops_a_running_turn_before_rewinding`）；
- [x] 标记携带 `shared` / `heads` / `label`，标签取折叠区间最后一条内容行（`archive_folds_the_tail_and_restore_brings_it_back`、`archive_label_takes_the_last_content_line_in_the_folded_range`、`archive_mark_reads_shared_heads_and_label_only_for_archive_rows`）；
- [x] 缓存读写解析（`usage_of_reads_cache_fields_without_inventing_zeros`）；
- [x] 回放 = 实时（开关打开）：`read_facts_rebuild_identically_to_live`——读事实随工具行落盘，切换开关重建后与实时逐条相同（正文按 `tool_result_body` 现算）。

### 3.5 收尾

- [ ] PR 合并后删除本章
- [x] 更新 `docs/session/session-model.md` 的留档/恢复语义

## 4. 全部完成

- [ ] 删除 `plan.md`
- [ ] 移除 `tests/gates/docs/root-router.js` 的白名单条目
- [ ] 移除 `docs/templates/index.json` 里 `plan.md` 的例外
- [ ] 三处文档同步完成，`node run-tests.js` 全绿
