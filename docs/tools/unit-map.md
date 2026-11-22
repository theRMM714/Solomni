# 单元地图 · tools

> 定位：tools（工具与围栏）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-04｜依据：`src/capabilities/tools/api.rs`

> 本文是 **tools** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/capabilities/tools/api.rs` | 入站能力面：`Tools`（按角色发放工具面、总表与保留名）、`ToolExec`（执行模块 / 内置 / shell 工具、释放围栏授权）与 `CommandBook`（产品命令面）的用例面与公开类型重导出；`SessionWriteGate` 把批前观察事实与 `work_gate_info` 合成为 `WriteGate` 判定（未受管放行、未观察 / 过期 / 非全文拒绝；批量 `check_many` 只取一次工作区快照，拒绝记 `warn` 审计） |
| `src/capabilities/tools/service.rs` | 工具能力的用例与端口持有者：持工具总表、角色表与名字表（`SystemTools`）及 `SysIo` / `ProcessRunner` / `FenceHost` / `FenceGate` 四个端口 |
| `src/capabilities/tools/service/fence.rs` | 围栏裁决（`ToolFenceGate`）：必要落点授不上时问用户、按裁决无围栏跑一次或拒绝；回执文案与停会话在这里 |
| `src/capabilities/tools/ports.rs` | 出站端口：三张表的加载 `SystoolsSource` 与内置文件工具的读写 `SysIo`（含整份读 `FileRead`、按行窗口读 `FilePage` / `read_page`、`DirEntry`、目录遍历 `walk` 与 `WalkOpts` / `WalkEntry`、流式按行扫 `scan_lines` 与 `ScanSummary`、配置类小文件 `read_small`、写盘的 `Expect` / `Mutation` / `WriteReceipt` / `WriteFault` 数据形态） |
| `src/capabilities/tools/service/grep.rs` | 内置 `grep` 的匹配与输出编排：模式编译（正则 / 字面、忽略大小写）、glob 过滤、目录遍历后的目标收敛（含逐项读权限）、逐文件流式扫描与上下文拼行、命中 / 文件 / 二进制 / 无读权限计数、回执与截断说明 |
| `src/capabilities/tools/service/systool.rs` | 内置工具的执行编排：解析入参 → 按声明校验 → 沙箱寻址 → 先读后改门禁（patch 按整批一次快照）→ 经 `SysIo::mutate` / `transact` 落盘（锁内版本 CAS、原子替换、跨文件回滚）→ 产出观察事实（规则在 `src/capabilities/tools/domain/systool.rs` 与 `src/capabilities/tools/domain/patch.rs`）；`GREP` 分发到 `src/capabilities/tools/service/grep.rs` |
| `src/capabilities/tools/domain/budget.rs` | 工具结果的**唯一预算**：单条静态上限 16k 字符（head+tail、按完整行取整、单行超长按字符切），再按本批剩余上下文给每条上限（`min(16k, 剩余 token × 2 / 本批调用数)`，reserve 见 `ResultBudget::for_batch`）；低于 1k 字符判枯竭、整批预检失败，未枯竭时生产者先截、会话层同预算兜底并允许二次收缩；截断加 `result.truncated` / `result.truncated.context` 与按工具续读提示（文件类给 read 的 offset/limit，执行类给缩小输出 / 写文件再读）；read 的续读 offset、shell 的状态行 / 末行在尾部保留 |
| `src/capabilities/tools/domain/systool.rs` | 内置文件工具与回报（read / write / edit / patch / delete / list / grep / submit_report）的纯规则：参数校验、观察账本、回执文案与 `ToolOutcome`；系统工具保留名（含 shell） |
| `src/capabilities/tools/domain/patch.rs` | 补丁通道（patch 工具）的纯逻辑：解析自由格式的改动文本并在文件内容上按整行应用 |
| `src/capabilities/tools/domain/schema.rs` | 工具参数契约：解析、校验与渲染声明在文本层的参数 schema（内置工具在 `systools/tools.yaml`，模块工具在 `module.yaml`） |
| `src/capabilities/tools/domain/module_tools.rs` | 模块清单 → 工具面：把 `module.yaml` 声明的工具与参数契约转成工具能力认识的形态，并把静态模块工具与已就绪服务操作的参数合并渲染成**一段** |
| `src/capabilities/tools/domain/commands.rs` | 产品命令面的纯数据形态（`Command` / `CommandBook` / `Phase`）：读 `systools/commands.yaml`、做骨架校验并投影成 `Op`；只管声明，不管执行 |
| `src/capabilities/tools/domain/op.rs` | 命令与动作的统一形状 `Op`（分组、相位、执行者、确认门槛、参数 schema、回执）与文本入口的全部派生：`parse` 把词法转成参数 JSON、`usage` 出写法、`complete` 出补全、`problems` 判自洽；一份声明供 CLI 手册、TUI 命令、Web 表单与模型工具声明共用 |
| `src/capabilities/tools/domain/roles.rs` | 角色表与工具总表的纯数据形态（`RoleDecl` / `SystemTools`）：按角色组装工具面、装配期自检与动作审计格式化 |
| `src/capabilities/tools/domain/fence.rs` | 围栏描述符的派生策略与裁决卡材料（`fence_ask` / 选项 id；描述符与落点判据在 `src/kernel/contracts/fence.rs`） |
| `src/capabilities/tools/domain/shell.rs` | 执行类系统工具 shell 的纯规则：参数 / timeout 解析、命令长度、共享区执行位提示、回执解析（退出码 / 状态 / cwd·耗时） |
| `src/capabilities/tools/detail/sys_io.rs` | `SysIo` 端口的磁盘实现：纯 Rust 直接读写（严格 UTF-8、非法字节如实标注、写时建父目录）；按行窗口读流式扫完整份算行数与哈希；目录遍历走 `ignore::WalkBuilder`（不跟链接、按路径排序、规则剪枝），流式按行扫用 `BufReader` 且不受整份读上限约束；写盘在跨进程路径锁内做版本 CAS（锁目录由组合根注入，缺省退回进程内锁），临时文件 + 原子替换发布，`transact` 先校验/暂存再发布并在失败时回滚已发布文件 |
| `src/capabilities/tools/detail/yaml_systools.rs` | `SystoolsSource` 的磁盘实现：读 `systools/tools.yaml`、`roles.yaml` 与 `names.yaml`（缺文件如实报装配错误） |
