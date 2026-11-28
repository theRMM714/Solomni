# 单元地图 · sandbox

> 定位：sandbox（执行环境策略）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-28｜依据：`src/units/business/sandbox/api.rs`

> 本文是 **sandbox** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/business/sandbox.rs` | 单元根：只写目的 / 管 / 不管 / 联动与 `api` / `domain` / `service` 的 mod 声明 |
| `src/units/business/sandbox/api.rs` | 入站面：重导出声明与事实、就绪判定（`Requirement` / `Readiness` / `Machine`）、执行端口（`ExecKind` / `ExecRequest` / `ExecOutcome` / `Sandbox` / `refused`）、客机词汇（`GuestArch` / `GuestOs`）、镜像契约与派生（`GuestImage` / `GuestClients` / `GuestAgent` / `classify_base` / `guest_platform` / `write_image` / `guest_image_of` / `guest_platform_of` / `guest_os_of` / `guest_runtimes_of` / `path_codec_of`）、策略契约 `SandboxPolicy` 与注册表 `SandboxPolicies`（其它单元与呈现层只准用这里） |
| `src/units/business/sandbox/domain.rs` | 纯定义的入口：声明 `decl` / `exec` / `facts` / `guest` / `image` / `policy` / `readiness` 七个概念子模块 |
| `src/units/business/sandbox/domain/decl.rs` | `SandboxDecl`（策略 id + 定版 + 不透明参数，手写 `Default` = 本机策略）与声明到选型的换算 `target(&facts)`；策略 id 常量 `HOST_POLICY` / `VM_POLICY` |
| `src/units/business/sandbox/domain/facts.rs` | `SandboxFacts`（策略 id / 哪一侧 / 客机名与系统架构）与策略自我介绍 `PolicyDecl`（id / 名字 / 说明 / 参数声明） |
| `src/units/business/sandbox/domain/policy.rs` | 策略契约 `SandboxPolicy`：`decl` / `check` / `facts` / `open`（开一席的执行句柄）四个必答，加 `machine` / `readiness` / `provided` / `refusal` / `machine_refusal` 五个带默认的判据（措辞与缺什么留在实现里），`OpenRequest` 带会话、租约、声明、事实与运行期路径；组合根把实现登记进注册表 |
| `src/units/business/sandbox/domain/readiness.rs` | 就绪判定词汇：`Requirement`（一项前置：id / 成不成立 / 这次的细节 / 怎么补）、`Readiness`（一次判定的清单，`ready` / `unmet` / `missing`）、`Machine`（与某条会话无关的机器能力，`absent` 给未登记的策略一条如实的不满足项） |
| `src/units/business/sandbox/domain/exec.rs` | 执行端口：`ExecKind`（`Shell` / `Module`）、`ExecRequest`（cwd、围栏、超时、可问的前端、取消旗、回执预算）、`ExecOutcome`（回执 + 围栏准备耗时）、`trait Sandbox`（`precheck` 自检 + `run` 真跑）；`Refusing` / `refused(why)` 是「执行面开不出来就拒绝一切」的兜底句柄 |
| `src/units/business/sandbox/domain/image.rs` | 镜像契约与派生：`GuestImage`（磁盘 + 可选内核/initrd + agent + 共享客户端 + 自带运行能力）、`GuestClients`（`debian()` / `names()`）、`classify_base`（按磁盘与可选 `guest.yaml` 判一份虚拟根可不可用）、画像读写 `write_image`、平台名 `guest_platform`，以及按选型取镜像的派生 `guest_image_of` / `guest_platform_of` / `guest_os_of` / `guest_runtimes_of` / `path_codec_of`；`vm` 起客机前读它，`provision` 装完写它，`packages` 按它选运行包 |
| `src/units/business/sandbox/domain/guest.rs` | 客机词汇 `GuestArch`（`Amd64` / `Arm64`）与 `GuestOs`（`Linux` / `Macos` / `Windows`，带路径方言与 `parse`）；本体从 `vm` 搬来，两侧共用这一份 |
| `src/units/business/sandbox/service.rs` | 注册表 `SandboxPolicies`：按 id 取策略、`check` / `facts` / `open` 统一出口，以及 `machine` / `readiness` / `provided` / `refusal` / `machine_refusal` 五个判据的统一入口（调用方不按策略 id 分支）、缺省回落本机、未登记如实报错 |
