# storage（存储空间）

> 可回收缓存的盘点、预算、自动/手动回收与显式清理；虚拟根还原入口。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；设置字段见 [REGISTRY_SPEC.md](../../REGISTRY_SPEC.md)。

## 一、管什么 / 不管什么

**管**：构建缓存卷、依赖下载缓存、过期可写层、日志的体积与回收；策略（预算 / 保留天数 / 单卷大小 / 最低剩余）；
构建前 headroom 检查；虚拟根“还原到初始状态”的作业入口与进度；空间报告（含虚拟根、会话、模块产物的大小，只读）。

**不管**：构建怎么跑（`builds`）、guest 怎么起（`vm`）、下载怎么装（`provision`）、工作对象可达性（`workspace` 的 `work_gc`）。
工作对象回收保持现状（`solomni --work-gc`），不进本能力——可达性判定、工作锁与事务恢复属于 workspace。

## 二、策略与语义

`settings.yaml` 的 `storage` 段：

| 字段 | 默认 | 语义 |
| --- | ---: | --- |
| `periodic_gc` | true | 启用全局周期回收；构建前 headroom 始终启用，不受它控制 |
| `gc_interval_minutes` | 15 | 全局自动回收间隔（分钟，至少 15；改设置最多 30 秒生效） |
| `cache_cap_gib` | 20 | 可回收缓存总预算（GiB）；0 = 按磁盘 10%，夹 10–50 GiB |
| `cache_keep_days` | 30 | 超过这么久未使用的构建卷/下载缓存可回收；0 = 不按龄 |
| `build_volume_gib` | 16 | 新构建缓存卷的虚拟大小；现有卷不自动缩 |
| `min_free_gib` | 2 | 宿主剩余低于它先回收；回收后仍低才拒绝构建 |
| `log_keep_days` | 14 | 日志保留天数；另保留最近 50 份 |

- 上限只约束可回收缓存；会话、虚拟根、模块产物只展示，不自动删。
- 全局回收由后台线程按 `gc_interval_minutes` 触发；默认且最小 15 分钟，关闭 `periodic_gc` 只停周期回收。
- 构建前 `ensure_headroom` 是安全闸，始终启用；只有超预算或宿主余量低时才动。
- 到上限先回收一次；回收后仍超限但磁盘不紧，继续构建并如实报告；只有宿主剩余低于 `min_free_gib` 且回收后仍低才拒绝。
- 自动候选顺序：按龄的日志 / 过期可写层 / 下载缓存 / 构建卷；超上限再按最近使用补足；当前构建目标卷排最后。
- 构建前 `ensure_headroom` 由 `builds` 调用；并发回收靠 `.vm/storage.lock`，构建卷锁仍由 builds 持有。

## 三、显式动作

| 动作 | 语义 |
| --- | --- |
| 清理整卷 | 删 `.vm/build-cache/<虚拟根>.qcow2`、元数据与锁（有构建锁则拒绝） |
| 按模块清理 | 写 `.vm/build-cache/<虚拟根>.purge`；下次该虚拟根构建挂载时删除该模块工作区并 trim |
| 压缩 | 未被构建占用时 `qemu-img convert` 回写，失败保留原卷 |
| 清空下载缓存 | 删 `.vm/cache` 内容；补齐作业运行中拒绝；重装需重新下载 |
| 清理过期可写层 | 没有活跃 guest 时删 `.vm/overlays/*.qcow2` |
| 清理日志 | 按 `log_keep_days` 删旧日志 |
| 还原虚拟根 | 只允许产品清单构造的虚拟根；先删 `.vm/guest/<名>`，再按清单重建；**不备份**，Web 红字 + 二次确认，CLI `--yes` 二次确认；成功后清该虚拟根构建缓存 |

## 四、入口

- Web：设置 → 存储空间（`GET /api/storage`、`POST /api/storage/gc`、`POST /api/storage/clean`）。
- CLI：`solomni --storage`（JSON）、`solomni --gc [--yes] [--category=…] [--guest=…] [--module=…]`、`solomni --provision --reset=<虚拟根> --yes`。
- `--doctor` 的 `storage` 段只读汇总，不触发回收。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
