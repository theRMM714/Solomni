# 单元地图 · builds

> 本文是 **builds** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/builds/api.rs` | 入站面：`Target`（本机 / 虚拟根）、`Built`（一次确保的结果）、`Builds` 用例 trait、未接入的 `NoBuilds` |
| `src/capabilities/builds/service.rs` | 用例实现：本机档与虚拟机档两条构建路径、构建期能力选包、缓存命中判断、编完复验入口 |
| `src/capabilities/builds/detail/mod.rs` | 机制细节的入口：只声明 `fingerprint` |
| `src/capabilities/builds/detail/fingerprint.rs` | 源指纹（构建命令 + 能力 + 网络 + 源文件）与 `.vm/build-state/` 的读写 |
