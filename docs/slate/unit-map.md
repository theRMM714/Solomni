# 单元地图 · slate

> 本文是 **slate** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/slate/api.rs` | 入站能力面：拟名单的 DTO 与用例签名（`Mode` / `Pick` / `Proposal` / `Parties` / `Request`）与 `propose` 的重导出 |
| `src/capabilities/slate/service.rs` | 拟名单的用例实现：请核心报一份名单 → 按登记处（agent / 模型）与模块公地逐条核验 → 按模式收束（纯规则在 `domain/proposal.rs`） |
| `src/capabilities/slate/domain/proposal.rs` | 名单收束的纯规则：协作原样、单 agent 最多一条（多条并成一个当场组装的 agent） |
