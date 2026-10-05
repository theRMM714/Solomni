# workspace（工作区与运行包）

> 模块清单、运行包库、执行档位与沙箱寻址。
> 本目录是该单元的唯一细则入口：本页 → [`module-map.md`](module-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：模块清单（`modules/` 扫描结果）、运行包库（`runtimes/`）、`module.yaml` / `package.yaml` 契约、执行档位与执行计划派生、工作区与沙箱寻址（含越界判定）、建工作与上传、文件视图，以及**共享区版本化工作区**（内容寻址的提交记录、各 agent 的拉取基线、文件级三方比较与按提交点还原）。

**不管**：不执行工具（`tools`）、不写会话流水（`session`）、不决定谁能用哪些工具（`tools` 的角色表）。

## 二、入站契约与状态归属

`api::WorkspaceOps`（呈现层清单事实）+ `api::Workspace`（roster / library / prepare / work_has / files / roots / **work_pull / work_commit / work_commit_user / work_status / work_restore / work_head / work_rewind_to / work_restore_point / work_discard_after** …）。提交记录带 `CommitAnchor(agent, line)`，回档据此按行精确定位。出站端口 `ModuleSource` / `PackageSource` / `Workdirs` / **`WorkStore`**（版本库落盘：文件原语 + 内容寻址对象 + 提交记录 + 拉取基线 + 删提交/清 head）**只由 `service.rs` 持有**（R12）。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`llm`、`prompt`
- 谁在用我：`cli`、`collab`、`conductor`、`registry`、`session`、`slate`、`tools`

## 四、改动本单元时必须同步

- `module.yaml` 契约 → `MODULE_SPEC.md`；`package.yaml` → `RUNTIME_SPEC.md`；`src/tests/workspace.rs`。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`module-map.md`](module-map.md) | 逐文件职责（T0 与磁盘双向比对） |