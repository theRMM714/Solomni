# 单元地图 · permission

> 定位：permission（权限）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-22｜依据：`src/units/pure/permission/api.rs`

> 本文是 **permission** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/pure/permission/api.rs` | 入站能力面：重导出网络档位类型与合并入口（其它能力只准用这里） |
| `src/units/pure/permission/domain.rs` | 纯规则的入口：声明 `permission` 子模块 |
| `src/units/pure/permission/domain/permission.rs` | `base` / `profile` 两个职责子模块的声明，并向 `api.rs` 重导出同一个调用面 |
| `src/units/pure/permission/domain/permission/base.rs` | 生效态 `PermissionSet`（只有 `profile`）、`allows_network` 与根 / 子两层的合并（`apply_root` / `apply_narrowing`） |
| `src/units/pure/permission/domain/permission/profile.rs` | 两档预设与声明层：`ProfileId` / `ProfileSpec` / `PermissionDefaults` / `PermissionLayer` / `SessionSandbox` |
