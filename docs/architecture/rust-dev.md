# Rust 开发规范

> 定位：Rust 源码的模块布局、格式与静态检查、unsafe 边界、跨平台机制、依赖与测试入口的唯一规范。
> 效力：规范｜核对：2026-11-25｜依据：`Cargo.toml`、`tests/gates/structure/cfg-boundaries.js`

## 适用与判据
- 写或改 `.rs`、`Cargo.toml`、`deny.toml` 与模块目录结构时以本文为准；违反由 `node run-tests.js` 的结构审查、`node dev.js fmt` 与 `node dev.js clippy` 判定。

## 一、模块布局

- 模块入口一律用与目录同名的文件，不得再出现 `mod.rs`（`tests/gates/structure/module-files.js`）。
- 入口与子模块目录并列，入口只做子模块声明与重导出，实现留在子模块里（`src/units/maintenance/builds.rs`）：

```text
units/maintenance/
├── builds.rs            # 子业务入口：声明 api / ports / service / detail
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

- 格式以 `cargo fmt --check` 为准，不手工对齐；写格式跑 `node dev.js run -- cargo fmt --all`（`node dev.js fmt` 只做 `--check`，报告偏差、不改文件）。
- 静态检查以 `node dev.js clippy` 为准，`-D warnings` 下零告警；不得用 `#[allow]` 掩盖告警，确需放宽的逐条登记在 `Cargo.toml` 的 lints 段（`Cargo.toml`）。
- edition 固定为 2024；工具链版本与 `rust-version` 同步升级（`Cargo.toml`）。

## 三、unsafe 与平台分叉

- unsafe 只允许出现在围栏与平台后端，并在同处写明安全前提（`src/kernel/detail/confine.rs`）。
- 平台差异只允许出现在四种地方（判据：`tests/gates/structure/cfg-boundaries.js`）：
  - **平台后端**：文件名或目录名是 `windows` / `linux` / `macos` / `unix` / `other`（如 `src/kernel/detail/process/windows.rs`、`src/kernel/detail/confine/windows/`）；
  - **后端选择**：紧邻 `mod` / `use` / `pub use` 的 `#[cfg(...)]`（`src/kernel/detail/confine.rs` 的后端别名、`src/kernel/detail.rs` 的平台重导出）；
  - **平台事实**：`src/kernel/domain/platform.rs`（常量与纯判断，如 `.exe` 后缀、PATH 分隔符、宿主路径大小写、`IS_WINDOWS` / `HOST_ARCH`）；
  - **平台测试**：`src/tests/**`、与实现同目录的 `tests.rs`、测试项上的 `#[cfg(...)]`。
- 公共路径（`api` / `service` / `domain` / 普通 `detail` / 呈现层 / 根）不得出现平台 `#[cfg]` 分支或 `cfg!` 表达式：差异下沉到机制后端的稳定契约，调用方只认门面函数（`src/kernel/detail/process.rs`）。
- 运行期能力差异用探针（`HostProbe` / `FenceVerdict`）表达，不用 cfg：cfg 管「这个平台有没有实现」，探针管「本机能不能用」。
- 违反由 `structure/cfg-boundaries` 零豁免判定（`tests/gates/structure/cfg-boundaries.js`）。

## 四、跨平台机制

- 路径一律用 `PathBuf` / `Path` 组件拼接：**不得把 `/` 或 `\` 写进字符串再拼**（`src/kernel/domain/path.rs`）。
- **对外**（提示词、工具参数、回执、API）一律用 `/` 书写形式（Windows 反斜杠在 JSON 字符串里是非法转义），由 `src/kernel/domain/path.rs` 的 `slash` 统一给出。
- 编码：读严格 UTF-8、非法字节如实标注（**不猜编码**）；写一律 UTF-8；为工具子进程强制 UTF-8 环境（`src/units/tools/detail/sys_io.rs`）。
- 不假设平台：不写死盘符；模块工具的命令由模块作者声明，系统 `shell` 按平台选 sh / PowerShell。
- 平台差异按机制分后端：门面 `<机制>.rs` 零平台分支，实现落在 `<机制>/{windows,linux,macos,unix,other}.rs` 或平台目录（`src/kernel/detail/process.rs`、`src/kernel/detail/confine/`）；共享常量与纯事实只在 `src/kernel/domain/platform.rs`。cfg 的位置由 `structure/cfg-boundaries` 零豁免判定（本文第三节）。
- 某平台没有接入的部分**就是没有**——文档与界面照实说。

## 五、错误与日志

- 可恢复失败返回错误并带上上下文，不用 `panic` / `expect` 走正常路径（`src/kernel/api.rs` 的错误类型）。
- 用户可见提示与日志遵循日志契约（[state.md](state.md) 的「运行日志」）。

## 六、依赖与供应链

- 新增或升级依赖先过 `deny.toml` 的许可证、来源与禁用策略（`deny.toml`）。
- 发行版保持纯 Rust：不得引入需要额外 C 工具链的 crate（`AGENTS.md`）。
- 版本约束只写在 `Cargo.toml`，锁文件随同一次提交更新（`Cargo.lock`）。

## 七、测试入口

- 测什么层、放哪里、怎么判定按测试门户执行（`TESTING.md`）。
- 新增测试文件必须能从入口追溯到执行位置，否则算孤儿文件（`tests/gates/structure/targets.js`）。

## 检查

- 模块布局（不得有 `mod.rs`）→ `structure/module-files`
- 单元地图与代码注释里的文档引用 → `structure/unit-map`、`structure/doc-refs`
- 分层依赖方向（谁能依赖谁）→ `structure/dependency`
- 跨平台路径与平台分叉 → `structure/cfg-boundaries`、`structure/values`
- 平台 cfg 边界 → `structure/cfg-boundaries`（`tests/gates/structure/cfg-boundaries.js`）
- 格式 → `node dev.js fmt`；静态检查 → `node dev.js clippy`
- 许可证、来源与禁用 → T0 供应链步骤（`deny.toml`）

## 路由

- 架构细节目录（分层 / 端口 / 状态 / 边界 / 注释）：[README.md](README.md)
- 硬要求清单 R1–R20：[boundaries.md](boundaries.md)
- 测试与质量：[../testing/README.md](../testing/README.md)
- 文档约定与术语：[../conventions.md](../conventions.md)
- 任务流程（受理、开工、闸门）：[../../AGENTS.md](../../AGENTS.md) 的「六、请求受理」