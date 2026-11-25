# 单元地图 · entry

> 定位：根逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-15｜依据：`src/entry.rs`

> 本文是 **entry** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> 根（组合根 / 内部协议探针 / 守门进程）+ 根共用机制。只有程序入口能用它，**任何单元都不许依赖它**（门禁判定）；
> 它依赖所有人，**任何人都不许依赖它**。启动、探针、守门三个入口各司其职，互不借用；共用装配只在 `src/entry/assembly.rs`。

| 文件 | 职责 |
| --- | --- |
| `src/entry.rs` | 根共用机制的入口：注册装配、探针、产品根规范化与启动期工具链 |
| `src/entry/args.rs` | 内部协议分派：取根前的守门子进程、探针与路由目录，取根后的围栏台账；产品命令不在这里（启动相在 `src/presentation/cli/boot.rs`） |
| `src/entry/portable.rs` | 启动期的便携对账：围栏机器级预授的缺项提示、首次授权后的一次性预授；`--setup` 本身是启动相命令（实现在 `system` 单元） |
| `src/entry/assembly.rs` | 组合根的共用装配：把产品根变成各单元面；端口选型、适配器构造与依赖顺序只在这里落一次 |
| `src/entry/root.rs` | 产品根规范化：纯词法拼接与归一化，取不到当前目录才退回 canonicalize |
| `src/entry/toolchains.rs` | 启动期基础工具链：读 `.tools/toolchains/active.json`，校验目标与相对路径，把项目 bin 前置到 PATH 并设置工具环境；每次启动报告项目内 / 宿主回退（含 Windows 围栏可授权性）/ 缺失；不阻断启动 |
| `src/entry/startup.rs` | 启动门：围栏能力与授权、缺项报告、Windows 首次引导、围栏核对，以及与 Web 概览共用的 `FenceReport` |
| `src/entry/web.rs` | Web 通道启动：把围栏事实翻成呈现层结构后交给 `presentation::web::serve` |
| `src/main.rs` | 组合根：启动形态、产品根规范化、基础工具链 → 启动相命令（有 `flags.command` 就地执行后退出）→ 启动对账与围栏报告 → 首次引导与启动门（过不了只降级 / 重设 / 退出）→ 经 `entry::assembly` 装配各单元 → 交给 CLI 或 Web；内部协议分派见 `src/entry/args.rs` |
| `src/entry/probes.rs` | 内部协议探针的入口：导出围栏环境与验证、出站 HTTPS、解归档、路由目录四个模式 |
| `src/entry/probes/fence.rs` | 探针：`--print-fence-env` / `--fence-verify`（围栏环境与机制验证） |
| `src/entry/probes/https.rs` | 探针：`--https-check`（出站 HTTPS） |
| `src/entry/probes/routes.rs` | 探针：`--print-routes`（HTTP 路由目录） |
| `src/entry/probes/unpack.rs` | 探针：`--unpack`（解归档） |
| `src/guard.rs` | 守门模式分发：围栏守门（--fence-run）、进程守门（--proc-watchdog）、围栏台账按条处置 |