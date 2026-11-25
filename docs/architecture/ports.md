<!-- docs-template: spec -->
# 端口：只画在 IO 与可替换点上
> 定位：端口的唯一权威：什么该是端口、有哪些端口、谁是持有者。
> 效力：规范｜核对：2026-11-25｜依据：`src/kernel/ports.rs`、各单元 `ports.rs`

## 适用与判据
- 新增或改造端口前读本文；判据只有一条：**这里有 IO，或者这里有可替换的实现**（`tests/gates/structure/capability.js`）。

## 一、判据与归属
- 端口 = 各单元 `ports.rs` 与 `src/kernel/ports.rs` 里的 trait（`Chat` 的定义在 `src/units/llm/api.rs`、`ProxyHost` 的定义在 `src/units/proxy/api.rs`，因为它们同时是能力的入站面）。
- 纯逻辑（信封解析、引擎循环、协作状态派生、提示词渲染）**刻意不抽象**：不是 IO、没有可替换实现，就不加 trait。
- 端口只有一个持有者：**定义它的那个单元的 `service.rs`**（R12）。**例外**：`kernel` 的机制端口（`Log` / `HostProbe` / `ToolHandler` / `AskUser` / `ProcessRunner` / `FenceHost` / `PathCodec` / `SessionHost`）全项目共享；`TurnHost` 由 `turn` 声明（`src/units/turn/turn/api.rs`）、由 `runtime` 实现（`src/units/runtime/service/handle.rs`）——业务声明它需要宿主代做什么，最顶层（`runtime`）来实现。
- 新增端口前先问一句：**这是 IO 或可替换点吗**？不是就别加 trait（`tests/gates/structure/capability.js`）。
- 端口的输入、输出、错误、取消、超时、重复调用与资源清理语义是**架构契约**：生产适配器与测试替身遵守同一份（R3、R11）。

## 二、端口清单
- 下表只记「有哪些端口、各自承诺什么」；谁实现它、落在哪个 `detail/`，由各单元 `docs/<单元>/unit-map.md` 给。

| 端口 | 职责 |
| --- | --- |
| `Chat` | 一次模型会话：收消息列表（可带工具声明）回 `Completion`（正文 + 结束原因 + 原生工具调用）；`on` 逐片回调，返回 `false` 即要求中止 |
| `ChatGateway` | 按**已解析**的通道建会话（核心 / 成员）；**不选择**模型、不回落假模型；探测一条通道支不支持原生工具调用 |
| `ModelCatalog` | 按端点与密钥列出一条通道当前可用的模型名 |
| `SettingsStore` | 登记处持久化（providers / models / settings / agents 四个 yaml） |
| `ModuleSource` | 模块清单来源（扫描 `modules/`） |
| `ModuleStateStore` | 模块与服务的启停覆盖落盘（`data/.home/modules.yaml`） |
| `PackageSource` | 运行包库来源（扫描依赖文件夹 `data/runtimes/`） |
| `SourceDigest` | 模块源指纹：构建命令、构建期能力、网络开关与全部源文件内容 → 一段稳定指纹（缓存命中的判据） |
| `BuildStates` | 构建状态：某模块上次构建的目标与指纹；每次构建的私有家目录落点 |
| `BuildCache` | 构建缓存卷：卷落点、同一虚拟根的构建锁、待清理账、模块用量元数据、挂载/对账/源同步/差异回写的脚本文本 |
| `CacheGuard` | 缓存卷的独占凭据：拿到即独占该虚拟根的构建，`drop` 释放 |
| `SpaceSurvey` | 可回收空间的盘点：时间戳、宿主总量 / 可用量、各分类（构建卷 / 下载缓存 / 可写层 / 日志 / 用户数据）与条目；只读 |
| `SpaceReclaim` | 回收动作：跨进程回收锁（凭据 drop 释放）、按分类删条目、按模块排队清理、构建卷压缩、上次回收摘要读写、构建占用判定 |
| `GuestLiveness` | 有没有 guest 在跑（删除可写层与还原虚拟根的前置判据） |
| `Fetcher` | 取数：按 URL 取文本、算文件 sha256、按候选源下载（测速选源 + 断点续传 + 逐行回执） |
| `HostFs` | 依赖落盘的宿主侧原语：存在性、建目录、原子写、删除、执行位 |
| `Ledger` | 依赖台账（`data/.vm/state/deps.json`）：快照、在场判定、guest 画像、装好登记与清旧、工具链生效登记 |
| `Installers` | 按 `kind` 落盘：归档解包、单文件、AppImage、工具链包装、guest 磁盘与 `guest.yaml` 画像 |
| `GuestImage` | `guest-rootfs` 的 ext4 镜像构造（选 deb、内核 / QGA / initrd、属性回填） |
| `Procs` | 进程台账条目与 `guest` 是否在跑（安装期 `reset_guest` 的前置判据） |
| `VmEngine` / `GuestVm` / `GuestChannel` | 虚拟机档的引擎：按规格拉起一台 guest、在它里面执行、销毁；通道是在 guest 里执行的入口 |
| `ShareBackend` | 宿主↔guest 的共享：声明能落哪些引擎 / guest，产出引擎参数、netdev 附加选项与 guest 端挂载命令，并可持有外部资源（`ShareGuard`） |
| `Workdirs` | 一次工作的共享区、各 agent 沙箱、版本库目录、文件清单、用量与寻址根 |
| `WorkStore` | 共享区版本库的落盘：文件原语 + 内容寻址对象 + 提交记录 + 每个 agent 的拉取基线（并拒绝符号链接逃逸） |
| `SysIo` | 内置文件工具的读写机制（读严格 UTF-8、非法字节如实标注；写一律 UTF-8） |
| `HistoryStore` | 会话历史：顶层会话一个目录，子会话落在**父会话目录内部**（meta + 事件流水） |
| `PromptSource` | 提示词册加载（`prompts/`） |
| `SystoolsSource` | 工具总表与保留名表的加载（`systools/`） |
| `ProxyHost` | 核心代理工具的外部动作（清单 / 建会话 / 转达 / 观察 / 消息倒查 / 生命周期） |
| `ProcessRunner` | 外部进程执行（围栏安装、拉起、stdin 送参、超时/取消杀树、一次性 shell）；**kernel 共享机制端口** |
| `SessionHost`（产出 `Session`） | 起一条**外部进程会话**（一次性子进程 / 常驻服务的按行协议）：给规格拿一个可收发、可杀、可判活的会话；**kernel 共享机制端口** |
| `EnvelopeRepair` | 手写信封不合法时的**无歧义**补救（改了字段含义就是错；拿不准就不修） |
| `ServiceAdapter` | 一个常驻服务的协议适配器：拉起、发现操作、调用、关闭（按行协议走 kernel 的 `SessionHost`） |
| `ServiceInstance` | 一个已拉起的常驻服务实例：调用操作、关闭、判活、取回被刷新的操作清单 |
| `SecretStore` | 模块隐秘字段的落盘（`data/.home/` 的 `secrets.yaml`；值不进提示词、转录与日志） |
| `FenceHost` | 围栏授权的释放：`release_by_lease` 按会话租约撤 ACE/删 profile（会话改配、删除），`release_call` 服务一次性调用（撤 ACE、删 profile 与临时目录）；**kernel 共享机制端口** |
| `Log` / `HostProbe` / `PathCodec` | 运行日志（三级）/ 宿主能力探测（**只问事实**：路径存在性、PATH 上的可执行文件、本机虚拟化能力）/ 模型侧路径 ↔ 宿主路径的翻译 |
| `ToolHandler` | **一类工具的执行者**（按名字认领）：内置、模块与核心自有工具因此走**同一条派发路径** |
| `AskUser` | **请用户裁决的提问端口**：需要用户裁决的机制（工具执行层、围栏）推一条问题并**阻塞**等一个选项 id——走会话的**统一裁决通道**（不是第二条） |
| `TurnHost` | **回合推进的宿主契约**：业务在自己的工作线程上需要宿主在核心线程代做的事——取放会话状态、事件台、guest 起停、裁决队与提问端口、生成登记、线程点火、回答裁决卡 |

## 检查
- 端口位置与归属 → `tests/gates/structure/capability.js`（`internal`：`service` 不引本单元 `detail` 与 `kernel::detail`、不直接做 IO）、`dependency.js`（跨单元只走 `api`，端口只有持有者的 `service` 定义）。
- 跨边界只传值 → `tests/gates/structure/values.js`。

## 路由
- 分层与依赖方向：[layers.md](layers.md)
- 业务边界判据与硬要求：[boundaries.md](boundaries.md)
- 判定与围栏策略：[../permission/README.md](../permission/README.md)
- 机制端口的落点：[../kernel/README.md](../kernel/README.md)
