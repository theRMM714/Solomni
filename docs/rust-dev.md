# Rust 开发规范

> 定位：Rust 源码的模块布局、格式与静态检查、unsafe 边界、依赖与测试入口的唯一规范。
> 效力：规范｜核对：2026-11-14｜依据：`Cargo.toml`

## 适用与判据
- 写或改 `.rs`、`Cargo.toml`、`deny.toml` 与模块目录结构时以本文为准；违反由 `node run-tests.js` 的结构审查、`node dev.js fmt` 与 `node dev.js clippy` 判定。

## 一、模块布局

- 模块入口一律用与目录同名的文件，不得再出现 `mod.rs`（`tests/gates/structure/module-files.js`）。
- 入口与子模块目录并列，入口只做子模块声明与重导出，实现留在子模块里（`src/capabilities/builds.rs`）：

```text
capabilities/
├── builds.rs            # 模块入口：声明 api / ports / service / detail
├── builds/
│   ├── api.rs
│   ├── ports.rs
│   ├── service.rs
│   ├── detail.rs        # 下一级入口
│   └── detail/
│       ├── cache.rs
│       └── fingerprint.rs
```

- 同一目录里自命名入口与 `mod.rs` 不得并存；本仓只保留自命名入口一种形态（`tests/gates/structure/module-files.js`）。
- 跨单元引用只走目标单元的入口或 `api`/`ports` 契约（`tests/gates/structure/dependency.js`）；`#[path]` 只用于测试探针共享夹具，且该文件必须能从测试入口追溯（`tests/gates/structure/targets.js`）。

## 二、格式与静态检查

- 格式以 `cargo fmt --check` 为准，不手工对齐；写格式跑 `node dev.js cargo -- fmt --all`（`node dev.js fmt` 只做 `--check`，报告偏差、不改文件）。
- 静态检查以 `node dev.js clippy` 为准，`-D warnings` 下零告警；不得用 `#[allow]` 掩盖告警，确需放宽的逐条登记在 `Cargo.toml` 的 lints 段（`Cargo.toml`）。
- edition 固定为 2024；工具链版本与 `rust-version` 同步升级（`Cargo.toml`）。

## 三、unsafe 与平台分叉

- unsafe 只允许出现在围栏与平台后端，并在同处写明安全前提（`src/kernel/detail/confine.rs`）。
- 平台差异用 `#[cfg(target_os = ...)]` 按文件分叉，不在公共路径里散落分支（`src/kernel/detail/confine/`）。

## 四、错误与日志

- 可恢复失败返回错误并带上上下文，不用 `panic` / `expect` 走正常路径（`ARCHITECTURE.md`）。
- 用户可见提示与日志遵循日志契约（`ARCHITECTURE.md`）。

## 五、依赖与供应链

- 新增或升级依赖先过 `deny.toml` 的许可证、来源与禁用策略（`deny.toml`）。
- 发行版保持纯 Rust：不得引入需要额外 C 工具链的 crate（`AGENTS.md`）。
- 版本约束只写在 `Cargo.toml`，锁文件随同一次提交更新（`Cargo.lock`）。

## 六、测试入口

- 测什么层、放哪里、怎么判定按测试门户执行（`TESTING.md`）。
- 新增测试文件必须能从入口追溯到执行位置，否则算孤儿文件（`tests/gates/structure/targets.js`）。

## 检查

- 模块布局（不得有 `mod.rs`）→ `structure/module-files`
- 单元地图与代码注释里的文档引用 → `structure/unit-map`、`structure/doc-refs`
- 分层依赖方向（谁能依赖谁）→ `structure/dependency`
- 格式 → `node dev.js fmt`；静态检查 → `node dev.js clippy`
- 许可证、来源与禁用 → T0 供应链步骤（`deny.toml`）

## 路由

- 架构、分层与硬要求：[../ARCHITECTURE.md](../ARCHITECTURE.md)
- 测试与质量：[testing/README.md](testing/README.md)
- 文档约定与术语：[conventions.md](conventions.md)
- 任务流程（受理、开工、闸门）：[../AGENTS.md](../AGENTS.md) 的「六、请求受理」
