# system 单元地图

> 定位：system（系统维护与自检）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-15｜依据：`src/capabilities/system/api.rs`

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/system.rs` | 能力入口：装配声明与文件头 |
| `src/capabilities/system/api.rs` | 入站契约：System trait、Fault（退出码 + 一句话）、各命令的请求形态、工具链事实的投影与提示文案、TOOLCHAIN_PROFILE |
| `src/capabilities/system/service.rs` | 用例实现：自检、存储报告、回收分派、补齐与还原的确认门槛、工作区回收与构建的前置校验 |
| `src/capabilities/system/detail.rs` | 机制入口：围栏能力事实 |
| `src/capabilities/system/detail/doctor.rs` | 自检报告的组装：平台 / 外部可执行 / 工具链 / 进程账 / 虚拟机档 / 存储概览 |
