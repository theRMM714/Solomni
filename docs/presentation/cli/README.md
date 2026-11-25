# cli（终端转录中心）

> 定位：argv → 入站面 → 渲染事件流。
> 效力：规范｜核对：2026-11-25｜依据：`src/presentation/cli.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../../architecture/layers.md)，业务边界判据见 [boundaries.md](../../architecture/boundaries.md)，测试规范见 [TESTING.md](../../../TESTING.md)。

## 管什么 / 不管什么

- 管：终端转录中心——argv → 入站面 → 渲染事件流；`split_names` 与空登记处的引导文案是它自己的传输侧的事。
- 管：一行输入的归属看前缀（`src/presentation/cli.rs`）——`/` 开头是命令（`/help` 看手册、`/exit` 退出、`/web` 转界面，参数补全只在这里生效；主提示里输入 `/` 时输入框下方实时列出匹配的命令与一句说明，↑↓ 选行、Tab 补全选中项、Esc 先收菜单、Enter 直接发送，菜单尺寸随匹配条数变化），`//` 转义成以字面斜杠开头的一句消息，空行不发任何东西，连按两次 Ctrl+C 离开提示符（会话不停），其余整行发给核心代理：顶层还没有会话时先开一条核心代理会话，把它当第一句。命令在命令行上的同一份入口见 `src/presentation/cli/shell.rs`。
- 管：Ctrl+C 按场景分流——`src/presentation/cli/term.rs` 的读行分「读到一行 / 输入到尽头 / 按了 Ctrl+C」三态，`src/presentation/signal.rs` 装进程级处理器：会话里连按两次才离开提示符（会话照跑）；一问一答（确认、裁决卡、附言）按一下 = 这一问不答了（裁决卡等同空行「先不答」，卡留着）；转录中心按一下退出产品；agent 干活时没人读键盘，第一次 Ctrl+C 由处理器接住、停掉这一轮（撤回答 + 作废等待中的裁决，不停会话），没人取走再按一次才放行默认终止。
- 管：启动目录自动成外部工作区——从哪个目录启动，默认就把那个目录当这次会话的外部工作区（`src/presentation/cli.rs` 的 `startup_workspace` 经 `WorkOps::ambient_workspace` 走 `ambient` 阶梯：档位策略、可达、是目录、不与产品根互含，不探可写），建会话后第一行提示工作区路径；判定不过（启动目录就是产品根 / 不可达 / 不是目录）就回退产品内 `work/` 并打印原因，`--no-workspace` 关掉这次启动的自动绑定。
- 管：终端呈现本身——TTY 与档位判定、raw mode 的开关与恢复、冰青字标启动页、固定输入框（行编辑、历史、宽字符宽度）与状态行。交互模式只画这些；非 TTY（管道）保持逐行输入与原样输出，脚本口径不变。
- 管：五档降级——真彩 / 256 / 16 色档自动取色，`NO_COLOR` 或 `--plain` 无色，`TERM=dumb` 把画框与状态字形退回 ASCII（`+-|` 与 `v/x/*/>`），`--no-anim` 让生成态停在静态帧；启动页如实打印当前档位。
- 管：交互模式的转录渲染——角色行与正文缩进、工具行（标签 + 参数摘要 + 成败）、验收清单与交付结论、裁决卡边框、生成态 spinner 与耗时、上下文用量写回状态行；同一批 `SessionEvent` 在非 TTY 下仍走原样文本。
- 管：共享区提交历史与回滚入口——`work <会话>` 看 head + 最近提交，`work <会话> log [条数]` 列提交，`work <会话> show <提交号>` 看详情，`work <会话> restore <提交号> [--yes]` 走统一的 `restore_work_commit` 动作（非破坏性；交互模式要求输入 yes，`--yes` 仅用于显式脚本）。
- 管：虚拟机档会话的只读诊断入口——`diagnose <会话>` 造一次 `guest_diagnose` 动作（`caller = user`），把动作返回的结构化 JSON（guest 进程表 / CPU 汇总 / 内存 / 负载 / 挂载）原样打印；与 `work` / `resident` / 模块工具同类，不做业务判断。
- 管：命令与模块工具同一条入口（`src/presentation/cli/shell.rs`）——命令行一次性调用与转录中心提示符都先把词表落到声明上（`Op` 的 `parse`），再按执行者交给业务面；声明里的 `runtime` 决定这次调用要不要常驻运行时（`none` 的一次性命令不占视图盘、不起存储自动任务、不拉常驻服务、不吃启动门的围栏授权；`services` 才进会话 / 服务 / 模块工具），`modules` 是模块清单（版本、启停、拒收原因），`<模块>` 是模块手册（版本、状态、`system` 原文、工具表），`<模块> <工具> --help` 是工具手册，`<模块> --version` 是模块版本（工具没有自己的版本）。
- 管：启动相命令的最后一公里——`--doctor` / `--provision` / `--gc` / `--storage` / `--work-gc` / `--build` / `--setup` 的操作数与回执排版（`src/presentation/cli/boot.rs`）；执行与确认门槛在 `system` 单元（见 [docs/system](../../system/README.md)）。`--setup` 的写入成果（`bin` shim、用户 PATH、围栏机器级预授）要新开终端才生效。
- 管：启动相命令的**第二个入口**——同一条命令在终端转录中心的提示符里也能跑（`--doctor`、`--gc<空格>--category=logs`），跑完回提示符、不退出进程；缺 `--yes` 时就地一问（先给预览再问），有会话在跑时写操作只给预览、不执行；`--json` 与 `--root` 在这里没有意义（前者提示后忽略，后者报错、退出码 2）——歧义判据见 `src/presentation/cli.rs` 的 `run_boot`。
- 管：机器回执——`--json` 让命令只输出一段 JSON。启动相命令的字段逐命令写在 `systools/commands.yaml` 的 `json`（`solomni <命令> --help` 里给出同一份说明），失败时 stderr 是一行 `{"error","code","domain"}`、退出码不变；转录相命令（`modules`、模块手册与工具、`work` …）按声明输出结构化 JSON。**启动期的诊断（工具链 / 围栏 / 进程 / 便携 / 沙箱授权）一律走 stderr**，stdout 只留命令结果，所以 `solomni --json …` 可以整段进管道与 `JSON.parse`。
- 不管：不做业务判断、不持状态、不落盘；**永不接触端口对象，也拿不到 `Core` 本身**（`src/presentation/cli.rs`）；启动相命令的语义也不在这里——`boot.rs` 只把参数翻成单元调用。

## 入站契约与状态归属

- 入站面：只依赖**入站面**（各单元的 `::api`）；`Faces`（`src/presentation/faces.rs`）是它与核心之间的唯一通道。
- 入口：`src/main.rs` 调 `presentation::cli::run(faces, presentation::web::DEFAULT_PORT, plain, anim, startup, system)`（`startup` 是组合根算好一次的启动目录工作区：`Startup { workspace, skip }`）；`web [端口]` 子命令把控制权交给 Web 渠道（同一份 `Faces`）。`system` 由组合根注入，与启动相共用同一份 provision / storage 端口——两套实例会互相看不见对方的清理作业。
- 旗语：`--root <dir>`、`--web`、`--port <n>`、`--plain`、`--no-anim`、`--no-workspace`、`--json`、`-h/--help`、`-v/--version` 由 `argv.rs` 统一解析（`--x v` 与 `--x=v` 等价；未知旗语给编辑距离建议）；全局旗语与启动命令的先后无关（`--doctor --root <dir>` 与 `--root <dir> --doctor` 等价），命令自己的参数按原顺序留在操作数里交给声明解析器；命令手册、写法、Tab 补全、未知命令建议与参数校验都由 `systools/commands.yaml` 这一张表派生——表经 tools 能力的 `CommandBook` 编译进二进制（`include_str!`，无产品根也能 `--help`），`src/units/tools/domain/op.rs` 做解析与派生，`catalog.rs` 只做排版；补全与建议覆盖两相（启动相给 `--名字` 写法，与提示符里的写法一致）。新增或改命令 = 改这张表（名字、别名、分组、`phase`、执行者、`runtime`、一句话说明、确认门槛、参数声明）；分派按执行者：`system` 在 `src/presentation/cli/boot.rs`（启动相，装配核心之前），`core` / `ui` 在 `src/presentation/cli/shell.rs`（装配之后，命令行与提示符共用）；声明自洽由 `src/main.rs` 的 `command_ops()` 在装配期硬失败。
- 状态归属：不持状态、不落盘；会话与业务状态都在核心，本层只读 `Faces` 回报。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`runtime`、`work`、`turn`、`modules`（经 `Faces` 字段）、`registry`、`system`（启动相命令）、`vm`
- 谁在用我：（无）

## 改动本单元时必须同步

- 入站契约（`Faces` 各面的方法与事件）一改 → 本处渲染与 `src/tests/cli.rs`。
- 测试缺口：本单元是适配器、不是业务单元，没有独立的缺口账；缺口记在 `tests/gaps.yaml`；格式见 [docs/testing/gaps-acceptance.md](../../../docs/testing/gaps-acceptance.md) 的「缺口账」。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
