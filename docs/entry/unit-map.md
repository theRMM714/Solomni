# 单元地图 · entry

> 本文是 **entry** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 入口层（组合根 / 探针 / 守门进程）+ 入口层共用机制。只有程序入口能用它，**任何能力都不许依赖它**（门禁判定）；
> 它依赖所有人，**任何人都不许依赖它**。三个入口各司其职，互不借用。

| 文件 | 职责 |
| --- | --- |
| `src/entry/mod.rs` | 入口层共用机制的入口（当前只有产品根规范化） |
| `src/entry/root.rs` | 产品根规范化：纯词法拼接与归一化，取不到当前目录才退回 canonicalize |
| `src/entry/toolchains.rs` | 启动期基础工具链：读 `.tools/toolchains/active.json`，校验目标与相对路径，把项目 bin 前置到 PATH 并设置工具环境；缺失 / 损坏只报告 |
| `src/main.rs` | 组合根 + argv 分发：隐藏模式 / 探针 → 装配各能力 → 启动对账与围栏报告（本机能力 / 本次实际）→ 交给 CLI 或 Web |
| `src/diagnostics/mod.rs` | 机器可读探针：doctor / provision / https-check / unpack / print-routes / print-fence-env / fence-verify / build |
| `src/guard/mod.rs` | 守门模式分发：围栏守门（--fence-run）、进程守门（--proc-watchdog）、围栏台账按条处置 |
