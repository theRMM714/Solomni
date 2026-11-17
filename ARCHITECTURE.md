# 架构（门户）

> 定位：架构约束的唯一门面：回答「要做什么、读哪一份」；细则在 `docs/architecture/`，本页不复述判据。
> 效力：索引｜核对：2026-11-17

## 架构细则

| 要找什么 | 读哪一份 |
| --- | --- |
| 三层与地基、层表、开放架构五条、单元与子业务的分工 | [docs/architecture/layers.md](docs/architecture/layers.md) |
| 什么该是端口、有哪些端口、谁持有它 | [docs/architecture/ports.md](docs/architecture/ports.md) |
| 状态归谁、落盘契约、运行日志、提示词册 | [docs/architecture/state.md](docs/architecture/state.md) |
| 业务边界判据、状态归属、三分法、粒度上限、硬要求 R1–R20、T0 判据 | [docs/architecture/boundaries.md](docs/architecture/boundaries.md) |
| 注释契约：文件头四槽、条目槽、行内限量、零豁免 | [docs/architecture/comments.md](docs/architecture/comments.md) |
| Rust 模块布局、格式与静态检查、unsafe 与平台分叉、跨平台机制、依赖 | [docs/architecture/rust-dev.md](docs/architecture/rust-dev.md) |
| 架构细则的目录索引与「按改动找入口」 | [docs/architecture/README.md](docs/architecture/README.md) |

## 相邻领域

| 要找什么 | 读哪一份 |
| --- | --- |
| 项目方向、承诺与判据 | [PHILOSOPHY.md](PHILOSOPHY.md) |
| 产品行为、运行流程与用户旅程 | [PRODUCT.md](PRODUCT.md) |
| 测试分层、替身语义、质量门禁与缺口账 | [TESTING.md](TESTING.md) |
| 模块开发与模块产物 | [MODULE_SPEC.md](MODULE_SPEC.md)、[docs/builds/README.md](docs/builds/README.md) |
| 运行包开发与修改 | [RUNTIME_SPEC.md](RUNTIME_SPEC.md) |
| providers、models、agents、settings 登记处 | [REGISTRY_SPEC.md](REGISTRY_SPEC.md) |
| 会话目录、转录、行结构、回档与压缩 | [docs/session/session-model.md](docs/session/session-model.md) |
| 提示词键表 | [docs/prompt/prompts.md](docs/prompt/prompts.md) |
| 工具、角色与工具清单 | [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md) |
| 协作任务链 | [docs/collab/task-chain.md](docs/collab/task-chain.md) |
| 呈现层契约（CLI / Web 的入站面） | [docs/presentation/contracts.md](docs/presentation/contracts.md) |
| 围栏落点、进程台账、运行日志细则 | [docs/kernel/README.md](docs/kernel/README.md) |
| 全部文档单元 | [docs/README.md](docs/README.md) |

## 改动时必须同步

- 改分层、端口、状态、边界或注释契约 → `docs/architecture/` 对应细则与本页路由表同次改
- 新增或删除能力 → `layers.yaml`、层表重算（`node tests/gates/structure/layers.js --write`）、`docs/README.md` 的单元表与 `AGENTS.md` 的文档路由同次改
- 本页增删入口 → `AGENTS.md` 的文档路由表与 `docs/README.md` 同次改
