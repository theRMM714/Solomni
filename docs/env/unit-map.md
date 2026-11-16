# 单元地图 · env

> 定位：env（环境事实）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-16｜依据：`src/capabilities/env/api.rs`

> 本文是 **env** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 它是**环境事实的唯一派生者**：权限面、沙箱清单、一席的围栏 / 进程环境 / 工具链根都从入参算出，
> 不取事实、不落盘、不启 guest；取事实与装配留在 `conductor`。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/env.rs` | 能力入口与公开面 |
| `src/capabilities/env/api.rs` | 入站契约：六个派生函数与事实结构的重导出 |
| `src/capabilities/env/domain.rs` | 纯派生分区声明 |
| `src/capabilities/env/domain/env.rs` | 规则本身：权限链 / 沙箱清单 / 围栏 / 进程环境 / 工具链根 / 网络开关 / 宿主只读根 |
