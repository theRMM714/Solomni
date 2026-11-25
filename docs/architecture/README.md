# architecture（架构细则）

> 定位：`docs/architecture/` 的目录索引：架构约束按事项分文件，根门户 `ARCHITECTURE.md` 只做路由，本页不复述细则。
> 效力：索引｜核对：2026-11-17

## 本目录

| 要找什么 | 读哪一份 |
| --- | --- |
| 三层与地基、层表、开放架构五条、单元与子业务的分工 | [layers.md](layers.md) |
| 什么该是端口、有哪些端口、谁持有它 | [ports.md](ports.md) |
| 状态归谁、落盘契约、运行日志、提示词册 | [state.md](state.md) |
| 业务边界判据、状态归属、三分法、粒度上限、硬要求 R1–R20、T0 判据 | [boundaries.md](boundaries.md) |
| 注释契约：文件头四槽、条目的 `///` 槽、行内限量、零豁免 | [comments.md](comments.md) |
| Rust 模块布局、格式与静态检查、unsafe 与平台分叉、跨平台机制、依赖 | [rust-dev.md](rust-dev.md) |

## 按改动找入口

| 要找什么 | 读哪一份 |
| --- | --- |
| 新增一个能力、判断它该不该独立 | [boundaries.md](boundaries.md) 的「业务边界判据」→「单元与子业务的分工」（[layers.md](layers.md)） |
| 动依赖边、给能力归层 | [layers.md](layers.md)：动过边就重算全表 |
| 给某处 IO 加端口或换适配器 | [ports.md](ports.md) |
| 新增落盘目录、改布局、加日志埋点、加提示词段 | [state.md](state.md) |
| 拆文件、判断多大算过大 | [boundaries.md](boundaries.md) 的「粒度上限」 |
| 写文件头、`pub` 项文档与行内注释 | [comments.md](comments.md) |
| 加平台后端、处理路径与编码 | [rust-dev.md](rust-dev.md) 的「unsafe 与平台分叉」「跨平台机制」 |

## 改动时必须同步

- 本目录增删或改名细则 → 本页路由表、`docs/templates/index.json` 的 spec 登记、`ARCHITECTURE.md` 的路由表（三处同次改）
- 改 `layers.yaml` 的环或边 → 同次跑 `node tests/gates/structure/layers.js` 看环表、rank 与命中，环表生成段由 `node tests/docs-sync.mjs --write` 写回
- 改硬要求 R 清单 → `AGENTS.md` 文档路由表里的 R 编号范围与 `TESTING.md` 的判据引用同次核对
- 本页增删入口 → `docs/README.md` 的架构分区同次改
