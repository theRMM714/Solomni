# 模块契约（MODULE_SPEC）

> 模块与核心之间的全部约定。除此之外，双方互不依赖。
> **本文只讲模块作者要遵守的契约**；系统工具、路径模型与回报/验收见 [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md)（门户 [SYSTOOL.md](SYSTOOL.md)），
> 代码分层与开发规则见 [ARCHITECTURE.md](ARCHITECTURE.md)，登记处字段见 [REGISTRY_SPEC.md](REGISTRY_SPEC.md)，
> 运行包契约见 [RUNTIME_SPEC.md](RUNTIME_SPEC.md)，项目方向见 [PHILOSOPHY.md](PHILOSOPHY.md)，产品流程见 [PRODUCT.md](PRODUCT.md)，
> 测试规范见 [TESTING.md](TESTING.md) 与 `docs/testing/`。

## 设计立场

**自由是默认，锚点只有机器不得不判定的几处。**

- 讨论里怎么说话、任务怎么拆、中间产物用什么格式——完全自由，由 agent 自决。
- 固定的结构只有机器不得不判定的几处，都只是文本；**没有 SDK、没有基类、没有编译期依赖**：
  ① 打包格式（`module.yaml`），② 发言信封，③ 回报与验收清单，④ 供应商引用。

## 模块可以是什么

**模块工具的调用约定只有一条：一条启动命令 + 参数 + stdin/stdout 回执。** 下列形态都只是它的实例——不要求、也不检查模块内部用了什么语言（长驻服务另有 `services` 契约，见下）：

| 形态 | 怎么落成 | 需要满足 |
| --- | --- | --- |
| 一份提示词 | 只写 `system`，不声明 `tools` | 无——模型就是执行者 |
| 脚本（Python / Node / C++ / …） | `tools.<名>.command` 指向自己的程序 | 命令能被拉起、按声明接参、把回执写进 stdout |
| 一个现成的提示词包 | 说明正文进 `system`，脚本做成工具 | 同脚本 |
| 一个 MCP | 一次性工具写 `tools.<名>.command`；长驻的 MCP 服务写 `services.<名>`（见「一、打包格式」） | 工具能一次请求—响应往返后退出；服务由适配器（当前 `mcp`）拉起并长驻 |
| 另一个 harness | `command` 指向那个 harness 的命令行 | 同上；桥由谁写由你决定 |

**边界**：契约只规定"命令怎么被拉起、参数怎么传、回执怎么回"；模块内部完全自由——**能跑通就是合法的模块**，
跑不通时核心如实报错，不猜测、不兜底。

**外部可执行文件与可达范围**（边界由 `src/tests/fence_reach.rs` 的围栏可达范围用例钉住）：

- 命令里**按 PATH 名**写的程序（`python tools/x.py`、`node tools/x.js`、`npx …`）——它的安装目录被放行，可达；
- 命令里写**绝对路径**的程序——绝对路径的 token 一律不当程序解析（那可能是数据文件），所以模块目录之外的
  可执行文件**不可达**；这类 harness 要放进模块目录，或改用它的 PATH 名；
- 模块目录之外的**运行时依赖**（缓存、`node_modules`、HOME 下的数据）不可达。

这是**跨平台设计**而非某平台的偶然行为：Linux 的只读基线 + PATH 解析、Windows 的容器按落点授权、macOS 的 seatbelt 同一口径。

## 一、打包格式（module.yaml）

一个文件夹 = 一个模块。模块被识别的唯一条件：文件夹内有合法的 `module.yaml`。

```yaml
id: research            # 全局唯一，= 文件夹名
enabled: true           # 【可选】缺省 true：模块的作者默认启用状态；用户覆盖落 .home/modules.yaml
brief: >-               # 能力简述（两三句）：给核心选人看，给菜单上的用户看
  调研、对比与选型。只说明能干什么，不列具体能力。
system: >-              # 工具用法：参数 / 路径 / 输入输出 / 错误 / 前置条件（能力写在 brief）
  scan 用法：root 与 out 必填；include 是可选扩展名白名单，缺省 md,txt,html,json。
runtimes:               # 【可选】运行能力：工具需要哪些运行包（只写能力名，版本由用户定）
  - python
tools:                  # 【可选】模块工具表：工具名 → 声明
  read_txt:
    command: python tools/read_txt.py   # 工作目录 = 本模块根目录
    desc: 读一个文本文件并原样返回        # 【可选】给模型看的一句话
    parallel: true                      # 【可选】声明可并发（只读、无副作用才写）
    params:                             # 【可选】参数契约：不写 = 核心不校验，args 原样交给工具
      path:
        type: string                    # string / integer / number / boolean / array / object
        required: true                  # 缺了就是参数错（核心拒收，不启动进程）
        desc: 要读取的路径（用【工作环境】的写法）
services:               # 【可选】常驻服务：服务名 → 声明（用哪个适配器拉起 + 启动命令）
  fs:
    adapter: mcp        # 组合根注册的适配器名（当前已注册 mcp：stdio 长驻会话）；未知 = 拒收并列出可用名
    command: npx -y @modelcontextprotocol/server-filesystem
    desc: 访问本地文件系统     # 【可选】给模型/用户看的一句话
    enabled: false           # 【可选】缺省 false：没经用户同意不起外部常驻进程
secrets:                # 【可选】隐秘字段：字段名 → 注入该模块进程的环境变量名（只声明标识，值归产品）
  github_token:
    env: GITHUB_TOKEN   # 注入该模块工具/服务进程的环境变量名
    desc: 访问 GitHub 的细粒度只读令牌   # 【可选】
```

规则：

- `id` 与文件夹名一致且全局唯一；不一致 = 非法，核心拒收并说明原因。
- `brief` 只说"能干什么"，不承诺具体能力；它是**选人的唯一依据**。
- `system` 写工具用法；**有 `tools` / `services` 的模块只写怎么用**（参数、路径、输入输出、错误、前置条件），能力摘要写在 `brief`，不要写“你负责 / 你是 / 你的职责 / 管辖边界”这类职责与人格描述。
  纯提示词模块（不声明任何工具或服务）没有工具可描述，`system` 写它要求模型遵守的行为约束；同样避免把 agent 人格化。
  不要在 `system` 里复述讨论协议——协议由核心按身份注入（见「模块与工具的身份」）。
- `runtimes` 只声明能力名（`python` / `node` / `bash` / `cc`……），格式不合法或重复 = 拒收并说明原因。
  它是**两档**的就绪判据：本机档按宿主目标从 `runtimes/` 选 `prefix` 包，基础能力由项目内 base profile 满足，缺失时用可授权的宿主回退；虚拟机档按 guest 平台选包或 guest 自带。
  **声明不构成保证**：命令仍是模块作者的自由文本；能力没装载、命令跑不起来都如实报错，不静默兜底
  （唯一的例外是下面那条**相对路径入口**——它是模块内产物，建会话前会先查它到底在不在）。
- `tools.<名>.command` 的**首个 token** 决定入口形态，产品据此判"跑不跑得起来"：
  **相对路径**（含 `/` 或 `\`，如 `build/indexer`、`./tools/run.sh`）= 模块内的产物，模块目录下必须**存在且能直接执行**
  （unix 要求可执行位）——不在场时**建会话就拒绝**，并指到模块自己的 README；
  **裸名**（`python` / `node` / `indexer`）走 PATH 与 `runtimes` 能力；**绝对路径**不由模块目录判。
- `tools` 占用内置保留名（`read` / `write` / `edit` / `patch` / `list` / `search` / `submit_report`）= 拒收；
  每个工具必须写 `command`（空 = 拒收）。
- `tools.<名>.params`（可选）：写了就由核心按它校验（参数不符**不启动进程**，把"哪个参数不合"与工具签名一起发回模型），
  并渲染进 agent 的系统提示；不写 = 不校验、`args` 原样交给工具。**契约写在清单里，不写进代码。**
- `services` 与 `secrets`（可选）：`services.<名>` 必写 `adapter` 与 `command`（缺 = 拒收），`enabled` 缺省 false；
  顶层 `enabled` 是模块的作者默认（缺省 true），用户覆盖落 `.home/modules.yaml`；**模块停用 ⇒ 它的服务一律有效地停用**；
  `secrets.<名>` 必写 `env`（缺 = 拒收）。核心只校验语法，不解释适配器名与 `options`（注册表匹配在常驻服务能力）。
- `tools.<名>.parallel`（可选，缺省 false = 独占串行）：只该给只读、无副作用的工具写 true。
  同一模块被多个 agent 共用时，未声明 `parallel` 的调用在**模块内跨 agent 串行**（保护模块自己的 `userdata/`），
  声明了的只读调用可并发；不同模块、内置工具与各 agent 的沙箱写入不受影响。
  **声明即承诺**：把有副作用的工具写成可并发，后果由模块作者承担。
  **虚拟机档里不生效**：同一租约一把锁、一台 guest 只有一条 QGA 命令通道，模块工具在 guest 里本来就只能串行——
  `parallel` 表达的是"可并发"的意图，在不具备并发条件的执行环境里以串行为准（不算违约）。
- **模块的独立入口**：声明的 `tools`（与 `services`）就是模块**不经 agent 可直接跑**的入口——
  CLI `module <模块id>.<工具名> [json]`、`resident …` 与 Web 的「模块与服务」面板走同一份动作目录；
  只写 `system`、不声明任何入口的模块在列表里如实标注「只能经 agent 使用」。
  **没有额外的独立入口声明节**（有意少一层契约：声明了哪个工具，就是能直跑哪个）。
- `userdata/` 是模块私有的**跨任务**状态区（缓存 / 记忆 / 配置）：git 忽略、数据不出工作区；载入模块时**确保它存在**（产品唯一的自动写盘，幂等；建不了不阻断加载、如实标注）；
  **模块目录（含 `userdata/`）对 agent 的内置文件工具只读**——agent 只有模块的使用权；`userdata/` 只由模块自己的工具 / 服务进程写。
  **模块工具的入参若会落到 `userdata/`，语义由模块作者校验**；围栏只保证不越出 `userdata/`。
  一次工作的产物写该 agent 的沙箱（工作副本）再用 `work_commit` 提交回共享区；文本、图片、视频与其它二进制都按字节处理（内容流式读写、不整份进上下文；单文件有上限，超限会明确拒绝），冲突用 `work_versions` 看三方、`work_resolve` 选边。不要塞进 `userdata/`。**密钥永不落在模块。**
- 模块损坏或 yaml 非法 → 不进清单，菜单少一项，永不崩溃。

## 二、模块与工具的身份

**模块不发言，agent 发言。** 模块交三样东西：`brief`（能力摘要）、`system`（工具用法）与 `tools`（模块工具表）。
模块被装进哪个 agent，就以那个 agent 的名义说话；一个 agent 装多个模块时，它的工具说明 = 这些 `system` 的合成，能力摘要 = 这些 `brief` 的一行。

工具的身份是 **(模块 id, 工具名)** 这一对：**不同模块可以声明同名工具**，调用时写明 `module` 就各归各模块。
**讨论协议、发言动词、回报与验收都不在模块这边**——由核心按身份注入，所以 `system` 里**不要复述协议**（写了多余，协议改了还会漂）。

## 三、回报与验收

**都由系统工具承载，模块不参与。** 执行席做完调 `submit_report` 交回执（它就是这一轮的最终答复）；
节点与总验收由核心调 `node_verdict` / `checklist`，`fail` 必须点名要返工的**节点 id**。

模块作者只需记住一件事：**回报不是要模块自己手写的 JSON 块**。工具怎么声明、谁能调、载荷什么形状，
唯一权威在 [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md)（声明在 `systools/tools.yaml`）。

## 四、编译型模块的构建（`build:` 段）

模块可以是**任何语言**：脚本型（`runtimes: [python]` + `tools.<名>.command`）就够了；要编译的模块在清单里声明**构建方法**：

```yaml
build:                  # 【可选】系统在**目标环境**里照它把产物编出来
  command: tools/build  # 平台各一份脚本：unix 放 `tools/build`、Windows 放 `tools/build.cmd`
  needs: [gcc]          # 【可选】构建期能力名 = 目标环境 PATH 上的**可执行文件名**
  net: false            # 【可选】构建期要不要出站网络（缺省 false）
tools:
  build:
    command: build/indexer build     # 相对路径入口 = 模块内的产物（见下）
```

- **谁编**：**系统**（不是 agent）——`solomni --build=<模块 id>`。产品在**目标环境**里跑 `command`
  （本机档在宿主；虚拟机档用 `--guest=<虚拟根>` 在 **guest 里用它的编译包**编），围栏由产品给
  （只写模块目录与状态目录、按 `net` 声明）；**工具链不进 agent 的工具围栏**。
  本机档构建围栏额外**只读**放行项目内基础工具链的安装根（编译器要读自己的 libexec / 动态库），仍然不能写它。
  `net` 的平台落点：虚拟机档决定 guest 网卡；本机档 Windows 授 `internetClient`、macOS 放行
  `network-outbound`、Linux `net=false` 用 seccomp 拒绝非本地 socket（`net=true` 不装网络过滤器，不限制入站/绑定）。
  默认 false；Windows 只放行公网出站，不授 private network / loopback。
- **入口判据**：`tools.<名>.command` 的首个 token 是**相对路径**时，它就是模块内的产物，模块目录下必须有那个
  **在目标环境里能直接跑**的文件——名字按**目标环境**的壳解析（本机档 = 宿主；虚拟机档 = guest 的 `os`）：
  unix 要求可执行位，Windows 补 `.exe` / `.cmd` / `.bat` / `.com`；跨平台时还要看**文件形式**（ELF / Mach-O / PE / 脚本头）——
  宿主编出来的产物在另一种平台的 guest 里跑不了，一样算不在场。不在场就在「开始」那一刻**拒绝**，
  模块声明了 `build` 就指到 `--build`（虚拟机档要在那个虚拟根里编：`--build=<模块 id> --guest=<虚拟根>`）。
- **缓存**：状态落 `.vm/build-state/<模块>.json`（源指纹 + 命令 + 能力 + 网络 + 目标 + 实际提供者）；指纹一致且入口在场就跳过重编。
- **产物**：写进模块目录（`build/` 已在 `.gitignore` 里）；agent 看得见产物、看不见工具链。
- **本机档**：`needs` 由 `packages` 按**宿主目标**选包（运行包优先；基础能力由项目内 base profile 满足，缺失时用宿主回退，但只在围栏能授权时算可用）；缺工具如实拒绝，并给"装基础工具链 / 把适用于本机目标的包放进 `runtimes/` / 自己编 / 让作者发产物"几条出路。
- **跨平台**：构建命令写成**模块内的脚本**——unix 放 `tools/build`（要执行位），Windows 放 `tools/build.cmd`；两个文件并排，
  各自的壳自己找得到（Windows 的 cmd 按 PATHEXT，与 `npm` 的 shim 同一条规矩）。
  **`.cmd` 只写 ASCII**：cmd 按控制台代码页解释批处理字节，非 ASCII 的 `rem` 注释会把**下一行整行吞掉**，构建直接失败；`chcp 65001` 在管道 stdio 下也救不了。
  说明文字写进模块 README，`--build` 与门禁都按这条判（`run-tests.js` 的结构审查扫 `.cmd`/`.bat` 的非 ASCII 与 BOM）。
  **产品负责安装基础工具链**（Python / Node / C-C++，运行前提，装进项目内 `.tools/toolchains`；
  `setup-runtime.js` 默认安装，`start.js` 首启询问、之后每次启动报告）；额外工具链不由产品预置：
  `needs` 就是目标环境生效 PATH 上的可执行名（如 `gcc`），缺了如实拒绝——"声明不构成保证"；
  宿主回退只在平台围栏能授权时算可用。
- **虚拟机档**：`needs` 由 guest 自带（`guest.yaml` 的 `runtimes`）或**运行包库**满足（prefix 的 `bin` 进 PATH、system 的载荷叠写进系统路径）；
  模块目录以**读写**挂进 guest，产物因此直接落回宿主那份；guest 用完即销毁（会话级可写层），不改基础镜像。

## 五、工具执行

工具属于模块，使用权跟着装了它的 agent：**执行阶段与单 agent 会话**里都能用；讨论阶段只发只读核实工具（`read` / `list` / `search`），不给模块工具。

- **调用**：agent 发 `tool` 信封；参数是 JSON 对象，经 stdin 送入工具进程（不进命令行，杜绝注入）。
- **编码**：stdin 与 stdout **统一 UTF-8**（核心为子进程强制；否则 Windows 上中文参数会被按系统代码页解坏）。
- **放行**：核心只执行本模块 `tools` 里声明过的工具；表外一律拒绝并把可用清单发回。
- **参数**：声明了 `params` 就按它校验；不符时**不启动进程**，回执说清哪个参数不合，并把该工具的签名原样发回。
- **所属模块**：`module` 指明用哪个模块声明的工具；**省略时**只有该 agent 恰好一个模块才替它定，否则如实报错并列出可用的 `模块.工具`。
  内置工具（`read` / `write` / `edit` / `patch` / `list` / `search` / `submit_report`）属核心，不写 `module`。
- **运行**：工具进程的工作目录 = **它所属模块**的根目录；命令语言不限，解释器须在运行机上可用；
  命令里的路径统一写 `/`（跨平台）——守门进程在 Windows 上只把**程序名**里的 `/` 换成 `\` 再交 `cmd` 解释（cmd 不认程序名里的正斜杠）。
- **运行能力**：模块在 `runtimes` 里声明工具需要的能力；执行环境随会话的**执行档位**而定（本机档按宿主目标装载 `prefix` 包，虚拟机档按 guest 平台装载包）。
  该能力在本档位不可用 = **该模块的工具不执行**并如实说明缺哪个能力（讨论与其它模块照常）；多版本未定版 / 定版不在库 / 两个包写同一系统路径 = 虚拟机档「开始」那一刻如实拒绝。
  本机档的 `system` 类包报"不服务本机档"；包没有可执行 `bin/` 也如实报，不静默回退。
- **回注**：stdout/stderr（截断后）作为 `[工具结果]` 消息发回 agent，继续生成直到最终答复；**单轮里的工具调用没有次数上限**。
- **如实呈现**：每次调用在转录里自成**一条工具行**（前端渲染成工具卡片：模块.工具名 + 成败，参数与结果折叠可展开）；未声明 / 越界也照此如实呈现。
- **边界如实**：工具进程统一从**守门进程**里跑（环境白名单、超时连根杀树、可达范围按平台围栏强制），能力等级在启动时如实报告。
  模块作者仍应把工具写成只碰自己工作区的样子。
- **工具看到的路径**：可读写的是该 agent 的私有沙箱（工作副本，**永远全权**）；模块目录（含 `userdata/`）对 agent 的**内置文件工具**永久只读——
  agent 只有模块的使用权；模块工具 / 服务进程可写自己的 `userdata/`；共享区**主副本对 agent 只读**
  （要改先 `work_pull` 拉进沙箱，改完 `work_commit`；文本 / 图片 / 视频 / 二进制同一套字节规则，单文件有上限；冲突用 `work_versions` / `work_resolve`；提交白名单可限定"只准提交哪一块"，见 [docs/permission/README.md](docs/permission/README.md)）；
  **这些目录本身与它们的直接父目录可以判断存在性**（`exists` / `stat` 得到真实结果），所以"产物目录不存在就先建"这类写法在围栏里也能正常工作。
  写进未被授权的目录仍会失败——失败在**该失败的那一层**，不连累无关路径。内置工具清单与路径模型见 [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md) 的「路径模型」。

- **模块目录就是发现边界**：围栏只让工具可达模块目录、`userdata/` 与声明的工作区。
  许多运行时会按目录**向上**自动找配置——Node 就从入口脚本向上找最近的 `package.json` 来定模块格式、解析 `require`——
  这类发现一旦落在可达范围之外，就会被系统拒绝（`ERR_INVALID_PACKAGE_CONFIG` 之类），进程在脚本执行前就倒。
  所以工具要自足：把运行时会向上找的东西自带在模块目录里，让发现终止在模块根
  （Node 模块的根放一份 `package.json`，或入口用 `.mjs`）。

- **人可以直接用它**：模块工具同时是动作表里的一条动作（id = `module.<模块id>.<工具名>`）。
  不经 agent 会话也能跑：CLI `module <模块id>.<工具名> [json 参数]`，Web `POST /api/actions/module.<模块id>.<工具名>`
  （清单见 `GET /api/actions`）。参数仍按本模块 `params` 校验；可选 `workspace` 给一份工作目录（缺省 = 模块自己的 `userdata/`）；
  围栏与 agent 会话同一口径（模块目录递归只读、`userdata/` 可写）。两条路径的审计记录同一处格式化。
  **缺运行包 = 不执行**（与成员循环同一把尺子）：本机档按宿主目标选包，缺哪个能力就如实说明、不静默降级；虚拟机档在「开始」前就**拒绝创建**（见 [RUNTIME_SPEC.md](RUNTIME_SPEC.md) §五）——没有工具用户完不成工作。

## 六、模块测试交付

模块交付时必须能说明并验证自己的行为边界——正常 / 错误 / 边界三类场景、工具协议契约、资源清理与跨平台假设、观察面与交付前检查清单。
**细则只有一份**：[docs/testing/module-delivery.md](docs/testing/module-delivery.md)；测试层级、替身语义、质量门禁与缺口账见 [TESTING.md](TESTING.md)。

## 七、供应商与密钥

**密钥归产品，不归模块。** 模块只写提示词与工具；**通道密钥永不接触模块**，只存在于 `.home/` 的登记处与核心的出站调用里，
不进提示词、转录、日志与模块工作区；模块只持 id 引用。模块需要用隐秘信息（例如 MCP server 的令牌）时，在 `secrets`
里**只声明标识与注入的环境变量名**；值由用户经设置配置、只存 `.home/`，由核心按模块作用域注入该模块自己的工具/服务进程——
**值不进提示词、转录、日志、模块工作区与命令行**。四份 yaml 的字段见 [REGISTRY_SPEC.md](REGISTRY_SPEC.md)。

## 八、制作一个模块

1. 建文件夹，写 `module.yaml`（`id` / `brief` / `system`；**只有真需要外部程序时才写** `tools`）。
2. 需要工具就把脚本放进文件夹，确认它能**独立运行**（参数走 stdin JSON、结果走 stdout）；
   用了非通用解释器就把能力名写进 `runtimes`。
3. 单独试运行：只给 `brief` + `system` + 一句用户需求，看它是否按用法调用工具；声明了 `tools` 再按
   [docs/testing/module-delivery.md](docs/testing/module-delivery.md) 验证工具协议、错误处理与清理行为。
4. 按该文件的"交付前检查清单"整理测试证据；没实现的测试如实登记为 `gap` 或 `env-skip`。
5. 放入 `modules/`——完成。清单自动出现它，菜单自动展示它的 `brief`。

文件读写、精确改写（含自由格式补丁）、关键词搜索这类通用能力**不用自己造**：核心内置了
`read` / `write` / `edit` / `patch` / `list` / `search`。**只写 `brief` + `system`、一个模块工具都不声明，就是一个能用的提示词模块。**

没有注册、没有继承、没有编译依赖。**一个文件夹就是全部。**
