# 架构与开发规则（ARCHITECTURE）

> 定位：分层与机制归属的唯一权威：依赖方向、端口画在哪、机制落哪一层，以及状态落盘的归属规则与去向。
> 效力：规范｜核对：2026-11-09｜依据：`src/main.rs`

## 适用与判据
- 改代码前必须先读本文；分层判断以本文为准（`ARCHITECTURE.md`）。
- 违反由 T0 结构审查与 `node run-tests.js` 判定；依赖方向基线在 `tests/dependency-baseline.json`。

## 一、分层与依赖方向

- 依赖箭头只有一种画法（每层只指向它的下层）：

```text
presentation/{cli,web} ──▶ capabilities（含协调业务 conductor）──▶ kernel
（main = 组合根，装配全部；各能力的 `detail` 与 kernel 的实现只在这里被 new 出来，它们不认识业务编排）
```

| 层 | 干什么 | 禁令 |
| --- | --- | --- |
| `kernel/` | **机制型业务**：无领域语义、无领域状态的机制（运行日志端口、宿主探测、取消表、跨业务共享的事实类型、路径书写、外部进程执行与长驻子进程账、围栏机制）；形状与别的能力一致（`api` / `ports` / `domain` / `detail`） | **不依赖任何人**；不放有领域语义的类型 |
| `capabilities/` | **业务能力**：按业务功能垂直切分。每个能力有 `api`（入站契约：trait + DTO）/ `service`（本能力的状态与用例）/ `ports`（出站端口，只由定义它的能力持有）/ `domain`（纯逻辑）/ `detail`（只有组合根能构造） | **业务之间只经对方的 `api`**；不反向依赖 `entry` / `presentation`（T0 机器判定，见 §九.7） |
| `capabilities/conductor/` | **协调业务**：会话中心（在世表、命令队列、运行态）、生成驱动、跨能力用例与跨会话回档编排；与别的能力**平级** | 不读文件、不发网络、不碰 stdin/stdout——机制下沉各能力的 `detail/` |
| `presentation/{cli,web}/` | **前端（交付机制）**：各渠道一个子目录，完全分开——传输（argv/stdout vs HTTP）、路由、纯渲染。**不是业务能力** | 只依赖各能力的 `::api`；**拿不到端口对象，也拿不到 `Core`**；两者互不依赖 |
| `entry/` + `main.rs` + `diagnostics/` + `guard/` | **入口层**：组合根（`main.rs`：`new` 出所有适配器并注入）+ 机器可读探针（`--doctor` / `--https-check` / `--print-routes` / `--print-fence-env` / `--fence-verify`）+ 围栏守门进程与台账处置（`--fence-run`；以及 `--fence-clean` / `--fence-reconcile` / `--fence-ledger` / `--fence-restore` / `--fence-revoke` / `--fence-profile-rm`）+ 长驻子进程守门进程（`--proc-watchdog`，macOS 的父死子亡） | 它依赖所有人，**任何人都不许依赖它**；除装配与探针外无业务 |

- 推论：

- 「用哪个供应商 / 模型」是**策略**（`registry` 的解析链决定）；「怎么建通道」是**机制**（llm 的 `detail`）。两者不互换。
- 出站依赖由**各能力定义端口**、**各能力的 `detail` 实现**；入站依赖由**各能力定义 `api`**；`Ops` 只把它们**组装**给呈现层。
- **能力不定义"前端接口让别人实现"**。
- 呈现层拿到的是 `Ops`（各能力的能力接口 + 核心自己的两个 + 事件台），不是 `Core`，也不是任何锁。

## 二、端口：只画在 IO 与可替换点上

- 端口 = 各能力 `ports.rs` 与 `src/kernel/ports.rs` 里的 trait（`Chat` 的定义在 `src/capabilities/llm/api.rs`，因为它同时是能力的入站面，列于本表以便对照）。判据只有一条：**这里有 IO，或者这里有可替换的实现**。
- 纯逻辑（信封解析、引擎循环、协作状态派生、提示词渲染）**刻意不抽象**。

| 端口 | 职责 | 适配层实现 |
| --- | --- | --- |
| `Chat` | 一次模型会话：收消息列表（可带工具声明）回 `Completion`（正文 + 结束原因 + 原生工具调用）；`on` 逐片回调，返回 `false` 即要求中止 | `HttpChat`（测试 `FakeChat`） |
| `ChatGateway` | 按**已解析**的通道建会话（核心 / 成员）；**不选择**模型、不回落假模型；探测一条通道支不支持原生工具调用 | `HttpGateway`（测试专用 `DemoGateway`） |
| `ModelCatalog` | 按端点与密钥列出一条通道当前可用的模型名 | `HttpModelCatalog` |
| `SettingsStore` | 登记处持久化（providers / models / settings / agents 四个 yaml） | `YamlSettingsStore` |
| `ModuleSource` | 模块清单来源（扫描 `modules/`） | `modules/detail::FsModules` |
| `ModuleStateStore` | 模块与服务的启停覆盖落盘（`.home/modules.yaml`） | `modules/detail::YamlModuleState` |
| `PackageSource` | 运行包库来源（扫描依赖文件夹 `runtimes/`） | `vm/detail::FsPackages` |
| `SourceDigest` | 模块源指纹：构建命令、构建期能力、网络开关与全部源文件内容 → 一段稳定指纹（缓存命中的判据） | `BuildStore`（`builds/detail::fingerprint`） |
| `BuildStates` | 构建状态：某模块上次构建的目标与指纹；每次构建的私有家目录落点 | `BuildStore`（同上） |
| `BuildCache` | 构建缓存卷：卷落点、同一虚拟根的构建锁、待清理账、模块用量元数据、挂载/对账/源同步/差异回写的脚本文本 | `CacheOnDisk`（`builds/detail::cache`） |
| `CacheGuard` | 缓存卷的独占凭据：拿到即独占该虚拟根的构建，`drop` 释放 | 由 `CacheOnDisk::lock` 产出（`builds/detail::cache`） |
| `SpaceSurvey` | 可回收空间的盘点：时间戳、宿主总量 / 可用量、各分类（构建卷 / 下载缓存 / 可写层 / 日志 / 用户数据）与条目；只读 | `DiskSurvey`（`storage/detail::survey`） |
| `SpaceReclaim` | 回收动作：跨进程回收锁（凭据 drop 释放）、按分类删条目、按模块排队清理、构建卷压缩、上次回收摘要读写、构建占用判定 | `DiskReclaim`（`storage/detail::reclaim`） |
| `GuestLiveness` | 有没有 guest 在跑（删除可写层与还原虚拟根的前置判据） | `ProcessGuests`（`storage/detail::guests`，读进程台账） |
| `Fetcher` | 取数：按 URL 取文本、算文件 sha256、按候选源下载（测速选源 + 断点续传 + 逐行回执） | `HttpFetch`（`provision/detail::fetch`） |
| `HostFs` | 依赖落盘的宿主侧原语：存在性、建目录、原子写、删除、执行位 | `RealHost`（`provision/detail::host`） |
| `Ledger` | 依赖台账（`.vm/state/deps.json`）：快照、在场判定、guest 画像、装好登记与清旧、工具链生效登记 | `FileLedger`（`provision/detail::ledger`） |
| `Installers` | 按 `kind` 落盘：归档解包、单文件、AppImage、工具链包装、guest 磁盘与 `guest.yaml` 画像 | `HostInstallers`（`provision/detail::install`） |
| `GuestImage` | `guest-rootfs` 的 ext4 镜像构造（选 deb、内核 / QGA / initrd、属性回填） | `Ext4Images`（`provision/detail::guest_image`） |
| `Procs` | 进程台账条目与 `guest` 是否在跑（安装期 `reset_guest` 的前置判据） | `BookProcs`（`provision/detail::procs`） |
| `VmEngine` / `GuestVm` / `GuestChannel` | 虚拟机档的引擎：按规格拉起一台 guest、在它里面执行、销毁；通道是在 guest 里执行的入口 | `vm/detail::QemuEngine` + `QemuHandle`（QEMU + QGA） |
| `ShareBackend` | 宿主↔guest 的共享：声明能落哪些引擎 / guest，产出引擎参数、netdev 附加选项与 guest 端挂载命令，并可持有外部资源（`ShareGuard`） | `vm/detail::VirtioFsShare`（vhost-user-fs + virtiofsd）/ `NinePShare`（9p）/ `SmbShare`（用户态 SMB helper + guestfwd，Windows 实时共享）/ `CopyShare`（QGA 复制，仅开发/应急）/ `NoneShare`（内部无共享，供给编排） |
| `Workdirs` | 一次工作的共享区、各 agent 沙箱、版本库目录、文件清单、用量与寻址根 | `FsWorkspace` |
| `WorkStore` | 共享区版本库的落盘：文件原语 + 内容寻址对象 + 提交记录 + 每个 agent 的拉取基线（并拒绝符号链接逃逸） | `FsWorkStore` |
| `SysIo` | 内置文件工具的读写机制（读严格 UTF-8、非法字节如实标注；写一律 UTF-8） | `FsSysIo` |
| `HistoryStore` | 会话历史：顶层会话一个目录，子会话落在**父会话目录内部**（meta + 事件流水） | `FsHistory` |
| `PromptSource` | 提示词册加载（`prompts/`） | `YamlPrompts` |
| `SystoolsSource` | 工具总表、角色表与保留名表的加载（`systools/`） | `YamlSystools` |
| `ProxyHost` | 核心代理工具的外部动作（清单 / 建会话 / 转达 / 观察 / 消息倒查 / 生命周期） | `ProxyBridge`（队列桥；测试 `FakeProxyHost`） |
| `ProcessRunner` | 外部进程执行（围栏安装、拉起、stdin 送参、超时/取消杀树、head/head+tail 截断、一次性 shell）；**kernel 共享机制端口** | `ProcTools`（守门进程 = 本程序的 `--fence-run` 模式） |
| `SessionHost`（产出 `Session`） | 起一条**外部进程会话**（一次性子进程 / 常驻服务的按行协议）：给规格拿一个可收发、可杀、可判活的会话；**kernel 共享机制端口** | `ProcessSessions`（`kernel/detail::session`） |
| `EnvelopeRepair` | 手写信封不合法时的**无歧义**补救（改了字段含义就是错；拿不准就不修） | `UnambiguousRepair` |
| `ServiceAdapter` | 一个常驻服务的协议适配器：拉起、发现操作、调用、关闭（按行协议走 kernel 的 `SessionHost`） | `residents/detail::McpAdapter`（MCP stdio） |
| `ServiceInstance` | 一个已拉起的常驻服务实例：调用操作、关闭、判活、取回被刷新的操作清单 | `McpInstance`（`residents/detail::mcp`） |
| `SecretStore` | 模块隐秘字段的落盘（`.home/` 的 `secrets.yaml`；值不进提示词、转录与日志） | `secrets/detail::YamlSecrets` |
| `FenceHost` | 围栏授权的释放：`release_by_lease` 按会话租约撤 ACE/删 profile（会话改配、删除），`release_call` 服务一次性调用（撤 ACE、删 profile 与临时目录）；**kernel 共享机制端口** | `confine::FenceHostAdapter`（本平台无该机制时为空操作） |
| `Log` / `HostProbe` / `PathCodec` / `ProcessTexts` | 运行日志（三级）/ 宿主能力探测（**只问事实**：路径存在性、PATH 上的可执行文件、本机虚拟化能力）/ 模型侧路径 ↔ 宿主路径的翻译 / 外部进程回执的收尾文案 | `FileLog` / `HostProbeAdapter` / `vm` 的路径桥 / `kernel::detail::process::CodeProcessTexts`（测试 `NoopLog` / `FixedProbe`） |
| `ToolHandler` | **一类工具的执行者**（按名字认领）：内置、模块与核心自有工具因此走**同一条派发路径** | 各能力注入（工具能力管内置与模块；conductor 注入代理工具那个） |
| `AskUser` | **请用户裁决的提问端口**：需要用户裁决的机制（工具执行层、围栏）推一条问题并**阻塞**等一个选项 id——走会话的**统一裁决通道**（不是第二条） | 会话侧实现（conductor 的 `SessionAsk`：卡片进那一条队、`answer_card` 作答；构不出可用选项时停会话 + 落警告） |

- 端口只有一个持有者：**定义它的那个能力的 `service.rs`**（R12）。**例外**：`kernel` 的机制端口（`Log` / `HostProbe` / `ToolHandler` / `AskUser` / `ProcessRunner` / `FenceHost` / `PathCodec` / `ProcessTexts` / `SessionHost`）全项目共享。
- 新增端口前先问一句：**这是 IO 或可替换点吗**？不是就别加 trait。

## 三、单元地图与入站契约

- 这两块是查阅型细则，各只有一份：

- 逐个文件讲某个单元（`capabilities/` 含 `conductor/`、`kernel/`、`entry/`、`presentation/`）各干什么：各单元自己的 `docs/<单元>/unit-map.md`（单元划分与完整路由见 [AGENTS.md](AGENTS.md)「文档路由」）。
- 呈现层入站契约与机器可读的 HTTP 路由目录：[docs/presentation/contracts.md](docs/presentation/contracts.md)
- （**路由表由 `src/tests/routes.rs` 直接读本文比对**，对不上就是测试失败）。
- 系统工具总表、角色表与"谁能用哪些工具"：[docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md)
- 协作如何从讨论走到交付（审查关卡、任务链、子会话、验收）：[docs/collab/task-chain.md](docs/collab/task-chain.md)
- 提示词册（`prompts/`）的结构与键清单：[docs/prompt/prompts.md](docs/prompt/prompts.md)
- 业务边界判据与硬要求：见 §九。

## 四、运行日志（`Log` 端口）

- 业务只调用 `Log`（`info` / `warn` / `error`），机制与落盘在 `src/kernel/detail/file_log.rs`；组合根注入唯一实例，测试用 `NoopLog`。
- **关键节点必须埋点**：通道降级、HTTP 失败、会话动作失败、装配失败、工具执行异常——出问题时**看日志定因**，不靠推理猜（`src/kernel/detail/file_log.rs`）。
- 按运行分文件、目录与不入库规则见 [docs/kernel/README.md](docs/kernel/README.md) 的「运行日志」。

## 五、提示词册（`prompts/`）

- **所有发给 LLM 的指令、身份、协议、环境提示词一律写入 `prompts/`**，禁止硬编码进代码；改文案只改册子。占位符 `{{key}}` 在调用现场替换，缺文件 / 缺键 / 缺变量**报错暴露**，禁止静默兜底（`src/capabilities/prompt/domain/prompt.rs`）。
- **册子只由提示词能力持有一次**（`Arc` 共享）；别的能力**按名字取段**（`Prompt::text` / `render` + `Segment`），不点字段路径；"哪个回合发哪几段"的组装留在各业务。路径占位符由协调业务替换成真实根目录后才交给 agent。
- **不进册子的两类**：行身份是结构化字段（认 `LineView` 的 `speaker` / `verb` / `kind`，不认文案）；只给用户看的呈现层文案（各类 `Notice`、工具轨迹行的成败字样、面向界面 / CLI 的 `Err`）。
- 运行时回执**按产生它的模块归属**：判断标准是"谁做决定谁拥有文案"，不是"进上下文就进册子"。
- 键清单与册子规则：[docs/prompt/prompts.md](docs/prompt/prompts.md)。

## 六、状态与落盘契约

- **每个状态只有一个家**：会话状态归 `session`、构建状态归 `builds`、空间台账归 `storage`；别的单元读写只能经该能力的 `api`，不直接读对方文件（R1 / R4，判据在 `tests/gates/structure/dependency.js`）。
- **跨能力共享的事实类型放 `kernel`**（R6）；业务自己的入参 / 出参留在本能力 `api`。
- **转录即状态**：会话内容 = 按 `rewind` 标记回放出的活动窗口；内存与落盘不一致时**以流水为准**。
- **能重建的不落盘**：视图、索引、账本缓存与执行计划是派生物，重启后由真相重建；写盘只写派生不出来的事实。
- **落盘路径相对根派生**：一律用 `PathBuf` 组件拼接、不写机器绝对路径（R9，例见 `src/capabilities/builds/detail/cache.rs`）；含机器路径的内容不进仓库。
- **改布局同次改三处**：`docs/<单元>/unit-map.md`、本文对应节与 [AGENTS.md](AGENTS.md) 的「文档路由」（R8）。
- **权限与围栏**：策略在 `src/capabilities/tools/domain/fence.rs`，机制与落点在 `src/kernel/detail/confine/`；规则见 [docs/permission/README.md](docs/permission/README.md)，平台差异见 [docs/kernel/README.md](docs/kernel/README.md) 的「围栏落点：必要与可选」。

改哪个方向，读哪一份（细则只有一处权威）：

| 要改什么 | 权威 |
| --- | --- |
| 会话状态：目录布局、转录与行结构、会话参数、回档与压缩、思考强度 | [docs/session/session-model.md](docs/session/session-model.md) |
| 共享区与版本化工作区：`work_pull` / `work_commit`、三方与冲突解决、按行锚回档 | [docs/workspace/README.md](docs/workspace/README.md) |
| 落盘机制、进程台账与回收、围栏落点与守门协议、运行日志 | [docs/kernel/README.md](docs/kernel/README.md) |
| 权限策略：三维度 reach × effect × decision、四档 profile、围栏派生与裁决 | [docs/permission/README.md](docs/permission/README.md) |
| 工具总表、角色表与目录保留名 | [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md) |
| 提示词册的键清单与规则 | [docs/prompt/prompts.md](docs/prompt/prompts.md) |
| 登记处四份 yaml（providers / models / settings / agents） | [REGISTRY_SPEC.md](REGISTRY_SPEC.md) |
| 运行包契约与执行档位（packages / exec） | [RUNTIME_SPEC.md](RUNTIME_SPEC.md) |
| 模块清单、模块产物与模块运行包 | [MODULE_SPEC.md](MODULE_SPEC.md) |
| 业务边界判据、硬要求 R1–R13 与门禁 | 本文 §九 |

## 七、可测性（架构约束）

- 测试的层级、替身语义、端口契约矩阵、质量门禁、缺口账与执行入口全部由 [TESTING.md](TESTING.md) 与 `docs/testing/` 规定；本节只列架构硬约束。

- 任意需要 IO 或存在可替换实现的机制必须通过端口注入；能力不直接依赖真实模型、网络、文件系统、时钟、随机数或外部进程（`src/kernel/ports.rs`）。
- 纯逻辑不为测试强行增加 trait；端口只放在真实边界和确有替换价值的点上。
- 端口的输入、输出、错误、取消、超时、重复调用与资源清理语义是**架构契约**：生产适配器与测试替身遵守同一份。
- 端口不为了测试暴露生产实现的内部状态；要观察交互就用替身的记录能力或公开行为。
- 组合根测试用的内存装配（`InMemory*`、`ScriptGateway`、`NoopLog` 等）集中在 `src/tests/doubles.rs`；新增替身用能表达职责的名字。
- **分层与依赖方向由 T0 结构审查机器判定**（豁免清单 `tests/dependency-baseline.json`，当前为空 = 零豁免，条目一旦不再成立即失败）。
- 质量门禁（格式、编译、Clippy、依赖重复、测试结构冗余）与业务测试是两类事实，分别记录，互不抵消。
- 代码冗余检查不改变分层与端口设计，也不以增加 trait、包装层或测试用例为目标。

## 八、跨平台机制

- 路径一律用 `PathBuf` / `Path` 组件拼接：**禁止把 `/` 或 `\` 写进字符串再拼**。
- **对外**（提示词、工具参数、回执、API）一律用 `/` 书写形式（Windows 反斜杠在 JSON 字符串里是非法转义）；由 `kernel::path::slash` 统一给出。
- 编码：读严格 UTF-8、非法字节如实标注（**不猜编码**）；写一律 UTF-8；为工具子进程强制 UTF-8 环境（`src/capabilities/tools/detail/sys_io.rs`）。
- 不假设平台：不写死盘符；模块工具的命令由模块作者声明，系统 `shell` 按平台选 sh / PowerShell。
- 围栏按平台给不同机制（`detail/confine/` 一个平台一个文件）；某平台没有接入的部分**就是没有**——文档与界面照实说。

## 九、业务边界与硬要求

> 这一节回答两个问题：**什么该独立成一个业务**，以及**所有业务必须守的硬要求**。
> 依赖方向由 T0 结构审查按 §九.7 的口径**机器判定**；当前基线为空 = 零豁免。

### 九.1 判据（两条同时成立，才配独立成一个业务）

1. **有自己的用例**：一组有领域词、有**自有协议**、有**用户可见后果**的行为。**它说的概念属于某个参与方 → 就归那个参与方**；不属于任何参与方的概念（名单、任务链）才自成业务。
2. **能指名 ≥2 个调用方**：不抽出来，这两个（或更多）能力就会各写一遍。**已被证实的重复是最强证据**；**只有一个调用方的编排不抽**，留在它自己的 `service.rs`。

- **状态不参与这条判据**——它只在 §九.2 回答「这块状态归谁」。

### 九.2 状态归属

- 有参与方拥有它的不变式 → 归**那个参与方**（它的 `service.rs` 写，别人只经 `api` 读改）；
- 没有参与方拥有、且 ≥2 个参与方共同依赖 → 归**协调业务**（`conductor` 的会话在世表、命令队列与运行态）；
- 只被一个参与方用、自己又没有不变式 → 它是**派生值**，留在 owner 的 `domain/`，现算。

- **协调业务可以持状态**；唯一的限制是：**它的状态只能它自己写，别人的状态只能经 `api` 拿**（R4）（`src/capabilities/conductor/service/mod.rs`）。

### 九.3 三分法（正交：只决定它持有什么、依赖谁）

| 类型 | 判据 | 归属与形态 |
| --- | --- | --- |
| **协调型业务** | 跨参与方的编排：有自有协议 + 用户可见后果 + 领域词 API | 独立业务；**可持状态**；向下调参与方 `api`，**参与方不得反向调它**（`conductor`） |
| **领域型业务** | 说的是自己的数据 / 概念，规则与状态自成一体 | 独立业务；**只持自己的状态**；有 IO 或可替换点才有 `ports.rs` / `service.rs`（`taskchain` 就是没有端口、没有 `service` 的领域型） |
| **机制型（`kernel`）** | 没有领域语义 | `kernel/`：**领域词测试**——它不需要知道什么是回合 / 回复 / 工具执行；有机制状态与机制端口，只被依赖 |

### 九.4 反例：什么不配切成业务

- **纯机制**（线程、锁、定时、文件句柄、HTTP 客户端）→ `kernel` 或所属能力的 `detail/`；
- **纯逻辑 / 派生**（解析器、状态派生、提示词渲染）→ 所属业务的 `domain/`，**不加 trait**；
- **纯交付**（HTTP 路由、终端渲染）→ `presentation/{cli,web}/`，**不是能力**；
- **只服务单一调用方的脚本 / 编排** → 它自己的 `service.rs`；
- **代理方法族（`core_proxy`）** → 留在 `conductor`，不升格：没有自有落盘真相（幂等账在内存，授予读会话 `meta.delegation`，转录与工作区分别归 `session` / `workspace`），§九.1 第 2 条也不成立——唯一调用方是 `conductor` 自己；升格还会成环，代理工具的执行面必须回到核心线程改核心状态（`ProxyBridge` → `ConductorHandle::call`），而 `conductor` 又要调它建代理会话（判据见 `src/capabilities/conductor/ports.rs` 的 `ProxyHost` 与 `src/capabilities/conductor/service/proxy.rs`）；
- **DTO 中转站**（把两个能力的类型拼一起的适配业务）→ 新的水平层，禁止（`node run-tests.js` 的 T0 结构审查）。

### 九.5 粒度上限

- 一个能力的实现文件（`service/**` / `domain/**` / `detail/**`）≤ **800 行**（`conductor/service/` 与 `collab/service/` 按方法族分块）——这是**软线**：超线只进 `[big]` 回执（`node run-tests.js` 打印行数 / 增量 / IO 数 / impl 数 / 文件头目的槽，**不改变成败**，行数是气味、依赖才是规则）；
- 唯一硬线是**新建**文件的 **1200 行**：新文件没有历史包袱，超线说明职责没拆，门禁直接失败；存量文件永不为行数失败，只按能力重构逐步消账；
- 一个能力的 `api.rs` ≤ **20 个方法**——按「一个调用角色的面」计数；`registry` 是已知的宽面，**它仍是一个能力**（判据是 §九.1，不是方法数）；
- 测试文件按 R10 的上限（2000 行）；
- 平台后端按**平台目录**切，再按关注点分（如 Windows 的 `acl` / `record` / `container`）。

### 九.6 硬要求清单

| 编号 | 要求 |
| --- | --- |
| **R1** | 业务之间**只经对方的 `api` 交流**（trait + DTO）。不准引 `ports` / `domain` / `detail`；不准给别的能力的类型写 `impl`；`::detail` 只有入口层能碰 |
| **R2** | 依赖图**无环**，由 T0 结构审查机器判定 |
| **R3** | DIP **只画在 IO 或可替换点上**；纯逻辑刻意不抽象 |
| **R4** | **状态所有权排他**：一块状态只有一个能力写，别人只经 `api` 读改 |
| **R5** | **并发模型不变式**：单线程命令队列 + 能力间同步调用，**核心状态不加锁** |
| **R6** | **共享事实类型只属于 `kernel`**，禁止各业务复制 DTO |
| **R7** | **不留兼容层**：项目是 GREEN FIELD，**只维护当前唯一实现**——旧实现直接删，不留转发壳或历史说明 <!-- hygiene-ignore --> |
| **R8** | **文档同步**：改结构必须同一次改 `ARCHITECTURE.md` / `docs/<单元>/unit-map.md` / 相关细则 / `AGENTS.md` 路由表 |
| **R9** | **跨平台与路径**：一律 `PathBuf` 组件拼接；对外用 `/`；不假设平台 |
| **R10** | **测试跟着业务分区走**：`capabilities/<名称>/` ↔ `src/tests/<名称>.rs`（用例多的按域分成子模块），单文件 ≤ 2000 行 |
| **R11** | **测试入口 = 生产入口**：禁止 `#[cfg(test)]` 专用语义入口 |
| **R12** | **端口只有一个持有者**：定义它的那个能力的 `service.rs`；别人只拿 `api` 面。**例外**：`kernel` 的机制端口是全项目共享的机制接口 |
| **R13** | **跨能力的同形重复不许存在**：要么收进唯一所有者，要么独立成一个业务（见「判据」第 2 条） |

### 九.7 依赖方向门禁与基线

- `node run-tests.js` 的 T0 结构审查按下列判据机器判定（规则在 `tests/gates/structure/dependency.js` 与 `tests/gates/structure/capability.js`，逐条清单用 `node run-tests.js --list`；豁免清单分别在 `tests/dependency-baseline.json` 与 `tests/structure-baseline.json`）：

- **apiOnly**：跨能力引用必须以 `::api` 结尾；任何非入口层引用别人的 `ports` 都算违规；
- **apiPorts** / **domainPorts**：`api` 与 `domain` 都不引端口；
- **foreignImpl**：不得给别的能力的类型写 `impl`（`node run-tests.js`）；
- **coreCycles**：能力节点图无环；
- **internal**（能力内部）：`service` 不引本能力 `detail` 或 `kernel::detail`；`service` 不直接做 IO（`std::fs` / `Command` / 网络）；同能力的方法族之间不成环；
- **reverse** / **presentation**：能力不反向依赖入口层或呈现层；
- **channelCross**：呈现层的两个渠道（`cli` 与 `web`）互不依赖；
- **unitMap**：各单元的 `unit-map.md` 合起来与磁盘**双向一致**（结构审查）；**文档链接**：代码注释里的 `docs/**.md` 引用由结构审查判，文档之间的 md 链接与根 `*.md` 路由表由**文档审查**判（本地全量入口的「T0 结构审查」步骤跑全部门禁、含文档审查；CI 拆成 `docs` / `structure` 两个 job，`node run-tests.js --audit=docs` / `--audit=structure`）；
- **commentContract**：全仓 `.rs` 的文件头四槽（目的 / 管 / 不管 / 联动）、`pub` 项的条目槽（目的 / 参数 / 返回 / 错误 / 约束）与行内注释限量；存量（`文件 × 规则` → 计数）记在 `tests/comment-baseline.json`，门禁只减不增：新增与应销账都硬失败（见 §十）。

- **当前基线为空（零豁免）**：任一判据不成立即报错；豁免条目一旦不再成立，门禁报「基线豁免已过期」强制销账。

## 十、注释契约

> 代码注释只描述**当前**情况：这份文件是什么、管什么、不管什么、和谁联动。
> **格式**由 T0 的 `commentContract` 判据机器判定（棘轮 + 销账，见十.4）；**内容**层面的卫生（悬挂引用这类）另由 `run-hygiene.js` 报，不进 T0。

- 开发历史归 git，待办与路线归 `tests/gaps.yaml`，行为约束归用例——注释里复述它们只会多一份会漂的副本。

- **槽名一律中文 + 全角冒号**：它们是仓库自己的词（`管` / `不管` 就是 `docs/<单元>/README.md` 的节词），译成英文等于给同一个概念造第二份词汇；全角冒号在代码注释里是唯一形态，不必兼容半角。

### 十.1 文件头（`//!`）

- 每个 `.rs` 文件开头第一段 `//!` 就是文件头（它前面只允许 `#![…]` 内部属性与空行）。**四槽、固定顺序、槽名精确**（全角冒号）：

```rust
//! 目的：这份文件是什么（名词性一句话，不写"为什么现在做"）。
//! 管：职责内侧——它负责什么。
//! 不管：职责外侧——明确不负责什么，尤其是看起来该在这儿、其实在别处的。
//! 联动：契约关系——被谁消费、依赖谁、机制/策略落在哪（用仓库相对路径）。
```

- 槽的值可以续行（续行在 `//!` 后缩进两格）；槽外**不允许**漂浮的散文段。
- 引用只写**文件 + 标题**或顶层节号（`ARCHITECTURE.md` 的 §一 / §九）：不写深层节号，不写缺口 id——那是工作项，会随关账消失。

### 十.2 条目（`///`，只强制 `pub` 项）

- `pub` 项（含 `pub(crate)`、含 `pub` 字段）的文档块：**首行必须是 `目的：…`**，之后每一行**必须是已知槽**或缩进续行——槽外不许有行，散文没有地方藏（`tests/comment-baseline.json`）：

```rust
/// 目的：它是什么（一句话）。
/// 参数：`name` 是……（只有含义不明显时才写，一行一个参数）。
/// 返回：……（有返回值才写）。
/// 错误：……（会失败才写）。
/// 约束：调用方必须知道的**当前**事实（不写来历）。
```

- 槽都是可选的，只有 `目的` 必填；不需要别的槽就只有一行。
- 条目层不复用文件头的 `管` / `不管`：那两槽说的是**职责边界**（文件是容器），函数没有这回事。
- 非 `pub` 项与测试的文档块这轮不强制。

### 十.3 行内（`//`）

- 行内注释只回答"就地为什么"，而且必须**紧贴**它解释的那一行（`node run-hygiene.js`）：

- 连续 `//` ≤ 2 行、单行 ≤ 150 字；中间不许空行，后面必须紧跟代码（成段说明属于文件头或 `///`）；
- 契约、边界、机制位置这类信息只能进文件头或条目的 `///`；
- 禁块注释 `/* */`（判定按词法：字符串、glob、散文里的 `/*` 不算）；
- 历史措辞（曾经 / 原来 / 旧版 / 旧实现 / 改成 / 先是 / 后来 / 以前 / 与旧逻辑）与显式待办标记（TODO / FIXME / 待补）一律不许出现（`node run-hygiene.js`）。<!-- hygiene-ignore -->
- 词表只收**叙事性**词：领域词（`临时 agent`、`后续片段`、`本地遗留`、`暂未暴露`）不算——它们描述的是当前状态。
- 全仓同类措辞由 `run-hygiene.js` 的语义卫生只报（见 `docs/testing/quality-isolation.md`），不进门禁。

### 十.4 强制方式（棘轮 + 销账）

- `commentContract` 对**全仓**都判，存量放在 `tests/comment-baseline.json` 里（键是 `文件 × 规则`，值是**计数**——不记行号，行号会漂）。棘轮**只减不增**，T0 结构审查报两种：

- **新增**：某 `文件 × 规则` 的计数比快照多 → 硬失败。快照里没有的文件从 0 起算，所以**新文件零容忍**（`tests/comment-baseline.json`）；
- 老文件同类变多同样算新增（不许把新违规藏进老文件的同类存量里）；
- **应销账**：某计数比快照少（含整条消失）→ 同样硬失败（本仓基线的既有规矩：条目一旦不再成立必须销账，见 §九.7 的依赖基线）（`tests/comment-baseline.json`）。

- 收紧快照：`node run-hygiene.js --tighten`——**卫生工具是唯一会写它的地方**，且**只能往下**：出现任何新增就拒绝写盘（`tests/comment-baseline.json`）（先违规、再收紧等于把违规洗成存量）。门禁只读、不做任何写动作。收紧后连同本次改动一起提交，diff 里就是减少的那些计数。

## 检查
- 分层与依赖方向 → T0 结构审查（`node run-tests.js`）的 apiOnly / apiPorts / domainPorts / foreignImpl / coreCycles / reverse / presentation 判据（`tests/gates/structure/dependency.js`；清单 `node run-tests.js --list`）。
- 注释契约 → `commentContract` 判据（`tests/gates/structure/comments.js`，存量快照 `tests/comment-baseline.json`，只减不增）。
- 文档引用与单元地图 → 结构审查的 `unitMap` 与代码注释文档引用（`tests/gates/structure/map.js`）、文档审查的 md 链接与根文档路由（`tests/gates/docs/`）。

## 路由
- 项目方向：[PHILOSOPHY.md](PHILOSOPHY.md)
- 产品行为：[PRODUCT.md](PRODUCT.md)
- 模块契约：[MODULE_SPEC.md](MODULE_SPEC.md)
- 运行包契约：[RUNTIME_SPEC.md](RUNTIME_SPEC.md)
- 登记处契约：[REGISTRY_SPEC.md](REGISTRY_SPEC.md)
- 测试规范门户：[TESTING.md](TESTING.md)
- 仓库协作规则：[AGENTS.md](AGENTS.md)
- 会话与落盘布局：[docs/session/session-model.md](docs/session/session-model.md)
- 提示词键表：[docs/prompt/prompts.md](docs/prompt/prompts.md)
- 工具与角色：[docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md)
- 任务链：[docs/collab/task-chain.md](docs/collab/task-chain.md)
- 呈现层契约：[docs/presentation/contracts.md](docs/presentation/contracts.md)
- 围栏落点：[docs/kernel/README.md](docs/kernel/README.md)
