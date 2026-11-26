# 单元地图 · workspace

> 定位：workspace（工作区）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-25｜依据：`src/units/business/workspace/api.rs`

> 本文是 **workspace** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/business/workspace/api.rs` | 入站面：寻址 / 文件视图 / 共享区版本化 / 备份恢复的契约与共享类型，以及外部工作区根策略（`product_root` / `policy` / `resolve` / `ambient` / `recheck`）的导出（其它能力只准用这里） |
| `src/units/business/workspace/service.rs` | 门面与底座：`Workdirs` / `WorkStore` 只在这里（R12）；`WorkspaceService` 的构造、实例级 gate、树回溯、恢复事务、提交号预留、待恢复事务与外部工作区冲突核校；`work_gate_info` 的受管判定与当前 / 基线版本快照 |
| `src/units/business/workspace/service/roots.rs` | 外部工作区根的策略：按档位判网络路径 / 共享根 / 文件系统根 / UTF-8 与产品根的互包含（文本 + 落点身份），并做目录与可写探测；创建走 `resolve`，CLI 启动目录走 `ambient`（同一条阶梯、不探可写），加载复检走 `recheck`，三种用法共用同一份 `policy` 与 `check_bounds` |
| `src/units/business/workspace/service/commit.rs` | 提交族：`commit_locked` 的两条线分派与对象落盘、`refs/agents/<agent>`（发布线）与 `refs/agents/<agent>/saves`（存档线）的读写、归档点、树铺设、归档清理、存档提交与用户提交 |
| `src/units/business/workspace/service/diff.rs` | 差异族：`work_diff` 取两侧树（工作线头 / 指定提交 / 沙箱扫描）、按对象与沙箱文件取文本、组装 `DiffReport` |
| `src/units/business/workspace/service/sync.rs` | 同步族：pull、沙箱采纳、过期读取、双向同步、导入根、跟踪同步 |
| `src/units/business/workspace/service/history.rs` | 历史族：状态盘点、日志、单次提交视图、单文件差异、版本列表 |
| `src/units/business/workspace/service/resolve.rs` | 冲突解决族：`ResolvePlan`、拒绝对话渲染、`work_resolve` 的选边、备份与未合并守卫 |
| `src/units/business/workspace/service/reset.rs` | 回退族：`work_reset` 的祖先链校验、`paths` 形态按基线铺回 / 删除、`to` 形态移线与重铺工作线跟踪的路径、覆盖前备份进 `lost/` |
| `src/units/business/workspace/service/lifecycle.rs` | 生命周期族：丢失文件、恢复、回收、回滚、归档、删除、归档恢复、还原点、提交后丢弃 |
| `src/units/business/workspace/service/ignore.rs` | 过滤名单族：`work_ignore` 的 list / add / remove、逐条越界校验、行数与字节上限、整份规则的可编译校验与原子写回（只动版本库目录里的 `.workignore`） |
| `src/units/business/workspace/service/facade.rs` | `Workspace` 的唯一 trait 实现：七个基础用例直接委托 `Workdirs`，其余方法逐条转发到同级子模块（Rust 不允许 trait impl 分块） |
| `src/units/business/workspace/ports.rs` | 出站端口：`Workdirs`（目录布局）与 `WorkStore`（流式资产原语 + 按 `.workignore` 剪枝的清单 `list_visible` + 版本库小文本读 `read_text` / 写 `write_text` + 跨根复制 + 内容寻址对象 + 提交记录 + 拉取基线 + 待恢复事务 + 提交号 + 跨进程锁 + 对象枚举 / 删除） |
| `src/units/business/workspace/service/work_tools.rs` | 成员侧工作工具执行者：`WorkHandler` 认领 `work_pull` / `work_commit` / `work_push` / `work_sync` / `work_status` / `work_ignore` / `work_versions` / `work_diff` / `work_resolve` / `work_reset` / `work_lost`，参数解析与路径权限在这里收口；`work_handlers` 按席位面发放的 `allowed` 现取执行者 |
| `src/units/business/workspace/domain/workspace.rs` | 工作区与沙箱的纯数据与寻址（含越界判定）；`Sandbox.filter_rules` 指向版本库目录里的 `.workignore`——检索面（grep）与内容面共用这一份规则文件 |
| `src/units/business/workspace/domain/asset.rs` | 通用资产纯逻辑：分块扫描（指纹 / 大小 / NUL 与 UTF-8 判定 / 受限预览）、`FileVersion`、扩展名媒体类型提示与单文件上限 |
| `src/units/business/workspace/domain/freshness.rs` | 读取判决纯逻辑：读过的行在新版本里是否原样还在（按内容找连续行窗口，不看行号与时间），结论分 `SameVersion` / `Shifted` / `Changed` 三态 |
| `src/units/business/workspace/domain/hash.rs` | 内容指纹 SHA-256：增量实现，共享区内容寻址的唯一键（纯逻辑） |
| `src/units/business/workspace/domain/path_filter.rs` | 工作区过滤的唯一名单与匹配规则：`.workignore` 的文件名与播种内容、把规则文本编译成与 git 同引擎的匹配器（`Rules`）、按相对路径判定忽略（命中的目录连带整棵子树）；内容面与检索面都从这里取规则，不另立名单 |
| `src/units/business/workspace/domain/readfact.rs` | 读取事实 `ReadFact`：哪个工作树、哪个路径、读到哪几行、当时的整份指纹（随工具行落盘，过期由它派生） |
| `src/units/business/workspace/domain/scope.rs` | 共享区路径归属的唯一实现：三态 `ScopeMode`（all / none / 路径数组，手写序列化）、`SharedScope`（读 / 写 / 黑名单的判定、求交、收窄、合并全局硬底线）、`SharedEdit`（会话级覆盖）与条目规范化 `hit` / `normalize` / `validate_entry` |
| `src/units/business/workspace/domain/work_tools.rs` | 成员侧工作工具的回执渲染纯逻辑：工具名清单（`is_work_tool`）、拉取 / 提交 / 推送 / 同步 / 状态 / 过滤名单 / 三方视图 / 差异 / 解决 / 回退 / 备份恢复的回执文本 |
| `src/units/business/workspace/domain/workstore.rs` | 版本化工作区纯逻辑的入口：`record` / `path` / `merge` / `resolve` / `status` / `diff` / `reset` 七个职责子模块与内联单测 `tests` 的声明，并向 `api.rs` / `service` 重导出同一个调用面 |
| `src/units/business/workspace/domain/workstore/record.rs` | 版本库记录形态：`Index` / `Tree` / `Change` / `Commit` / `Ref` 与待提交、对象信息、GC 报告 |
| `src/units/business/workspace/domain/workstore/path.rs` | 路径纯逻辑：相对路径清理、批量校验、拼接、展开与丢失条目判定 |
| `src/units/business/workspace/domain/workstore/merge.rs` | 三方比较与冲突：`Conflict` / `PullVerdict` / `Unchanged` / `CommitPlan`、`plan_commit` / `pull_verdict` |
| `src/units/business/workspace/domain/workstore/resolve.rs` | 版本预览与解决判定：`VersionView` 族、`local_take_verdict`、备份与丢失报告 |
| `src/units/business/workspace/domain/workstore/status.rs` | 状态与回执：`StatusState` / `status_of`、拉取 / 提交 / 同步 / 状态报告 |
| `src/units/business/workspace/domain/workstore/diff.rs` | 文本差异纯逻辑：Myers 行级差分、带上下文的 hunk 归并、单文件上限与 `DiffRequest` / `DiffReport` 形态 |
| `src/units/business/workspace/domain/workstore/reset.rs` | 回退请求与报告形态：`ResetRequest` / `ResetReport` |
| `src/units/business/workspace/domain/workstore/tests.rs` | workstore 纯逻辑单测：路径清理、提交规则、拉取不覆盖本地、解决判定、丢失与备份 |
| `src/units/business/workspace/detail/fs_workspace.rs` | `Workdirs` 的落盘实现：`data/session/<工作名>/` 下的 `work/` 与各 agent 沙箱；准备会话时在版本库目录播种 `.workignore`（已存在不覆盖）；外部工作区绑定投影 `.work-root` 与共享区根解析；跨会话写锁按落点身份做键；用量统计带 2 秒内存缓存且不跟随目录链接 |
| `src/units/business/workspace/detail/walk.rs` | 工作区目录遍历的唯一实现：递归列文件、逐层按名字排序、给了 `.workignore` 规则就整棵剪掉被忽略的子树、按条数上限截断；`fs_workspace` 的清单与 `fs_workstore` 的 `list` / `list_visible` 共用 |
| `src/units/business/workspace/detail/fs_workstore.rs` | `WorkStore` 的落盘实现：流式扫描 / 写入 / 复制（含跨根备份复制）、按规则剪枝的目录列举、版本库小文本读 `read_text` / 原子写 `write_text`、跨进程锁、版本库对象与对象枚举 / 删除、提交记录、基线、待恢复事务与提交号计数器；目录刷新经 `fs_meta::sync_dir` |