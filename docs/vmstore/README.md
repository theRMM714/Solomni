# vmstore（虚拟机盘资源）

> 定位：`data/.vm` 这块盘的唯一主人——构建缓存、下载缓存、可写层、构建状态、虚拟根、便携引擎、安装台账、回收状态与回收锁的落点。
> 效力：规范｜核对：2026-11-25｜依据：`src/units/vmstore/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)。

## 管什么 / 不管什么

- 管：`.vm` 下每个子目录与文件名的拼法，以及路径段的安全化——同一个虚拟根在卷、锁、元数据上必须落到**同一个 stem**（`src/units/vmstore/domain/paths.rs` 的 `safe_segment`）。
- 不管：这些落点上发生的事——构建在 `builds`、依赖补齐在 `provision`、空间盘点在 `storage`、引擎与会话可写层在 `vm`。
- 不管：介质名 `.vm` 本身（唯一权威是 `src/kernel/domain/layout.rs`）；台账目录 `data/state` 的其它文件（归台账机制）。

## 入站契约与状态归属

- 入站面：`api` 只有 `paths(root) -> Paths`；`Paths` 的每个方法只做字符串派生，不碰磁盘。
- 出站端口：无——本单元不做 IO，因此没有 `ports.rs`、没有 `service.rs`（R3：纯逻辑不抽象）。
- 状态归属：**无内存状态**；`Paths` 只持一个产品根，派生结果每次重算。
- 路径不外泄：调用方本来就是持有产品根的 detail 适配器，不经过业务层——业务 service 只见 `GuestImage` / `BuildCache` 这类端口（R16）。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`（只取介质名 `layout::vm`）
- 谁在用我：`builds`、`provision`、`storage`、`vm` 的 detail，以及 `entry`（组合根装引擎时为可写层取目录）

## 一、布局

- 全部落点都由 `src/units/vmstore/domain/paths.rs` 派生；下表是它们的形状与用盘方。

| 落点 | 谁用 | 内容 |
| --- | --- | --- |
| `data/.vm/build-cache/<虚拟根>.qcow2` | `builds` | 该虚拟根的构建缓存卷 |
| `data/.vm/build-cache/<虚拟根>.json` | `builds` 写、`storage` 读 | 卷的模块占用元数据 |
| `data/.vm/build-cache/<虚拟根>.lock` | `builds` 持、`provision` 与 `storage` 读 | 同一虚拟根的构建锁 |
| `data/.vm/build-cache/<虚拟根>.purge` | `builds` | 按模块清理的排队标记 |
| `data/.vm/cache/` | `provision` | 下载缓存（cloud 镜像、deb、tar） |
| `data/.vm/overlays/` | `vm` | 会话可写层（每个租约一份覆盖层） |
| `data/.vm/build-state/<模块>.json` | `builds` | 构建状态（目标 + 源指纹） |
| `data/.vm/build-state/<模块>/home/` | `builds` | 那次构建的私有家目录 |
| `data/.vm/guest/<名>/` | `provision` 装、`vm` 起 | 一个虚拟根（磁盘 / 内核 / `guest.yaml`） |
| `data/.vm/qemu/bin/` | `provision` 装、`storage` 用 | 便携引擎工具（`qemu-img` 等） |
| `data/.vm/state/deps.json` | `provision` | 依赖安装台账 |
| `data/.vm/storage.json` | `storage` | 上次回收摘要 |
| `data/.vm/storage.lock` | `storage` | 回收的跨进程互斥 |

## 二、为什么只做路径所有权

- 这些落点的读写分散在四个单元的 detail 适配器里，各自都已经持有产品根；把 IO 也收进来会要求它们改构造签名并把整条调用链换成异步/回锁的新形态——那是**行为变更**，不是本阶段的目标。
- 本单元先拿走**唯一容易漂移的东西**：名字与形状。改名或加一层目录只改 `src/units/vmstore/domain/paths.rs` 一处；卷、锁、元数据共用同一个 `safe_segment`，不会再出现"构建按收过的名字建、盘点按原名找"这类对不上的路径。
- 存在性与锁的**语义**仍归用盘方：`storage` 的 `lock_held`（pid 活着或三小时内建的）与 `builds` 的等锁策略各自持有，因为它们的判据不同（盘点要保守，构建要等）。

## 三、待办（本阶段没做，别当成已完成）

- 两套锁（`builds` 的 `<虚拟根>.lock` 与 `storage` 的 `storage.lock`）**没有合一**：它们的用途不同（同一虚拟根的构建互斥 vs 全局回收互斥），合一属于并发行为变更，要单独拍板。

## 改动本单元时必须同步

- 落点改名 → [docs/architecture/state.md](../architecture/state.md) 的盘面契约、[storage/README.md](../storage/README.md) 的盘点点位、[builds/README.md](../builds/README.md) 的卷与锁、[provision/README.md](../provision/README.md) 的台账与下载缓存。
- 新增落点 → 先读 [boundaries.md](../architecture/boundaries.md) 的 R13（跨单元不许同形重复）：同一种形状只准有一处拼法。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
