# packages（运行包）

> 定位：运行包契约、包库扫描、目标筛选与选包计划。
> 效力：规范｜核对：2026-11-04｜依据：`src/capabilities/packages/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；作者契约见 [RUNTIME_SPEC.md](../../RUNTIME_SPEC.md)，分层与业务边界见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §九。

## 管什么 / 不管什么

- 管：`package.yaml` 契约与校验；`runtimes/` 包库的扫描事实；按**发起方目标**（本机 / guest 平台）选载荷；选型诊断（缺包 / 多版本 / 定版 / 冲突 / 目标不匹配 / 无 bin）；`PackagePlan` 的装载顺序、本机 PATH 与只读根；按模块汇总的不可用能力。
- 不管：guest 生命周期、挂载 / 软链 / system 叠写（`vm` 消费 `PackagePlan` 后自己做）；实际工具进程（`tools`）；会话装配与工具面（`conductor`）；构建编排与补编（`builds`）。

## 入站契约与状态归属

- 入站面：`api::Packages`（`library` / `runtimes_dir` / `base`）+ 纯逻辑入口（`plan_host` / `plan_guest` / `diagnoses` 等）。
- 出站端口：`PackageSource` **只由 `src/capabilities/packages/service.rs` 持有**（R12）。
- 状态归属：本单元不落盘；包库事实来自 `runtimes/`，选包计划交给调用方。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`modules`（清单事实里的 `Module`）
- 谁在用我：`builds`、`conductor`、`residents`、`vm`

## 一、边界

- `PackageSource` 只由 `src/capabilities/packages/service.rs` 持有（R12）；`src/capabilities/packages/detail/fs_packages.rs` 只做目录遍历与 yaml 解析。
- `src/capabilities/packages/domain/manifest.rs` 只认清单事实与目标载荷；`src/capabilities/packages/domain/plan.rs` 只做选型与诊断，不读盘、不起进程。
- 目标由调用方给：`Target::host()` / `Target::guest(platform)`；包不声明自己服务哪一档。
- `vm` 只经 `packages::api` 拿 `PackagePlan` / `PlannedPackage`，不再持有包库。
- 包库不预置、不分发；`files` 里的 sha256 用于可复现，不用于信任。

## 二、当前消费方

- `conductor`：本机档工具面（包 `bin/` 前置 PATH、包根只读放行）、运行能力报告、缺包判据与定版。
- `vm`：按 guest 平台筛库、取 `PackagePlan`，把 prefix 挂到 `/<prefix>`、system 载荷叠写进系统路径。
- `builds`：本机 / guest 构建期 `needs` 的包计划（创建会话前补编与 `--build` 共用）。
- `residents`：常驻服务起进程前的本机包 PATH 与只读根。

## 改动本单元时必须同步

- `package.yaml` 契约 → [RUNTIME_SPEC.md](../../RUNTIME_SPEC.md)；包库与选型用例在 `src/tests/packages.rs`。
- `PackagePlan` / `PlannedPackage` 的形状一改，四个消费方（`vm` 的挂载、`builds` 的补编、`conductor` 的工具面、`residents` 的起进程环境）要一起改。
- 端口矩阵（`PackageSource`）→ [docs/testing/doubles.md](../testing/doubles.md)。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
