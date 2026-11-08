# 上下文新鲜度与全局版本化 · 施工计划

> 临时施工文档，不对外。三个 PR 顺序合并（PR1 已合并）：每完成一个，删掉对应章节并把状态改成"已合并"；三个全部合并后删除本文件，并移除 `tests/gates/docs/root-router.js` 里 `plan.md` 的白名单条目。

## 0. 状态

| # | 分支 | 内容 | 状态 |
| --- | --- | --- | --- |
| 1 | `task/ws-refs` | 存储层：refs、增量提交、祖先遍历 GC | 已合并（#53） |
| 2 | `task/context-freshness` | 写路径提交、读事实、过期判定 | 进行中：2A 基线事实与准入、2B 写后自动提交已落（#54） |
| 3 | `task/archive-checkpoint` | 留档全局检查点、恢复剪叉、开关与读数 | 待开始 |

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
| `src/capabilities/collab/service/tool_loop.rs` | 工具执行边界统一提交；回合末 sweep |
| `src/capabilities/tools/domain/systool.rs` | `Observations` 暴露 `touched` 路径集合；read 产出 `ReadFact` |
| `src/capabilities/tools/service/systool.rs` | read 前确保状态已提交；记录 blob |
| `src/capabilities/workspace/domain/base.rs`（新增） | `BaseRef`、`CommitRefusal`、`stale_paths` 准入纯函数 |
| `src/capabilities/workspace/service.rs` | `work_commit_auto`、逐路径基线落 `refs/agents/<agent>`、`work_sync_branch` |
| `src/capabilities/session/domain/freshness.rs`（新增） | 过期判定纯函数 |
| `src/capabilities/session/domain/events.rs` | 工具结果 payload 增 `read` 事实 |
| `src/capabilities/collab/service/round.rs` | `assemble` 在压缩边界之后替换占位符 |
| `src/capabilities/session/domain/session.rs` | `meta.context_freshness` |

### 2.3 算法

**写路径**：工具调用后取 `Observations.take_written()`（这次调用真正写过的路径）调 `work_commit_auto`，无差异 = `Ok(None)`；回合末全量扫沙箱兜 shell / 模块 / 外部编辑（待办 2B′）。`work_pull` 末尾把拉到的路径基线与 `source=pull` 提交一起落。显式 `work_commit` 保留逐路径冲突判定；**只有自动提交**走 fast-forward。

**准入（已定口径）**：基线是显式事实 `BaseRef { oid }`，落在 `refs/agents/<agent>` 的 `heads: path -> oid`；没有记录 = 从未拉取，不伪造历史。两个落点——工具执行前（写前准入）与提交时（CAS 复核），都在写共享区之前判定，失败不产出 `Pending`（否则 `recover_locked` 会重放）。自动提交**不 pull**：基线过期即 `CommitRefusal::Stale` 硬失败，由模型/用户显式 `work_pull` 后再写。新文件（主线里没有、也没拉过）放行，否则模型写不出第一份文件。

**读事实**：`ReadFact { worktree, path, from, to, blob, complete }` 随工具结果行落盘。读之前若该路径有未提交差异，先落一条单路径 `external` 提交，保证 blob 在库里。

**判定**：沿 `worktree` 对应 ref 的提交链找同路径后续 `Change`；blob 不同则 diff，按 16 行块判相交；命中即整条读取标过期。时间戳、遍历顺序不得进判定。

**呈现**：只在压缩边界之后替换，同回合批量；占位符文本 `[已过期：<path> <from>-<to>，读取后文件已修改]`。

### 2.4 测试

- 同回合并发工具，提交顺序 = 调用顺序；
- 崩溃后 sweep 自愈；fast-forward 拒绝有明确错误；
- 文件头插一行不让整份读失效；块内改一行只失效相交结果；
- shell 改文件也能判过期；两次 `assemble` 逐字节相同。

### 2.5 进度

- [x] 2A 基线事实与准入：`domain/base.rs`、`refs/agents/<agent>` 逐路径基线（pull / 提交后写实）、`work_commit_auto`（CAS → 提交 → 基线推进）、类型化拒绝；
- [x] 2B 接线：`MemberTools.workspace`（`CollabSession` 注入，生产必为 `Some`）、`tool_loop::commit_writes` 在写工具落地后按 `Observations.take_written()` 自动提交（锚 = 工具调用行）、`work_commit_auto` 返回 `Option`（无改动不算失败）、六处临时 `allow(dead_code)` 已删；提示词与 `docs/session/session-model.md`、`docs/permission/README.md`、`systools/tools.yaml` 同步为"自动提交 + 基线过期只留在沙箱"；
- [ ] 2B′ 回合末全量扫沙箱（shell / 模块 / 外部写的文件）落 `source=external` 提交；
- [ ] 2C `ReadFact` 与 `freshness`；2D `assemble` 占位符与 `meta.context_freshness`。

### 2.6 收尾

- [ ] PR 合并后删除本章
- [ ] 更新 `docs/session/` 的发送视图说明

## 3. PR3 留档检查点与恢复

### 3.1 目标

留档 = 全局检查点 + ref 分叉；恢复 = 切回兄弟枝，剪叉前先钉 `live`。

### 3.2 改动文件

| 文件 | 改动 |
| --- | --- |
| `src/capabilities/session/domain/rewind.rs` | `RewindMark` 扩为 `ArchiveMark`（`heads`、`shared`、`label`） |
| `src/capabilities/session/service.rs` | 留档/恢复编排：屏障、受影响判定、跨会话 |
| `src/capabilities/workspace/service.rs` | `work_archive`、`work_restore_archive`、多工作树物化 |
| `src/capabilities/collab/service/turn_io.rs` | 在途 turn 的中断入口 |
| `src/capabilities/session/domain/events.rs` | `ContextUsage` 增 `cache_read`/`cache_creation` |
| 设置界面 | 开关与占用显示 |

### 3.3 算法

**留档**：

1. 屏障：中断全部在途 turn，等空闲。
2. `P` = 折叠点的主副本提交；`affected = [a | base(a) != P 且 P 不是 base(a) 的祖先]`。
3. 未提交内容 → `.work/lost/<agent>/<mark>/`。
4. 物化 `P` 到主副本与所有 affected 沙箱；各沙箱落 `parent = P` 的新提交，`base = P`。
5. `refs/archive/<mark> = {shared: 旧 main, heads: 旧各 HEAD}`。
6. `refs/main = P`。
7. **最后**写转录标记（顺序反了会出现"标记说已留档、文件还在新枝"）。

**恢复**：

1. 屏障。
2. `refs/live/<mark> = {main, agents}`（零拷贝）。
3. 物化 `archive/<mark>` 的 shared + heads 到全部工作树。
4. 移除 `refs/archive/<mark>`，展开转录行。

**跨会话**：枚举 `refs/agents/*` 得到全部工作树；中断走 collab 的中断入口，拿不到就拒绝回档；被回滚会话的上下文靠过期标记自然失效。

**开关**：`meta.context_freshness` 默认 false；`cache_read`/`cache_creation` 由适配器解析后进 `ContextUsage`。文案用"过期内容自动清理"，副标题"可能增加缓存成本"。

### 3.4 测试

- 三个 agent 一起留档后沙箱同点；
- 恢复后全部回退，且 `live` 仍可再恢复；
- 在途 turn 被中断且不留半状态；
- 档案往返 10 次后 GC 只回收 `live` 被删的那部分；
- 回放 = 实时（开关打开）。

### 3.5 收尾

- [ ] PR 合并后删除本章
- [ ] 更新 `docs/session/session-model.md` 的留档/恢复语义

## 4. 全部完成

- [ ] 删除 `plan.md`
- [ ] 移除 `tests/gates/docs/root-router.js` 的白名单条目
- [ ] 移除 `docs/templates/index.json` 里 `plan.md` 的例外
- [ ] 三处文档同步完成，`node run-tests.js` 全绿
