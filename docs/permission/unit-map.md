# 单元地图 · permission

> 本文是 **permission** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/permission/api.rs` | 入站能力面：重导出权限类型与校验入口 |
| `src/capabilities/permission/domain/permission.rs` | 纯规则：决定粒度、白/黑名单、模块写授权、覆盖解析与条目校验 |
