<!-- docs-template: spec -->
# IO：原语清单与唯一实现
> 定位：IO 面的唯一权威：哪些东西算 IO、各自只有哪一份实现、谁可以取。
> 效力：规范｜核对：2026-11-26｜依据：`src/kernel/api.rs`、`src/kernel/detail/io/`、`tests/gates/structure/io-owners.js`

## 适用与判据
- 新增副作用、新增网络或进程代码前读本文；违反由 `structure/io-owners`、`structure/env-surface`、`structure/capability-internal`、`structure/disk-owners` 判。
- 算 IO 的是**进程之外的系统面**：网络（出站与入站）、回环 TCP、进程环境、本机时间、长驻子进程、盘。判完还要再分一件事：**有 IO 的单元** ≠ **IO 单元**。

## 一、有 IO 的单元不是 IO 单元
- 业务单元的 IO 是**副作用**：副作用只能经 `kernel::api` 的原语发出去，业务单元只留自己的语义（argv、就绪判据、重试、错误文案）。
- 把「碰了 IO 的单元」收进一个 `src/units/io/` 会吞掉大半个仓库，变成 [boundaries.md](boundaries.md) 明令禁止的 DTO 中转站：上层每加一种用法都要在中转站加一个形状。
- 角色桶只收**同角色**的单元：`src/units/pure/`（环 1）与 `src/units/resource/`（盘主），见第三节。

## 二、原语清单
| 类别 | 唯一实现 | 取用面 | 直接调用的例外 |
| --- | --- | --- | --- |
| 出站网络（TLS 策略、代理预算） | `src/kernel/detail/io/net/backend.rs`（平台分叉在 `src/kernel/detail/io/net/backend/unix.rs` 与 `src/kernel/detail/io/net/backend/windows.rs`）、`src/kernel/detail/io/net/outbound.rs` | `kernel::api::net`：`agent` / `configure_reqwest` / `tls_backend` | `src/units/llm/detail/http_stream.rs` 自建 `reqwest::Client`（流式必须 async），TLS 仍经 `configure_reqwest` |
| 入站回环 HTTP | `src/kernel/detail/io/net/loopback.rs` | `kernel::api::net`：`serve` → `Loopback`（`port` / `url` / `stop`），handler 收 `Call` 回 `Answer` | 无：屏与单元都走这个面 |
| 回环 TCP | `src/kernel/detail/io/net/tcp.rs` | `kernel::api::net`：`reserve_port` / `await_loopback` | 内核中继 `src/kernel/detail/container/windows/relay.rs` 绑 `127.0.0.2`（同一条回环上的另一个地址），`dial` 不外露 |
| 进程环境 | `src/kernel/detail/io/env.rs` | `kernel::api::env`：`var` / `temp_dir` / `current_exe`（其余访问器只在核心里用） | `src/main.rs`、`src/entry/`、`src/presentation/`：启动参数与终端能力探测是原始输入，不收 |
| 本机时间 | `src/kernel/detail/clock.rs` | `kernel::api::clock`：`now_secs` / `now_nanos` / `local_stamp` | 无：`domain` 自己取时即门禁失败 |
| 长驻子进程 | `src/kernel/detail/procs/spawn.rs` 与 `src/kernel/detail/procs/daemon.rs` | `kernel::api::process`：`spawn`（`Spawn` → `Daemon`） | 一次性工具进程走 `ProcessRunner` 端口（围栏 + trace + 截断），不走 `spawn` |
| 盘 | `src/kernel/domain/layout.rs` 的访问器 + 主人单元 | `kernel::api::layout` 与主人单元的 `api` / 端口 | 主人表在 `tests/gates/structure/disk-owners.js`，一块盘一个主 |

- **TLS 只有一份决策**：ureq 与 reqwest 两条通道共用 `src/kernel/detail/io/net/backend.rs` 的平台判断；不合并传输——流式必须 async，统一的是策略与事实，不是传输栈。
- **长驻进程的界面**：`Spawn` 带 program / args / cwd / env / 三路流 / 尾行 / 回声 / purpose / owner / marker / 账本，`spawn` 一次办完进程组长、父死子亡、长命线程、stderr 常驻抽干与入账；`Daemon` 给 `try_wait` / `wait_exit` / `stderr_tail` / `take_stdin` / `take_stdout` / `stop`，丢掉即收掉。
- **入站服务只绑回环**：`serve` 固定 `127.0.0.1`，请求体有界，坏 body 由机制直接回 400；每请求一条线程，长轮询不挡别的请求。
- **时间分两种**：时刻（`kernel::api::clock`）与耗时（`Instant`，由调用方传进来）。

## 三、角色桶
| 桶 | 收哪些单元 | 判据 |
| --- | --- | --- |
| `src/units/pure/` | 环 1 的纯单元 | 环 1 与目录双向一致（`tests/gates/structure/layers.js` 的 `shapeProblems`） |
| `src/units/resource/` | 盘主单元：`home` / `assets` / `content` / `sessions` / `vmstore` | 盘面主人表与 `layout.rs` 双向比对（`tests/gates/structure/disk-owners.js`） |

- 桶编码**角色**，不编码环号：环只由 `layers.yaml` 决定，换环只改一行表，不引发全量改名。
- 认单元名一律走 `tests/gates/structure/unitpath.js`（`unitOfPath` / `unitOfModule` / `unitDir` / `unitPrefix`），禁止按路径位次假设。

## 四、明确不做
- 不合并 ureq / reqwest 两套传输；不实现 admission（并发、带宽、速率、端口准入），缺口见 `tests/gaps.yaml` 的 `io-admission-outbound` / `io-admission-process` / `io-admission-loopback`。
- 不收控制台输出（多为围栏诊断），不做随机数与 ID 端口，不把 qemu / smbd / virtiofsd 迁进内核（迁的是起停机制，不是 daemon 本体）。
- 不在没有重复实现的地方造抽象：代理环境变量由传输栈自己读，TLS 错误归类是 provider 侧词汇，两者都留在原地。

## 检查
- 唯一实现与调用点 → `structure/io-owners`（类别表在本门禁里，命中即失败）。
- 环境读取面 → `structure/env-surface`；单元内 IO 与 domain 取时 → `structure/capability-internal`。
- 盘面归属 → `structure/disk-owners`；内核不出现产品名 → `structure/kernel-products`。
- 依赖边与环 → `structure/layers` 与 `structure/dependency`，可读的地图见 [dependency-map.md](dependency-map.md)。

## 路由
- 端口与「IO 原语不画端口」：[ports.md](ports.md)
- 分层与角色桶口径：[layers.md](layers.md)
- 业务边界判据与硬要求：[boundaries.md](boundaries.md)
- 内核机制落点：[../kernel/README.md](../kernel/README.md)
- 单元之间的引用边：[dependency-map.md](dependency-map.md)
