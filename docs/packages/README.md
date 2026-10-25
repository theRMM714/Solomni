# packages（运行包）

> 运行包契约、包库扫描、目标筛选与选包计划。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）。
> 作者契约见 [RUNTIME_SPEC.md](../../RUNTIME_SPEC.md)，分层与业务边界见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §九。

## 一、管什么 / 不管什么

**管**：`package.yaml` 契约与校验；`runtimes/` 包库的扫描事实；按**发起方目标**（本机 / guest 平台）选载荷；
选型诊断（缺包 / 多版本 / 定版 / 冲突 / 目标不匹配 / 无 bin）；`PackagePlan` 的装载顺序、本机 PATH 与只读根；按模块汇总的不可用能力。

**不管**：guest 生命周期、挂载/软链/system 叠写（`vm` 消费 `PackagePlan` 后自己做）；实际工具进程（`tools`）；
会话装配与工具面（`conductor`）；构建编排与补编（`builds`）。

## 二、边界

- `PackageSource` 只由 `service.rs` 持有（R12）；`detail/fs_packages.rs` 只做目录遍历与 yaml 解析。
- `domain/manifest.rs` 只认清单事实与目标载荷；`domain/plan.rs` 只做选型与诊断，不读盘、不起进程。
- 目标由调用方给：`Target::host()` / `Target::guest(platform)`；包不声明自己服务哪一档。
- `vm` 只经 `packages::api` 拿 `PackagePlan` / `PlannedPackage`，不再持有包库。
- 包库不预置、不分发；`files` 里的 sha256 用于可复现，不用于信任。

## 三、当前消费方

- `conductor`：本机档工具面（包 `bin/` 前置 PATH、包根只读放行）、运行能力报告、缺包判据与定版。
- `vm`：按 guest 平台筛库、取 `PackagePlan`，把 prefix 挂到 `/<prefix>`、system 载荷叠写进系统路径。
- `builds`：本机 / guest 构建期 `needs` 的包计划（创建会话前补编与 `--build` 共用）。
- `residents`：常驻服务起进程前的本机包 PATH 与只读根。
