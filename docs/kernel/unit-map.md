# 单元地图 · kernel

> 本文是 **kernel** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

> **无领域语义、无领域状态**的机制；形状与别的业务一致（`api` / `ports` / `domain` / `detail`），但它在依赖图的最底层：**不认识任何能力**。
> `Log` / `HostProbe` / `ToolHandler` / `AskUser` / `ProcessRunner` / `SessionHost` / `ProcessTexts` / `FenceHost` 是**全项目共享**的机制端口（R12 的例外）。

| 文件 | 职责 |
| --- | --- |
| `src/kernel/mod.rs` | 机制型业务的入口与公开面：`api` / `ports` / `domain` / `detail` 四个子模块的形状声明 |
| `src/kernel/api.rs` | 机制型业务的对外面：共享事实（`SessionId` / `Tier` / `ExecSpec` / `ToolOutcome` / `Ask` / 默认预算）与纯机制（`slash` 的对外写法、生成中作业的取消表 `JobRegistry`）的重导出 |
| `src/kernel/ports.rs` | 全项目共享的机制端口：`Log` / `PathCodec` / `ProcessTexts` / `AskUser` / `ToolHandler` / `HostProbe` / `ProcessRunner` / `SessionHost` / `FenceHost` 的 trait 定义与它们的纯数据形态 |
| `src/kernel/domain/types.rs` | 跨业务共享的事实类型（纯数据）：`SessionId` / `Tier` / `ExecSpec` / `ToolOutcome` / `Ask` / `AskOutcome` 与默认预算、能力名文法 |
| `src/kernel/domain/fence.rs` | 围栏描述符（`FenceSpec`）与落点判据（`FencePart`/`FenceBlocked`/`fence_ask`）：纯数据 |
| `src/kernel/domain/path.rs` | 路径的对外书写形式：把路径统一写成 `/` 分隔的字符串（给提示词、回执与 JSON 用；纯逻辑） |
| `src/kernel/domain/jobs.rs` | 生成中作业的取消表（`JobRegistry`）：同一会话每次生成各持一面旗，停止置位全部、摘旗按身份 |
| `src/kernel/detail/file_log.rs` | `Log` 端口的文件实现：每次运行在 `logs/` 下按时间戳建文件并逐行追加（写经 `Mutex` 串行） |
| `src/kernel/detail/host_probe.rs` | `HostProbe` 端口的机制实现：查环境变量与 `PATH` 里的可执行文件、路径存在性，并按平台判定虚拟机能力 |
| `src/kernel/detail/outbound.rs` | 出站 HTTPS 的共享机制：唯一一处 TLS 后端装配与通用 ureq 代理 |
| `src/kernel/detail/unpack.rs` | 归档解包机制：tar / tar.gz / tar.zst / tar.xz / zip 解到目录，支持 strip 与宽容链接，并还原归档里的权限位（可执行位） |
| `src/kernel/detail/process.rs` | 外部进程执行机制（守门进程、stdin 送参、隐秘字段 env 注入、超时杀树、截断），实现 `ProcessRunner` |
| `src/kernel/detail/procs.rs` | 长驻子进程的账（`.home/processes.json`）与回收：父死子亡（Linux 内核信号 / macOS 守门进程 / Windows 作业对象）、存活探测、终止、启动对账；当前登记虚拟机档的 QEMU 与共享 helper |
| `src/kernel/detail/session.rs` | 长驻外部进程机制（守门进程长跑、按行收发、关闭杀树、父死子亡、Windows 容器预授权），实现 `SessionHost`；常驻服务的守门进程经它拉起 |
| `src/kernel/detail/confine/mod.rs` | 围栏机制入口（策略由调用方传入；实现 `FenceHost` 释放） |
| `src/kernel/detail/confine/linux.rs` | Linux 围栏后端：用 Landlock 把可达范围变成内核强制（`spec.rw` 读写、运行基线只读、其余拒绝），机制不可用时如实降级 |
| `src/kernel/detail/confine/macos.rs` | macOS 围栏后端：生成 seatbelt profile（默认拒绝，再逐条放行运行基线与 `spec.rw`）交 `sandbox_init` 装上，装不上如实降级 |
| `src/kernel/detail/confine/other.rs` | 其他平台后端：如实降级——只保证超时与整棵进程树终止，绝不作有文件系统围栏的假装 |
| `src/kernel/detail/confine/windows/` | Windows 围栏后端目录：入口与策略派发在 `mod.rs`，DACL 授权在 `acl.rs`、授权台账与撤权在 `record.rs`、容器身份与作业对象在 `container.rs`、真机探针在 `tests.rs` |
| `src/kernel/detail/confine/windows/mod.rs` | Windows 围栏后端入口：按 agent 派生 AppContainer SID、逐条授权可达范围，再用 `STARTUPINFOEX` 在容器与作业对象里起工具（不给 capability = 断网） |
| `src/kernel/detail/confine/windows/acl.rs` | Windows 安全描述符操作：逐路径改 DACL（授予 / 撤销）、解析 ACE 与 SID、展开泛型掩码 |
| `src/kernel/detail/confine/windows/record.rs` | 围栏授权台账与撤销：快照、根外授权摘要与容器 profile 的落盘、精确撤销、孤儿清扫与按条处置 |
| `src/kernel/detail/confine/windows/container.rs` | Windows 容器围栏的启动面：AppContainer 档案与 SID、UTF-16 转换、system shell 与 kill-on-close 作业对象 |
| `src/kernel/detail/confine/windows/tests.rs` | Windows 围栏的真机探针：AppContainer 授权、容器往返、ACL 台账与撤权（需改本机权限项的用例，`--fence-live` 才跑） |
