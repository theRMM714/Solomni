# 产品与用户旅程

> 本文档回答：用户怎么用这个产品。
> 产品 = 核心宿主（嵌 broker）+ 模块。产品入口（bin/solomni）只做一件事：拉核心宿主。
> 一切功能行为由模块自治（见 MODULE_DEV_GUIDE.md）；核心宿主不认识任何业务。

## 分层铁律（本文档的灵魂）

| 决定 | 归属 |
|---|---|
| 扫描哪里、拉起谁（service 自动 / surface 按需）、排除谁、退出语义 | **核心宿主**（嵌 broker；产品入口只是薄壳） |
| 我是 service 还是 surface | **模块自声明**（`Declaration.kind`），核心只校验枚举 |
| 某配置缺失时怎么办（例：无密钥时拒绝聊天还是降级） | **用到它的模块自决** |
| 交互面里能做什么（组件、命令、提示文案） | 模块经 `ui.contribution` 声明，UI 模块渲染 |
| 谁连着谁 | 核心配对（事实，非决策） |

推论：密钥只是某个模块的内部概念。产品入口从头到尾不知道"密钥"存在——
用户在调色板看到 `set-key`，是因为 secrets 模块声明了这个命令，不是因为产品了解它。

## 产品形态

一个命令，装好的模块即产品。

- **核心宿主进程** = broker（校验/配对/投递）+ 扫描/拉起 + 极简菜单（唤起 surface）+ 退出
- **所有模块都是同级外部进程**（cwd = 各自文件夹，loopback TCP 连核心），没有编译内模块：
  - `service`（conversation / secrets / llm_gateway）：无头，宿主启动即拉起
  - `surface`（ui_cli / ui_flutter）：交互面，宿主菜单按需唤起
- **交互面是模块**：终端 UI（ui_cli）与 GUI（ui_flutter）平级，核心对"几个交互面"无感知
- 模块类型由模块在 `Declaration.kind` 自声明，核心只校验枚举合法性，不代为决策

## 第一分钟（用户旅程）

```text
$ solomni                                  <- 命令名为占位，正式名待定
[发现] conversation, llm_gateway, secrets, ui_cli, ui_flutter   <- 扫描 modules/，纯事实
[conversation] 已连接核心 :9100，等待调用   <- service 无头拉起（事实）
[llm_gateway] 已连接核心 :9100，等待调用
[secrets] 已连接核心 :9100，等待调用
（核心宿主菜单；输入界面 id 唤起，exit 退出产品）
可唤起界面：
  ui_cli
  ui_flutter

> ui_cli
[唤起] ui_cli -> :9100
（终端对话模式；help 查看组件与命令，exit 返回）
> send 你好
[错误] 未配置密钥 llm，无法调用 LLM          <- 拒绝与否、怎么提示，由聊天链路自决
> set-key llm sk-****
ok
> send 你好
你好！今天想聊点什么？
> exit                                       <- 退出界面，回到宿主菜单
> exit                                       <- 退出产品
$
```

- 启动只报告**事实**：发现了哪些模块、service 已拉起。不解释任何模块的用法——那是模块自己的事
- 宿主菜单只列 `surface` 模块、只允许唤起它们；`help` 走 `ui.render`（模块能力）
- `send` / `set-key` 来自模块声明的命令，宿主不硬编码它们

## 启动选项

| 形式 | 行为 |
|---|---|
| `solomni` | 默认：拉核心宿主 + 拉起 service + 显示菜单（列 surface），**不自动启动任何 UI** |
| `solomni -gui <模块id>` | 先唤起指定 surface（`-gui ui_flutter` 与 `-gui=ui_flutter` 等价），关掉后回到菜单 |
| `solomni -gui` | 无 id：唯一候选直接唤起；多候选不替用户选，提示后回菜单 |
| `solomni --serve` | 无头常驻（拉起 service，无交互面；Ctrl+C 退出） |
| `--without=<模块id>` | 本次不拉起该模块（既有装卸语义） |
| `--verbose` | 详细诊断（默认状态行从简，不与聊天流抢屏） |
| `--port=` | 开发者覆盖；默认端口自动分配，用户与模块进程不感知 |

根目录另有**门户** `solomni.bat` / `solomni`：转发给 `bin/solomni`(.bat)，
仓库根直接 `.\solomni` 即可（`start` 是 cmd 保留字，故门户用产品命名）。

**运行日志**：每次启动器拉起产品，仓库根 `logs/` 下按时间戳生成一份
（`yyyyMMdd-HHmmss.log`，git 忽略）。日志是宿主声明的能力
`logs.append`：模块经 `Need(preferShared)` 消费——有 logs 就写，
没有降级为仅控制台；宿主自身的装配/唤起/离线事件同记其中。

surface 的唤起规则（宿主无静默选择）：

- 菜单只列有 `dev`/`dev.bat` 启动契约的模块（`surface`），只允许唤起它们
- `-gui` 无 id 且候选不唯一 → 不替用户选，列出候选后回菜单
- 唤起后宿主演等该模块 `hello` 到达，校验其自声明 `kind == surface`，不一致则拒绝
- 点名的模块存在但无对应 `dev`/`dev.bat` → 报"不支持当前平台"

service 无启动契约，宿主用默认 `dart bin/coordinated.dart --port=N` 拉起（cwd=模块文件夹）；
surface 走模块自备的 `dev`/`dev.bat`（怎么跑、依赖与环境都是模块自己的事；平台支持 = 脚本存在性）。
产品启动器 `bin/solomni`(.bat) 只是便捷薄壳：免去 cd 与长路径，不含任何业务。

## 退出语义

| 事件 | 语义 |
|---|---|
| surface 退出（GUI 关窗 / 终端 `exit`） | **模块离线**：核心重配对，宿主回到菜单继续运行 |
| 宿主菜单 `exit` / Ctrl+C | **产品进程消亡**：宿主终止全部子模块进程 |

原则：模块进程的消亡是离线事件，宿主进程的消亡才是产品退出。
终端不是特权交互面——ui_cli 与 ui_flutter 一样是 surface 模块。

## 模块私有数据（userdata）

数据归模块，落在模块自己文件夹内的 `userdata/`（如 `secrets/userdata/keys.json`），
git 忽略（`**/userdata/`），永不入库。密钥明文 + Unix 侧 0600；设环境变量
`SOLOMNI_USERDATA` 可整体搬迁（验收/隔离用）。

## 模块装卸的用户感知（v1）

- 放入/移出 `modules/` 文件夹 + 重启生效；启动时的 `[发现]` 清单即全部反馈
- 运行中的模块进程死掉 = 即时离线、消费方即时降级（已由核心配对保证）
- 无"不可卸载"清单：卸载任何模块的后果都是降级而非崩溃

## 后置工作（不在 v1）

- 命令名定稿与二进制打包（当前开发态：`dart run` 于 `apps/solomni`）
- GUI 模块的预编译产物（免每次 `flutter run` 的编译等待）
- 宿主被强杀（SIGKILL）时 Linux 侧子模块进程的进程组回收（当前正常退出/Ctrl+C 已显式终止子进程）
- 热装卸（免重启发现新模块文件夹）

## 实现进度

- 本文档为规范，已落地于 `apps/solomni`：`lib/assembler.dart`（核心宿主：扫描/拉起/校验）、
  `bin/main.dart`（菜单 + `-gui` 唤起 + `--serve/--without/--verbose/--port`）、
  `tool/smoke.dart`（脚本化验收，与宿主共用同一个 assemble）。
- 模块类型（`ModuleKind`）与私有数据（`user_data` 包）已随协议/新包落地；
  纯 broker 包（packages/core）只做校验/配对/投递，不读文件、不起进程。
