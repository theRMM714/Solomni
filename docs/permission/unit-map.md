# 单元地图 · permission

> 定位：permission（权限）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-04｜依据：`src/capabilities/permission/api.rs`

> 本文是 **permission** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/permission/api.rs` | 入站能力面：重导出权限类型、四档预设与校验入口 |
| `src/capabilities/permission/domain/permission.rs` | 纯规则的入口：`base` / `profile` / `scope` / `layer` / `set` / `path` 六个职责子模块的声明，并向 `api.rs` 重导出同一个调用面 |
| `src/capabilities/permission/domain/permission/base.rs` | reach / effect / decision 代数：`Reach` / `ReachSet` / `Effect` / `Decision` 与等级比较 |
| `src/capabilities/permission/domain/permission/profile.rs` | 四档预设：`ProfileId` / `ProfileSpec` / `Decisions` 与编辑入口 |
| `src/capabilities/permission/domain/permission/scope.rs` | 共享区三态与手写序列化：`ScopeMode` / `SharedScope` / `SharedEdit` / `DecisionEdit` |
| `src/capabilities/permission/domain/permission/layer.rs` | 层与调用范围：`CallScope` / `PermissionDefaults` / `PermissionLayer` / `SessionSandbox` |
| `src/capabilities/permission/domain/permission/set.rs` | 权限集合运算：`PermissionSet`、按路径求交与追加拒绝 |
| `src/capabilities/permission/domain/permission/path.rs` | 路径判据与条目校验：`hit` / `normalize` / `validate_entry` / `validate_defaults` / `validate_layer`、包含与收窄关系 |
