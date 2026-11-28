# storage 单元地图

> 定位：storage（存储空间）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-28｜依据：`src/units/business/maintenance/storage/api.rs`

| 文件 | 职责 |
| --- | --- |
| `src/units/business/maintenance/storage.rs` | 子业务入口：装配声明与文件头 |
| `src/units/business/maintenance/storage/api.rs` | 入站契约：Storage trait、SpaceReport / GcReport / CleanRequest、分类 id（`JobState` 从 `exec_model` 复用）、NoStorage |
| `src/units/business/maintenance/storage/ports.rs` | 出站端口：盘点（SpaceSurvey → SpaceFacts）、回收（SpaceReclaim + ReclaimLease）、guest 存活（GuestLiveness） |
| `src/units/business/maintenance/storage/service.rs` | 用例实现：预算与策略、自动投递回收、显式清理分派、还原作业、作业进度 |
| `src/units/business/maintenance/storage/domain.rs` | 纯策略：按龄 / 上限 / 保护项排序；名字合法性 |
| `src/units/business/maintenance/storage/detail.rs` | 机制入口：盘点、回收、存活、路径、qcow2、磁盘余量、锁、状态 |
| `src/units/business/maintenance/storage/detail/survey.rs` | `DiskSurvey`：扫构建卷 / 下载缓存 / 可写层 / 日志（及 full 时的用户数据），产出 `SpaceFacts` |
| `src/units/business/maintenance/storage/detail/reclaim.rs` | `DiskReclaim`：回收锁、按分类删除、按模块排队、卷压缩、上次回收摘要 |
| `src/units/business/maintenance/storage/detail/guests.rs` | `ProcessGuests`：从进程台账判断有没有 guest 在跑 |
| `src/units/business/maintenance/storage/detail/paths.rs` | 可回收物的落点与占用判定（删除与扫描共用同一套路径） |
| `src/units/business/maintenance/storage/detail/scan.rs` | 目录大小、修改时间、目录项枚举 |
| `src/units/business/maintenance/storage/detail/qemu.rs` | qemu-img info / convert（可执行文件名按平台事实 `EXE_SUFFIX`） |
| `src/units/business/maintenance/storage/detail/space.rs` | 宿主磁盘总量 / 可用量（门面）：按平台选中 `src/units/business/maintenance/storage/detail/space/{unix,windows,other}.rs` |
| `src/units/business/maintenance/storage/detail/space/unix.rs` | `space` 的 Unix 后端：`statvfs` |
| `src/units/business/maintenance/storage/detail/space/windows.rs` | `space` 的 Windows 后端：`GetDiskFreeSpaceExW` |
| `src/units/business/maintenance/storage/detail/space/other.rs` | `space` 的兜底后端：如实返回 None |
| `src/units/business/maintenance/storage/detail/lock.rs` | 跨进程回收锁：非阻塞抢 `data/.vm/storage.lock`（内核文件锁，进程退出即释放） |
| `src/units/business/maintenance/storage/detail/state.rs` | `data/.vm/storage.json` 上次回收摘要 |
