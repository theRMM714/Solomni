# 上下文新鲜度与全局版本化 · 施工计划

> 临时施工文档，不对外。三个 PR 顺序合并：每完成一个，删掉对应章节并把状态改成"已合并"；三个全部合并后删除本文件，并移除 `tests/gates/docs/root-router.js` 里 `plan.md` 的白名单条目。

## 0. 状态

| # | 分支 | 内容 | 状态 |
| --- | --- | --- | --- |
| 1 | `task/ws-refs` | 存储层：refs、增量提交、祖先遍历 GC | 实现完成，待评审 |
| 2 | `task/context-freshness` | 写路径提交、读事实、过期判定 | 待开始 |
| 3 | `task/archive-checkpoint` | 留档全局检查点、恢复剪叉、开关与读数 | 待开始 |

顺序依赖：2 需要 1 的 refs 与增量提交；3 需要 2 的读事实。每个 PR 从最新 `main` 切，前一个合并后才开下一个。

## 1. 通用约定

### 1.1 不变量

- **I1** 一次读取发生时，文件状态等于某条提交。
- **I2** 提交准入：分支 `base` == `refs/main`，否则要求先 pull。
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

## 2. PR1 存储层

### 2.1 目标

把"提交 = 整棵树快照 + GC 枚举全部提交"改成"提交 = 增量 + 从 refs 遍历"。**外部行为不变**，本 PR 不改变任何工具或会话可见结果。

### 2.2 改动文件

| 文件 | 改动 |
| --- | --- |
| `src/capabilities/workspace/domain/workstore.rs` | `Commit.tree` 改 `Option<Tree>`；增 `base`、`depth`、`source`；新增 `Source`/`Ref`/`RefKind`；`GcReport` 增 `removed_commits` |
| `src/capabilities/workspace/ports.rs` | `WorkStore` 增 `read_ref`/`write_ref`/`list_refs`/`remove_ref`（`remove_commit` 原本已有） |
| `src/capabilities/workspace/detail/fs_workstore.rs` | refs 落 `.work/refs/<name>.json`；`head` 走 `refs/main`，旧 `head` 文件读到一次即升级并删除 |
| `src/capabilities/workspace/service.rs` | 三处提交构造改落增量；`tree_of` 回溯重建；`work_gc` 从 refs 算活集合并删不活提交；回档 / 留档前 `pin_head` 钉 `refs/live/<id>`；丢弃后清悬空引用 |
| `src/tests/workspace.rs`、`src/tests/detail.rs`、`src/tests/doubles.rs` | 增量树、refs 往返、旧 head 升级、GC 两态；内存替身补 refs 端口 |

### 2.3 结构

```rust
pub struct Commit {
    pub id: u64,
    pub parent: Option<u64>,
    /// 分支 fork 点；提交准入与树重建都靠它。
    #[serde(default)]
    pub base: u64,
    pub author: String,
    #[serde(default)]
    pub session: String,
    pub time: i64,
    pub message: String,
    pub changes: Vec<Change>,
    /// Some = 根提交；None = 增量，树由最近的根 + 沿途 changes 重建。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<Tree>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<CommitAnchor>,
    #[serde(default)]
    pub source: Source, // tool / pull / external / archive / checkout
}

pub enum RefKind { Main, Agent, Archive, Live }

pub struct Ref {
    pub kind: RefKind,
    pub target: Option<u64>,
    /// 归档与剪叉携带的多工作树快照：agent 名 → HEAD。
    #[serde(default)]
    pub heads: BTreeMap<String, Option<u64>>,
}
```

### 2.4 算法

**树重建**：从提交沿 `parent` 回溯到最近的 `tree: Some`；沿途 `changes` 按从旧到新正放（Add/Modify 置 hash，Delete 移除）。回溯长度超过 512 视为损坏，直接报错。

**根间隔**：写提交时若"距最近根 ≥ 256"或调用方显式要求（留档、基线、checkout），落根提交（`tree: Some`）。

**GC 两段**：

1. 根集合 = `refs/main` + `refs/agents/*` + `refs/archive/*` + `refs/live/*` + 各 agent 索引（基线）。
2. 从根回溯得活提交集合；根提交展开 `tree.values()`，增量提交取 `changes[].hash`，合成活 blob 集合。
3. 删不活的提交记录（先写 `intent`，失败可重试）。
4. 删不活的对象；保留 `is_content_hash` 之外的临时文件策略不变。
5. `GcReport` 增 `kept_commits`/`removed_commits`。

**旧线保护**：回档 / 留档这类"移 head"的操作，动 head 之前先 `pin_head`——把当前 head 写成 `refs/live/<id>`。没有这一步，新 GC 会把回档离开的提交当垃圾删掉；有钉子则旧线永远可达（零拷贝）。

**本 PR 不做 checkout / intent**：切到非祖先提交要连带"工作树有未提交改动就拒绝、force 先备份 lost"的语义，归 PR3 与留档一起做；本 PR 不引入没有调用方的死代码。

### 2.5 旧数据

旧提交本来就带整棵树，而 `tree: Option<Tree>` 对"有 tree"的 JSON 天然解析成 `Some`；`base`/`depth`/`source` 都有默认值。所以旧数据**不需要解析分支**：`head()` 在 `refs/main` 缺失时读旧 `head` 文件，把它写成 `refs/main` 再删旧文件；旧提交因 `depth = 0` 一律当根。原计划的"报错要求重建"作废——那会让用户丢历史，且迁移本身只有几行。

### 2.6 测试

`src/tests/workspace.rs` 新增：

- `work_commits_store_roots_and_deltas_but_read_full_tree`：只有根带整棵树，增量只记 changes；`work_status` 的树仍完整；
- `work_gc_reclaims_commits_only_after_refs_drop`：有 `live` 钉子时提交全保住；引用全删后才回收；
- `fs_work_store_round_trips_and_refuses_symlink_escapes` 增补：旧 `head` 一次性升级成 `refs/main`、refs 写 / 列 / 删往返。

### 2.7 收尾

- [ ] PR 合并后删除本章，状态改"已合并"
- [ ] 更新 `docs/workspace/` 的存储与回收说明

## 3. PR2 写路径提交与过期判定

### 3.1 目标

每次工具写后提交；读事实落工具行；发送前按 refs 链重算过期并替换占位符。开关默认关（只记录、不呈现）。

### 3.2 改动文件

| 文件 | 改动 |
| --- | --- |
| `src/capabilities/collab/service/tool_loop.rs` | 工具执行边界统一提交；回合末 sweep |
| `src/capabilities/tools/domain/systool.rs` | `Observations` 暴露 `touched` 路径集合；read 产出 `ReadFact` |
| `src/capabilities/tools/service/systool.rs` | read 前确保状态已提交；记录 blob |
| `src/capabilities/workspace/service.rs` | `work_commit_auto`、`work_sync_branch`、pull 落提交 |
| `src/capabilities/session/domain/freshness.rs`（新增） | 过期判定纯函数 |
| `src/capabilities/session/domain/events.rs` | 工具结果 payload 增 `read` 事实 |
| `src/capabilities/collab/service/round.rs` | `assemble` 在压缩边界之后替换占位符 |
| `src/capabilities/session/domain/session.rs` | `meta.context_freshness` |

### 3.3 算法

**写路径**：工具调用后取 `Observations.touched` 调 `work_commit_auto(work, agent, touched)`，无差异 no-op；回合末 `work_sync_branch` 全量扫沙箱，差异落 `source=external` 提交，兜 shell/模块/外部编辑。`work_pull` 末尾落 `source=pull` 提交。`work_commit` 准入改 fast-forward。

**读事实**：`ReadFact { worktree, path, from, to, blob, complete }` 随工具结果行落盘。读之前若该路径有未提交差异，先落一条单路径 `external` 提交，保证 blob 在库里。

**判定**：沿 `worktree` 对应 ref 的提交链找同路径后续 `Change`；blob 不同则 diff，按 16 行块判相交；命中即整条读取标过期。时间戳、遍历顺序不得进判定。

**呈现**：只在压缩边界之后替换，同回合批量；占位符文本 `[已过期：<path> <from>-<to>，读取后文件已修改]`。

### 3.4 测试

- 同回合并发工具，提交顺序 = 调用顺序；
- 崩溃后 sweep 自愈；fast-forward 拒绝有明确错误；
- 文件头插一行不让整份读失效；块内改一行只失效相交结果；
- shell 改文件也能判过期；两次 `assemble` 逐字节相同。

### 3.5 收尾

- [ ] PR 合并后删除本章
- [ ] 更新 `docs/session/` 的发送视图说明

## 4. PR3 留档检查点与恢复

### 4.1 目标

留档 = 全局检查点 + ref 分叉；恢复 = 切回兄弟枝，剪叉前先钉 `live`。

### 4.2 改动文件

| 文件 | 改动 |
| --- | --- |
| `src/capabilities/session/domain/rewind.rs` | `RewindMark` 扩为 `ArchiveMark`（`heads`、`shared`、`label`） |
| `src/capabilities/session/service.rs` | 留档/恢复编排：屏障、受影响判定、跨会话 |
| `src/capabilities/workspace/service.rs` | `work_archive`、`work_restore_archive`、多工作树物化 |
| `src/capabilities/collab/service/turn_io.rs` | 在途 turn 的中断入口 |
| `src/capabilities/session/domain/events.rs` | `ContextUsage` 增 `cache_read`/`cache_creation` |
| 设置界面 | 开关与占用显示 |

### 4.3 算法

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

### 4.4 测试

- 三个 agent 一起留档后沙箱同点；
- 恢复后全部回退，且 `live` 仍可再恢复；
- 在途 turn 被中断且不留半状态；
- 档案往返 10 次后 GC 只回收 `live` 被删的那部分；
- 回放 = 实时（开关打开）。

### 4.5 收尾

- [ ] PR 合并后删除本章
- [ ] 更新 `docs/session/session-model.md` 的留档/恢复语义

## 5. 全部完成

- [ ] 删除 `plan.md`
- [ ] 移除 `tests/gates/docs/root-router.js` 的白名单条目
- [ ] 移除 `docs/templates/index.json` 里 `plan.md` 的例外
- [ ] 三处文档同步完成，`node run-tests.js` 全绿
