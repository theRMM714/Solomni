# 单元地图 · builds

> 定位：builds（模块构建）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-04｜依据：`src/capabilities/builds/api.rs`

> 本文是 **builds** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/builds/api.rs` | 入站面：`Target`（本机 / 虚拟根）、`Built`（一次确保的结果）、`Builds` 用例 trait、未接入的 `NoBuilds` |
| `src/capabilities/builds/service.rs` | 用例实现：本机档与虚拟机档两条构建路径（虚拟机档在 guest 本地工作区里编、产物按差异回写）、构建期能力选包、缓存命中判断、编完复验入口 |
| `src/capabilities/builds/detail/mod.rs` | 机制细节的入口：声明 `fingerprint` 与 `cache` |
| `src/capabilities/builds/detail/fingerprint.rs` | 源指纹（构建命令 + 能力 + 网络 + 源文件）与 `.vm/build-state/` 的读写 |
| `src/capabilities/builds/detail/cache.rs` | 构建缓存卷：卷落点与上限、卷/模块的环境标识、挂载与对账脚本、源同步与差异回写脚本、同一虚拟根的构建锁 |
