# workspace（工作区与运行包）

> 模块清单、运行包库、执行档位与沙箱寻址。
> 本目录是该单元的唯一细则入口：本页 → [`module-map.md`](module-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：模块清单（`modules/` 扫描结果）、运行包库（`runtimes/`）、`module.yaml` / `package.yaml` 契约、执行档位与执行计划派生、工作区与沙箱寻址（含越界判定）、建工作与上传、文件视图。

**不管**：不执行工具（`tools`）、不写会话流水（`session`）、不决定谁能用哪些工具（`tools` 的角色表）。

## 二、入站契约与状态归属

`api::WorkspaceOps`（呈现层清单事实）+ `api::Workspace`（roster / library / prepare / write_work / files / roots …）。出站端口 `ModuleSource` / `PackageSource` / `Workdirs` **只由 `service.rs` 持有**（R12）。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`llm`、`prompt`
- 谁在用我：`cli`、`collab`、`conductor`、`registry`、`session`、`slate`、`tools`

## 四、改动本单元时必须同步

- `module.yaml` 契约 → `MODULE_SPEC.md`；`package.yaml` → `RUNTIME_SPEC.md`；`src/tests/workspace.rs`。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`module-map.md`](module-map.md) | 逐文件职责（T0 与磁盘双向比对） |