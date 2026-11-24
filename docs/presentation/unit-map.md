# 单元地图 · presentation

> 定位：呈现层逐文件职责的唯一权威（渠道各一张子地图）；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-16｜依据：`src/presentation.rs`

> 本文是 **presentation** 根级文件的逐文件职责；两个渠道内部的逐文件职责见 [`docs/presentation/cli/unit-map.md`](cli/unit-map.md) 与 [`docs/presentation/web/unit-map.md`](web/unit-map.md)。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/presentation.rs` | 呈现层入口：两个渠道（`cli` / `web`）与面袋（`faces`）的模块声明 |
| `src/presentation/signal.rs` | 进程级 Ctrl+C：第一次只置标志、等消费方来取（工作窗口取走 → 停这一轮），标志没人取走还按第二次就放行默认终止（Windows 控制台处理器 / Unix `SIGINT`）；只做安装、取走与丢弃，不认识任何提示符 |
| `src/presentation/faces.rs` | 面袋 `Faces`：把核心手柄拆成各能力自己的窄 trait 对象（`work` / `turn` / `runtime` / `registry` / `history` / `modules` / `actions` / `log`）并放上组合根填的值（`provision` / `storage` / `toolchains`）；两个渠道各屏只取自己需要的那一面，只做装配、不长判定 |
