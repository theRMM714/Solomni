# 单元地图 · workspace

> 定位：workspace（工作区）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-04｜依据：`src/capabilities/workspace/api.rs`

> 本文是 **workspace** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/workspace/api.rs` | 入站能力面：寻址 / 文件视图 / 共享区版本化 / 备份恢复的契约与共享类型（其它能力只准用这里） |
| `src/capabilities/workspace/service.rs` | 门面与底座：`Workdirs` / `WorkStore` 只在这里（R12）；`WorkspaceService` 的构造、实例级 gate、树回溯、恢复事务、提交号预留、待恢复事务与外部工作区冲突核校 |
| `src/capabilities/workspace/service/commit.rs` | 提交族：`commit_locked` 的三方核校与对象落盘、agent 基线记录、归档点、树铺设、归档清理、自动提交与用户提交 |
| `src/capabilities/workspace/service/sync.rs` | 同步族：pull、沙箱采纳、过期读取、双向同步、导入根、跟踪同步 |
| `src/capabilities/workspace/service/history.rs` | 历史族：状态盘点、日志、单次提交视图、单文件差异、版本列表 |
| `src/capabilities/workspace/service/resolve.rs` | 冲突解决族：`ResolvePlan`、拒绝对话渲染、`work_resolve` 的选边、备份与未合并守卫 |
| `src/capabilities/workspace/service/lifecycle.rs` | 生命周期族：丢失文件、恢复、回收、回滚、归档、删除、归档恢复、还原点、提交后丢弃 |
| `src/capabilities/workspace/service/facade.rs` | `Workspace` 的唯一 trait 实现：七个基础用例直接委托 `Workdirs`，其余方法逐条转发到同级子模块（Rust 不允许 trait impl 分块） |
| `src/capabilities/workspace/ports.rs` | 出站端口：`Workdirs`（目录布局）与 `WorkStore`（流式资产原语 + 跨根复制 + 内容寻址对象 + 提交记录 + 拉取基线 + 待恢复事务 + 提交号 + 跨进程锁 + 对象枚举 / 删除） |
| `src/capabilities/workspace/domain/workspace.rs` | 工作区与沙箱的纯数据与寻址（含越界判定） |
| `src/capabilities/workspace/domain/asset.rs` | 通用资产纯逻辑：分块扫描（指纹 / 大小 / NUL 与 UTF-8 判定 / 受限预览）、`FileVersion`、扩展名媒体类型提示与单文件上限 |
| `src/capabilities/workspace/domain/base.rs` | 基线纯逻辑：`BaseRef` 精确提交号、`CommitRefusal` 拒绝原因、按路径核校（新鲜 / 从未拉取 / 新文件） |
| `src/capabilities/workspace/domain/freshness.rs` | 读取过期纯逻辑：读过的行在新版本里是否原样还在（按内容找连续行窗口，不看行号与时间） |
| `src/capabilities/workspace/domain/hash.rs` | 内容指纹 SHA-256：增量实现，共享区内容寻址的唯一键（纯逻辑） |
| `src/capabilities/workspace/domain/readfact.rs` | 读取事实 `ReadFact`：哪个工作树、哪个路径、读到哪几行、当时的整份指纹（随工具行落盘，过期由它派生） |
| `src/capabilities/workspace/domain/workstore.rs` | 版本化工作区纯逻辑：内容寻址、提交记录、拉取基线、文件级三方比较、三方版本预览、冲突解决判定（含 take=local 守卫 / 备份判定）与路径校验 |
| `src/capabilities/workspace/detail/fs_workspace.rs` | `Workdirs` 的落盘实现：`session/<工作名>/` 下的 `work/` 与各 agent 沙箱；外部工作区绑定投影 `.work-root` 与共享区根解析；跨会话写锁按落点身份做键；用量统计带 2 秒内存缓存且不跟随目录链接 |
| `src/capabilities/workspace/detail/fs_workstore.rs` | `WorkStore` 的落盘实现：流式扫描 / 写入 / 复制（含跨根备份复制）、原子写、跨进程锁、版本库对象与对象枚举 / 删除、提交记录、基线、待恢复事务与提交号计数器 |
| `src/capabilities/workspace/detail/estimate.rs` | 外部工作区成本预估：有界计数（250 ms / 5 万条目，超限标 `truncated`，数字是下界）、按卷速率样本（`.home/fence-costs.json`，扩展前缀与盘符写法归一到同一个卷键；真实授权与 `--setup=fence-calibrate` 的合成树样本都走 `record_grant_cost`）与按取整点值算的乐观 / 保守区间 |
| `src/capabilities/workspace/detail/grant_job.rs` | 外部工作区的后台一次性授权：按规范化根去重、进程内串行、成功后把授权时数到的对象数与耗时写 `.home/workspace-grants.json`（失败只记 `last_error`、保留上次成功数字），失败不阻断会话创建 |
