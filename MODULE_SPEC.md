# 模块契约（MODULE_SPEC）
> 定位：模块作者与核心之间的全部约定：一个模块长什么样、`module.yaml` 怎么写、字段与拒收条件。
> 效力：契约｜核对：2026-11-04｜依据：`src/capabilities/modules/domain/module.rs`

## 结构
```text
modules/<模块 id>/
  module.yaml      # 模块被识别的唯一条件：一份合法清单
  tools/           # 工具脚本 / 程序（作者自定；相对路径入口从这里声明）
  build/           # 构建产物（git 忽略；agent 看得见产物、看不见工具链）
  userdata/        # 模块私有的跨任务状态（载入时创建；git 忽略）
  README.md        # 作者自述（可选；入口缺失时产品指到这里）
```
- 一个文件夹 = 一个模块；核心只认文本清单，不要求、也不检查模块内部用什么语言。
- 模块与核心之间只有四处文本约定：`module.yaml`、发言信封、回报与验收清单、供应商引用；除此之外双方互不依赖。
- 没有注册、没有继承、没有编译期依赖：没有 SDK、没有基类，一个文件夹就是全部。
- 模块工具的调用约定只有一条：一条启动命令 + 参数 + stdin/stdout 回执。
- 纯提示词：只写 `brief` + `system`，不声明 `tools` / `services`；模型就是执行者。
- 脚本（Python / Node / C++ / …）：`tools.<名>.command` 指向模块自己的程序。
- 现成的提示词包：说明正文进 `system`，脚本做成工具。
- 一个 MCP：一次性工具写 `tools.<名>.command`；长驻服务写 `services.<名>`，由适配器（当前 `mcp`）拉起并长驻。
- 另一个 harness：`command` 指向那个 harness 的命令行，桥由作者自己写。
- 契约只规定命令怎么被拉起、参数怎么传、回执怎么回；模块内部完全自由——能跑通就是合法的模块，跑不通时核心如实报错，不猜测、不兜底。
- 外部可执行文件按 PATH 名写（`python tools/x.py`、`node tools/x.js`、`npx …`）时，它的安装目录被放行、可达。
- 命令里写绝对路径的程序 token 一律不当程序解析，模块目录之外的可执行文件不可达；这类 harness 要放进模块目录，或改用它的 PATH 名（路径模型见 `docs/tools/tools-and-roles.md`）。
- 模块目录之外的运行时依赖（缓存、`node_modules`、HOME 下的数据）不可达。
- 模块目录就是发现边界：运行时会向上找的东西要自带在模块目录里，让发现终止在模块根（Node 模块根放一份 `package.json`，或入口用 `.mjs`）。
- 回报与验收都由系统工具承载，模块不参与：执行席做完调 `submit_report` 交回执（它就是这一轮的最终答复），节点与总验收由核心调 `node_verdict` / `checklist`，`fail` 点名要返工的节点 id；声明在 `systools/tools.yaml`。
- 模块交付要说明并验证自己的行为边界（正常 / 错误 / 边界、工具协议契约、资源清理与跨平台假设、观察面与交付前检查清单），细则见 `docs/testing/module-delivery.md`。
- 制作：建文件夹写 `module.yaml`（`id` / `brief` / `system`；只有真需要外部程序时才写 `tools`）→ 脚本放进文件夹并确认能独立运行（参数走 stdin JSON、结果走 stdout）→ 按交付清单整理测试证据 → 放入 `modules/`；文件读写、精确改写、删除沙箱文件、关键词搜索这类通用能力核心已内置，不用自己造。

## 一、顶层字段
| 字段 | 类型 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- | --- |
| `id` | 字符串 | 是 | - | 全局唯一，= 文件夹名 |
| `brief` | 字符串 | 是 | - | 能力简述（两三句）；选人的唯一依据 |
| `system` | 字符串 | 是 | - | 工具用法：参数 / 路径 / 输入输出 / 错误 / 前置条件；纯提示词模块写行为约束 |
| `enabled` | 布尔 | 否 | `true` | 作者默认启用状态；用户覆盖落 `.home/` 下的 `modules.yaml` |
| `runtimes` | 字符串数组 | 否 | `[]` | 工具需要的运行能力名；只写能力名，版本由用户定 |
| `tools` | 映射 | 否 | 空 | 模块工具表：工具名 → 声明 |
| `services` | 映射 | 否 | 空 | 常驻服务表：服务名 → 声明 |
| `secrets` | 映射 | 否 | 空 | 隐秘字段表：字段名 → 声明 |
| `build` | 映射 | 否 | 无 | 构建方法；有它时 `command` 必填 |
- `id` 与文件夹名一致且全局唯一；不一致 = 非法，核心拒收并说明原因。
- `brief` 只说"能干什么"，不承诺具体能力。
- `system` 写工具用法；有 `tools` / `services` 的模块只写怎么用（参数、路径、输入输出、错误、前置条件），能力摘要写在 `brief`，不写"你负责 / 你是 / 你的职责 / 管辖边界"这类职责与人格描述。
- 纯提示词模块没有工具可描述，`system` 写它要求模型遵守的行为约束，同样避免把 agent 人格化。
- 不要在 `system` 里复述讨论协议——协议由核心按身份注入（见「二、模块工具」），写了多余，协议改了还会漂。
- `runtimes` 只声明能力名（`python` / `node` / `bash` / `cc`……），格式不合法或重复 = 拒收并说明原因；两档的就绪判据见 `docs/vm/README.md`。
- 声明不构成保证：命令仍是模块作者的自由文本，能力没装载、命令跑不起来都如实报错，不静默兜底；唯一的例外是相对路径入口——它是模块内产物，建会话前会先查它到底在不在。
- `userdata/` 是模块私有的跨任务状态区（缓存 / 记忆 / 配置）：git 忽略、数据不出工作区；载入模块时确保它存在（产品唯一的自动写盘，幂等；建不了不阻断加载、如实标注）。
- 一次工作的产物写该 agent 的沙箱（工作副本），再用 `work_commit` 提交回共享区；文本、图片、视频与二进制都按字节处理（内容流式读写、不整份进上下文；单文件有上限，超限会明确拒绝）；冲突用 `work_versions` 看三方、`work_resolve` 选边；产物不要塞进 `userdata/`。
- 模块损坏或 yaml 非法 → 不进清单，菜单少一项，永不崩溃。

## 二、模块工具（`tools`）
| 字段 | 类型 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- | --- |
| `<工具名>` | 映射 | 否 | 空 | 工具名 → 声明；不得占用内置保留名 |
| `command` | 字符串 | 是 | - | 启动命令；工作目录 = 模块根目录；首个 token 决定入口形态 |
| `desc` | 字符串 | 否 | 空 | 给模型看的一句话 |
| `params` | 映射 | 否 | 无 | 参数契约；不写 = 核心不校验，`args` 原样交给工具 |
| `parallel` | 布尔 | 否 | `false` | 声明可并发（缺省 false = 独占串行） |
- `command` 必填，空 = 拒收；命令里的路径统一写 `/`（跨平台）。
- 首个 token 决定入口形态：相对路径（含 `/` 或 `\`，如 `build/indexer`、`./tools/run.sh`）= 模块内的产物，模块目录下必须存在且能直接执行（unix 要求可执行位）——不在场时建会话就拒绝，并指到模块自己的 README。
- 裸名（`python` / `node` / `indexer`）走 PATH 与 `runtimes` 能力；绝对路径不由模块目录判。
- 占用内置保留名（`read` / `write` / `edit` / `patch` / `delete` / `list` / `search` / `submit_report`）= 拒收；内置工具属核心，调用不写 `module`。
- 工具身份是（模块 id, 工具名）这一对：不同模块可以声明同名工具，调用时写明 `module` 就各归各模块。
- 省略 `module` 时，只有该 agent 恰好一个模块才替它定，否则如实报错并列出可用的 `模块.工具`。
- 模块不发言，agent 发言：模块被装进哪个 agent，就以那个 agent 的名义说话；一个 agent 装多个模块时，它的工具说明 = 这些 `system` 的合成，能力摘要 = 这些 `brief` 的一行。
- `parallel`（缺省 false = 独占串行）：只该给只读、无副作用的工具写 true；同一模块被多个 agent 共用时，未声明 `parallel` 的调用在模块内跨 agent 串行（保护模块自己的 `userdata/`），声明了的只读调用可并发；不同模块、内置工具与各 agent 的沙箱写入不受影响；声明即承诺，把有副作用的工具写成可并发，后果由模块作者承担。
- 调用：agent 发 `tool` 信封；参数是 JSON 对象，经 stdin 送入工具进程（不进命令行，杜绝注入）。
- 编码：stdin 与 stdout 统一 UTF-8（由核心为子进程强制）。
- 放行：核心只执行本模块 `tools` 里声明过的工具；表外一律拒绝并把可用清单发回（清单与调用面见 `docs/tools/tools-and-roles.md`）。
- 参数：声明了 `params` 就按它校验；不符时不启动进程，回执说清哪个参数不合，并把该工具的签名原样发回。
- 回注：stdout/stderr（截断后）作为 `[工具结果]` 消息发回 agent，继续生成直到最终答复；单轮里的工具调用没有次数上限。
- 阶段：执行阶段与单 agent 会话里都能用；讨论阶段只发只读核实工具（`read` / `list` / `search`），不给模块工具。
- 独立入口：声明的 `tools`（与 `services`）就是模块不经 agent 可直接跑的入口——CLI `module <模块 id>.<工具名> [json]`、`resident …` 与 Web 的「模块与服务」面板走同一份动作目录；工具同时是动作表里的一条动作（id = `module.<模块 id>.<工具名>`）。
- 直接调用仍按本模块 `params` 校验；可选 `workspace` 给一份工作目录（缺省 = 模块自己的 `userdata/`），围栏与 agent 会话同一口径。
- 只写 `system`、不声明任何入口的模块在列表里如实标注「只能经 agent 使用」；没有额外的独立入口声明节——声明了哪个工具，就是能直跑哪个（动作表见 `docs/tools/tools-and-roles.md`）。
- 模块工具的入参若会落到 `userdata/`，语义由模块作者校验；围栏只保证不越出 `userdata/`。
- 工具看到的路径：可读写的是该 agent 的私有沙箱（工作副本，永远全权）；模块目录（含 `userdata/`）对 agent 的内置文件工具永久只读——agent 只有模块的使用权；模块工具 / 服务进程可写自己的 `userdata/`；共享区主副本对 agent 只读，要改先 `work_pull` 拉进沙箱、改完 `work_commit`，冲突用 `work_versions` / `work_resolve`。

## 三、工具参数（`params`）
| 字段 | 类型 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- | --- |
| `type` | 字符串 | 是 | - | `string` / `integer` / `number` / `boolean` / `array` / `object` |
| `required` | 布尔 | 否 | `false` | 缺了就是参数错（核心拒收，不启动进程） |
| `desc` | 字符串 | 否 | 空 | 给模型看的一句话 |
- 参数类型只认六种；整数与数字分开判定，不做 `1 == 1.0` 的宽容。
- 写了 `params` 就由核心按它校验，并渲染进 agent 的系统提示；不写 = 不校验、`args` 原样交给工具。
- 契约写在清单里，不写进代码。

## 四、常驻服务（`services`）
| 字段 | 类型 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- | --- |
| `<服务名>` | 映射 | 否 | 空 | 服务名 → 声明 |
| `adapter` | 字符串 | 是 | - | 组合根注册的适配器名（当前 `mcp`：stdio 长驻会话）；未知 = 拒收并列出可用名 |
| `command` | 字符串 | 是 | - | 启动命令；工作目录 = 模块根目录 |
| `desc` | 字符串 | 否 | 空 | 给模型 / 用户看的一句话 |
| `enabled` | 布尔 | 否 | `false` | 没经用户同意不起外部常驻进程 |
- `adapter` 与 `command` 必写（空 = 拒收）。
- 核心只校验语法，不解释适配器名与 `options`；注册表匹配在常驻服务能力。
- 模块停用（`module.yaml` 的 `enabled`）⇒ 它的服务一律有效地停用。

## 五、隐秘字段（`secrets`）
| 字段 | 类型 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- | --- |
| `<字段名>` | 映射 | 否 | 空 | 字段名 → 声明 |
| `env` | 字符串 | 是 | - | 注入该模块工具 / 服务进程的环境变量名 |
| `desc` | 字符串 | 否 | 空 | 说明 |
- 只声明标识与注入的环境变量名，不声明值；缺 `env` = 拒收。
- 值由用户经设置配置、只存 `.home/` 的隐秘登记处，由核心按模块作用域注入该模块自己的工具 / 服务进程。
- 值不进提示词、转录、日志、模块工作区与命令行；密钥永不落在模块。
- 通道密钥永不接触模块：只存在于登记处与核心的出站调用里，模块只持 id 引用。

## 六、构建段（`build`）
| 字段 | 类型 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- | --- |
| `command` | 字符串 | 是 | - | 构建命令；工作目录 = 模块根 |
| `needs` | 字符串数组 | 否 | `[]` | 构建期能力名 = 目标环境 PATH 上的可执行文件名 |
| `net` | 布尔 | 否 | `false` | 构建期要不要出站网络 |
- 有 `build` 时 `command` 必填（空 = 拒收）；`needs` 里的能力名格式同 `runtimes`，非法或重复 = 拒收。
- 构建命令写成模块内的脚本：unix 放 `tools/build`（要执行位），Windows 放 `tools/build.cmd`，两个文件并排，各自的壳自己找得到。
- 系统（不是 agent）在目标环境里跑 `command`：创建会话前对名单里入口不在场的模块自动补编，手动入口是 `solomni --build=<模块 id>`。
- 成败以产物为准：命令退出码只当快速失败信号；零退出但入口不在场同样算失败。
- `net` 缺省 false；要下载依赖的构建显式声明。
- 构建围栏、缓存、失败判定与跨平台写法的完整口径见 `docs/builds/README.md`。

## 示例
- 最小可用清单（只写 `brief` + `system`，不声明任何工具）：
```yaml
id: research
brief: >-
  调研、对比与选型。只说明能干什么，不列具体能力。
system: >-
  scan 用法：root 与 out 必填；include 是可选扩展名白名单，缺省 md,markdown,htm,txt,html,json。
```
- 带工具、参数、服务、隐秘字段与构建的清单：
```yaml
id: research
enabled: true
brief: >-
  调研、对比与选型。只说明能干什么，不列具体能力。
system: >-
  scan 用法：root 与 out 必填；index 用法：root 必填。
runtimes: [python]
build:
  command: tools/build
  needs: [gcc]
  net: false
tools:
  scan:
    command: python tools/scan.py
    desc: 扫描目录
    parallel: true
    params:
      root:
        type: string
        required: true
        desc: 要扫描的根目录
      out:
        type: string
        required: true
        desc: 结果写到哪里
  build:
    command: build/indexer build
services:
  fs:
    adapter: mcp
    command: npx -y @modelcontextprotocol/server-filesystem
    desc: 访问本地文件系统
    enabled: false
secrets:
  github_token:
    env: GITHUB_TOKEN
    desc: 访问 GitHub 的细粒度只读令牌
```

## 校验与拒收
- 文件夹内没有合法 `module.yaml`，或 yaml 解析失败 → 该文件夹不是模块，不进清单，菜单少一项，永不崩溃。
- `id` 与文件夹名不一致，或全局不唯一。
- `tools` 占用内置保留名，或某个工具没写 `command`。
- 相对路径入口在模块目录下不存在、或不能直接执行（unix 无可执行位）→ 建会话拒绝，并指到模块自己的 README。
- `tools.<名>.params` 的 `type` 不在六种之内。
- `runtimes` 里的能力名不合法（小写字母或数字开头，其余小写字母、数字、`-` `_` `.`，长度 1..=64）或重复。
- `services.<名>` 缺 `adapter` 或 `command`；`adapter` 不在注册表里时，使用前如实报错并列出可用名。
- `secrets.<名>` 缺 `env`，或字段名为空。
- `build.command` 为空，`build.needs` 里的能力名不合法或重复。
- 调用表外工具 → 拒绝并把可用清单发回。
- 参数与 `params` 不符 → 不启动进程，回执说清哪个参数不合，并附该工具的签名。
- 省略 `module` 且该 agent 不止一个模块 → 报错并列出可用的 `模块.工具`。
- 选定执行档位里缺能力 → 该模块的工具不执行并如实说明缺哪个能力（讨论与其它模块照常）。
- 核心只校验，不纠正：非法清单拒收并说明原因。

## 路由
- 构建的执行、缓存、失败归因与目标环境 → `docs/builds/README.md`
- 工具执行、路径模型、权限围栏与回报 / 验收 → `docs/tools/tools-and-roles.md`、`docs/permission/README.md`
- 运行包契约与执行档位 → `RUNTIME_SPEC.md`、`docs/vm/README.md`
- 供应商 / 模型 / agent / 设置登记处字段 → `REGISTRY_SPEC.md`
- 模块测试交付、测试层级与缺口账 → `docs/testing/module-delivery.md`、`TESTING.md`
- 模块公地清单与加载 → `docs/modules/README.md`
- 分层、依赖方向与业务边界 → `ARCHITECTURE.md`
