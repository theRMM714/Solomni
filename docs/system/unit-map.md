# system 单元地图

> 定位：system（系统维护与自检）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-15｜依据：`src/capabilities/system/api.rs`

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/system.rs` | 能力入口：装配声明与文件头 |
| `src/capabilities/system/api.rs` | 入站契约：System trait、Fault（退出码 + 一句话）、各命令的请求形态、便携维护的 SetupMode / SetupOps 端口、工具链事实的投影与提示文案、TOOLCHAIN_PROFILE |
| `src/capabilities/system/service.rs` | 用例实现：自检、存储报告、回收分派、补齐与还原的确认门槛、工作区回收与构建的前置校验、便携维护转发 |
| `src/capabilities/system/detail.rs` | 机制入口：围栏能力事实、便携维护模块 |
| `src/capabilities/system/detail/doctor.rs` | 自检报告的组装：平台 / 外部可执行 / 工具链 / 进程账 / 虚拟机档 / 存储概览 |
| `src/capabilities/system/detail/setup.rs` | 便携维护的实现（`--setup`）：模式分派、只读状态视图（platform / root / path / fence）、写入类模式的进度行；`SetupHost` 实现 `SetupOps` 端口 |
| `src/capabilities/system/detail/setup/path.rs` | 用户 PATH 的读取、追加、摘除与还原（门面）：状态段与备份落点；平台实现在 `src/capabilities/system/detail/setup/path/windows.rs` 与 `src/capabilities/system/detail/setup/path/other.rs`，纯规则在 `src/capabilities/system/detail/setup/path/rules.rs` |
| `src/capabilities/system/detail/setup/path/windows.rs` | `path` 的 Windows 后端：写用户 PATH、备份原串、按备份还原，各给一句话结果 |
| `src/capabilities/system/detail/setup/path/other.rs` | `path` 的非 Windows 后端：不自动改 PATH，只给一句指引 |
| `src/capabilities/system/detail/setup/path/rules.rs` | PATH 分段规则与可移植的 unset / undo 逻辑（Windows 后端与跨平台单测共用） |
| `src/capabilities/system/detail/setup/shim.rs` | 便携入口模块门面：只有 Windows 有实现（`src/capabilities/system/detail/setup/shim/windows.rs`），其余平台为空模块 |
| `src/capabilities/system/detail/setup/shim/windows.rs` | 便携入口 `bin/solomni.cmd` 的生成与核对：固定 `--root <绝对根>`、参数透传、内容相同不重写 |
| `src/capabilities/system/detail/setup/fence.rs` | 围栏机器级预授：`fence` 模式的分步进度、成功后退役旧的全应用包机器级授权、状态缺项清单、首次授权后的自动预授；无 ACL 的平台由平台事实如实降级 |
