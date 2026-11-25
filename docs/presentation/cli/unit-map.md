# 单元地图 · presentation/cli

> 定位：presentation/cli（终端呈现）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-25｜依据：`src/presentation/cli.rs`

> 本文是 **presentation/cli** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../../architecture/layers.md)，业务边界判据见 [boundaries.md](../../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/presentation/cli.rs` | 终端转录中心入口：启动页 → 提示符（前缀定归属：`/` 开头交 `shell`，其余开或接核心代理会话）→ 会话流；启动目录收成外部工作区（`startup_workspace` → `WorkOps::ambient_workspace`，`--no-workspace` 关闭）；启动相命令在提示符里也走这里（`run_boot`：拒 `--root`、忽略 `--json`、就地确认、有会话在跑时只预览）；不做业务判断、不持状态 |
| `src/presentation/cli/commands.rs` | 一次性命令：`diagnose` / `secret` / `resident` / `work` / `rewind`；取参与打印回执（参数已由声明解析成 JSON） |
| `src/presentation/cli/shell.rs` | 词表先落到声明上：命令与模块工具走同一条入口（`--json` / `--help` / `--version` 摘取、模块清单与三级手册 `modules` / `<模块>` / `<模块> <工具>`）；命令行一次性调用与转录中心提示符共用 |
| `src/presentation/cli/view.rs` | 渲染原语：启动页事实行、状态行、花名册、菜单（取自命令目录）；交互模式富渲染（角色行、工具行、验收/交付、上下文用量），非 TTY 保持原样文本 |
| `src/presentation/cli/catalog.rs` | 命令手册的排版：把 `systools/commands.yaml`（经 `OpBook` 的 `Op`）投影成手册、实时菜单（`/` 前缀）、Tab 补全与未知命令建议（两相都算，启动相给 `--名字`，写法由参数声明派生）；不持有一份自己的命令清单 |
| `src/presentation/cli/boot.rs` | 启动相命令的执行：把操作数与命令名翻成系统维护用例的一次调用，排版回执与退出码，写操作就地确认（`Ask`：命令行用 `NeverAsk`、转录中心用终端问答）；便携维护 `setup` 的模式名在这里翻成 `SetupMode`；不解析命令表、不做业务判断 |
| `src/presentation/cli/argv.rs` | 全局旗语的唯一解析器（含 `--no-anim` / `--no-workspace`）：`--x v` 与 `--x=v` 等价、内部协议旗语原样透传、未知旗语给编辑距离建议 |
| `src/presentation/cli/gate.rs` | 交互回合：跟读事件流、裁决卡问答、等安静与动作回显；建会话动作的入参收成 `SessionAsk`（名字 / 流程 / 席位 / 任务 / 档位 / 工作区），绑定与校验都在后端 |
| `src/presentation/cli/flows.rs` | 会话流：单 agent / 代理 / 协作 / 供应商 / 模型 / 核心设置；建会话时把启动目录工作区交给后端，并提示工作区路径或没绑的原因 |
| `src/presentation/cli/term.rs` | 终端 I/O：TTY 判定、raw mode 开关、启动页字标、固定输入框的绘制与擦除、事件到按键的翻译、非交互回退、`TERM=dumb` 的 ASCII 回退与 `--no-anim` 静态帧 |
| `src/presentation/cli/input.rs` | 输入框的行编辑：缓冲区、光标、历史；按键语义到编辑动作的映射 |
| `src/presentation/cli/theme.rs` | 档位判定（真彩 / 256 / 16 / 无色）与冰青语义 token 上色 |
| `src/presentation/cli/width.rs` | 显示宽度：东亚宽字符 2 格、按列截断不劈开宽字符 |
| `src/presentation/cli/view/tests.rs` | 终端渲染的用例与替身：startup/roster/menu/富与纯转录/工具行的逐行断言（`#[cfg(test)] mod tests;` 指向） |
