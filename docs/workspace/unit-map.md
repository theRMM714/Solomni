# 单元地图 · workspace

> 本文是 **workspace** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/workspace/api.rs` | 入站能力面：寻址 / 文件视图 / 共享区版本化的契约与共享类型（其它能力只准用这里） |
| `src/capabilities/workspace/service.rs` | 用例与端口持有者：`Workdirs` / `WorkStore` 只在这里（R12）；版本库写事务与恢复、提交号分配、对象回收、pull / commit / 三方视图 / 显式解决冲突 / 状态 |
| `src/capabilities/workspace/ports.rs` | 出站端口：`Workdirs`（目录布局）与 `WorkStore`（流式资产原语 + 内容寻址对象 + 提交记录 + 拉取基线 + 待恢复事务 + 提交号 + 跨进程锁 + 对象枚举 / 删除） |
| `src/capabilities/workspace/domain/workspace.rs` | 工作区与沙箱的纯数据与寻址（含越界判定） |
| `src/capabilities/workspace/domain/asset.rs` | 通用资产纯逻辑：分块扫描（指纹 / 大小 / NUL 与 UTF-8 判定 / 受限预览）、`FileVersion`、扩展名媒体类型提示与单文件上限 |
| `src/capabilities/workspace/domain/hash.rs` | 内容指纹 SHA-256：增量实现，共享区内容寻址的唯一键（纯逻辑） |
| `src/capabilities/workspace/domain/workstore.rs` | 版本化工作区纯逻辑：内容寻址、提交记录、拉取基线、文件级三方比较、三方版本预览、冲突解决判定与路径校验 |
| `src/capabilities/workspace/detail/fs_workspace.rs` | `Workdirs` 的落盘实现：`session/<工作名>/` 下的 `work/` 与各 agent 沙箱 |
| `src/capabilities/workspace/detail/fs_workstore.rs` | `WorkStore` 的落盘实现：流式扫描 / 写入 / 复制、原子写、跨进程锁、版本库对象与对象枚举 / 删除、提交记录、基线、待恢复事务与提交号计数器 |
