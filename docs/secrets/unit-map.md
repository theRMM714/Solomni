# 单元地图 · secrets

> 本文是 **secrets** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/secrets/api.rs` | 统一管理 API（`SecretOps`）与视图；未注入时的空实现 `NoSecrets` |
| `src/capabilities/secrets/ports.rs` | 存储端口（`SecretStore`） |
| `src/capabilities/secrets/service.rs` | 值缓存与按模块解析 / 脱敏（实现 `SecretOps`） |
| `src/capabilities/secrets/detail/mod.rs` | 机制实现入口（只有组合根能构造） |
| `src/capabilities/secrets/detail/yaml_secrets.rs` | `.home/secrets.yaml` 文件存储（实现 `SecretStore`；unix 下 0600） |
