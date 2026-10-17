# 单元地图 · tools

> 本文是 **tools** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/tools/api.rs` | 入站能力面：`Tools`（按角色发放工具面、总表与保留名）与 `ToolExec`（执行模块 / 内置工具、释放围栏授权）的用例面与公开类型重导出 |
| `src/capabilities/tools/service/mod.rs` | 工具能力的用例与端口持有者：持工具总表与角色表（`SystemTools`）及 `SysIo` / `ProcessRunner` / `FenceHost` 三个端口 |
| `src/capabilities/tools/ports.rs` | 出站端口：两张表的加载 `SystoolsSource` 与内置文件工具的读写 `SysIo`（含 `FileRead` / `DirEntry` 数据形态） |
| `src/capabilities/tools/service/systool.rs` | 内置工具的执行编排：解析入参 → 按声明校验 → 沙箱寻址 → 经 `SysIo` 读写盘（规则在 `domain/systool.rs`） |
| `src/capabilities/tools/domain/systool.rs` | 内置工具（read / write / edit / patch / list / search / submit_report）的纯规则：参数校验、观察账本、回执文案与 `ToolOutcome` |
| `src/capabilities/tools/domain/patch.rs` | 补丁通道（patch 工具）的纯逻辑：解析自由格式的改动文本并在文件内容上按整行应用 |
| `src/capabilities/tools/domain/schema.rs` | 工具参数契约：解析、校验与渲染声明在文本层的参数 schema（内置工具在 `systools/tools.yaml`，模块工具在 `module.yaml`） |
| `src/capabilities/tools/domain/module_tools.rs` | 模块清单 → 工具面：把 `module.yaml` 声明的工具与参数契约转成工具能力认识的形态，并渲染模型侧参数段 |
| `src/capabilities/tools/domain/roles.rs` | 角色表与工具总表的纯数据形态（`RoleDecl` / `SystemTools`）：按角色组装工具面、装配期自检与动作审计格式化 |
| `src/capabilities/tools/domain/fence.rs` | 围栏描述符的派生策略（其余中立类型在 `kernel/domain/fence.rs`） |
| `src/capabilities/tools/detail/prompt_process_texts.rs` | 提示词册 → kernel `ProcessTexts` 端口的适配器 |
| `src/capabilities/tools/detail/sys_io.rs` | `SysIo` 端口的磁盘实现：纯 Rust 直接读写（严格 UTF-8、非法字节如实标注、写时建父目录） |
| `src/capabilities/tools/detail/yaml_systools.rs` | `SystoolsSource` 的磁盘实现：读 `systools/tools.yaml`、`roles.yaml` 与 `names.yaml`（缺文件如实报装配错误） |
