# storage 单元地图

> 定位：storage（存储空间）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-04｜依据：`src/capabilities/storage/api.rs`

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/storage/mod.rs` | 能力入口：装配声明与文件头 |
| `src/capabilities/storage/api.rs` | 入站契约：Storage trait、SpaceReport / GcReport / CleanRequest / JobState、NoStorage |
| `src/capabilities/storage/service.rs` | 用例实现：扫描编排、预算、GC、显式清理、还原作业、策略与锁 |
| `src/capabilities/storage/domain/mod.rs` | 纯策略：按龄 / 上限 / 保护项排序 |
| `src/capabilities/storage/detail/mod.rs` | 机制入口：扫描、qcow2、磁盘余量、锁、状态 |
| `src/capabilities/storage/detail/scan.rs` | 目录大小、修改时间、目录项枚举 |
| `src/capabilities/storage/detail/qemu.rs` | qemu-img info / convert |
| `src/capabilities/storage/detail/space.rs` | 宿主磁盘总量 / 可用量 |
| `src/capabilities/storage/detail/lock.rs` | 跨进程回收锁 |
| `src/capabilities/storage/detail/state.rs` | `.vm/storage.json` 上次回收摘要 |
