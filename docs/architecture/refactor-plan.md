# 重构方案：按业务能力垂直切分

> 本文是**重构的迁移账**：目标形态、业务边界判据、能力清单与迁移批次。
> **未实施的条目一律记「未开始」，不得写成当前能力**（见 [AGENTS.md](../../AGENTS.md) 文档分层）。
> 全部分区完成后**删除本文**，把当时的当前状态收回 [ARCHITECTURE.md](../../ARCHITECTURE.md)。
> 分层规则见 [ARCHITECTURE.md](../../ARCHITECTURE.md)，逐文件职责见 [module-map.md](module-map.md)，
> 入站契约见 [contracts.md](contracts.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 零、现状与动机

层与层之间**当前是干净的**：`core` 不引用 `adapters` / `presentation`，`adapters` 不引用 `presentation`，主箭头没有破。
问题全部在 `core` 内部——它名义上是一层，实际是 7 个业务能力挤在一个包里：

| 文件 | 行数 | 实际承担 |
| --- | --- | --- |
| `core/mod.rs` | 2933 | 门面 + 会话中心 + 登记处 CRUD + 发现 + 文件视图 + 历史 + 回档压缩 + 协作驱动 + 任务链派发 + 重建 |
| `core/engine.rs` | 2314 | 讨论/执行/验收引擎 + 轮循环 + 工具调度 |
| `core/collab.rs` | 1685 | 协作状态机 + 讨论泵 + 节点验收 |
| `core/api.rs` | 1234 | 入站契约 + 线程 + 事件台 + 任务登记处 |
| `core/systool.rs` | 990 | 内置工具策略 + 寻址 + 观察账本 + 故障文案拼装 |
| `core/session.rs` | 773 | 单 agent 会话 |

`Core` 一个结构体 ≈68 个方法、持 12 个端口；`sessions` 用"搬进搬出"管理（`remove`/`insert` 共 23 处）；
`presentation` 直接持有 `core::ports::Log`；`core/exec.rs` 里有 `std::env` + `is_file` 的宿主探测。
**`core` 内部的依赖几乎是一团环**：20 个有出边的核心模块里 **16 个构成同一个强连通分量**
（`agents engine events exec fence history module ports prompt providers refs roles schema session systool workspace`）；
`engine ⇄ session`、`session ⇄ systool`、`module ⇄ systool`、`agents ⇄ providers`、`ports ⇄ providers`
这 5 对互环只是它的局部表现。以上由 T0 依赖方向门禁机器判定，基线见 `tests/dependency-baseline.json`。

重构范围因此可以精确锁定：**`core/` 内部拆开 + 一处呈现层违约 + 一条缺失的门禁**。

---

## 一、基本硬要求

### 1.1 目标形态：三层，不是两层

```text
presentation ──▶ 各业务的 api
                      ▲
   协调型业务（rewind / collab） ──▶ 领域型业务（session / llm / tools / registry / workspace / prompt）
                      ▲                                   ▲
                      └──────────────  kernel  ───────────┘
                          （jobs / bus / log；只被依赖，不依赖任何人）
```

| 层 | 判据 | 允许的依赖 |
| --- | --- | --- |
| **协调型业务** | 拥有跨参与方的不变式 | 向下调领域型业务的 `api`；同层协调型业务之间可互相调 `api` |
| **领域型业务** | 拥有自己的状态与端口 | 向下调 `kernel`；**不调协调型业务** |
| **kernel** | 无领域语义、无领域状态 | **不依赖任何人** |
| **adapters** | 实现各业务 `ports` | 只依赖 `kernel` + 各业务 `ports`/`api` |
| **presentation** | 只经 `api` 驱动 | 只依赖各业务 `api` |
| **main** | 组合根 | 装配全部 |

### 1.2 每个能力的内部形态

```text
capabilities/<name>/
  api.rs       入站能力面：trait + DTO。其它能力与呈现层只准用这个
  ports.rs     出站端口：本能力定义的抽象，由 adapters 或别的能力实现
  domain/      纯逻辑：状态机、解析、派生。不加 trait
  detail/      细节实现：含"用别的能力的 api 来实现本能力的端口"
```

### 1.3 硬要求清单

| 编号 | 要求 | 依据 / 落地判据 |
| --- | --- | --- |
| **R1** | 业务之间**只经对方的 `api`** 交流；禁止 `use` 别人的 `domain` / `detail` / `ports` | 门禁按 `use` 边判定 |
| **R2** | 依赖图**必须无环**，由 T0 门禁机器判定 | 白名单外的边 = 测试失败；迁移期允许的边进**基线豁免清单**，拆完即删 |
| **R3** | DIP **只画在 IO 或可替换点上**；纯逻辑刻意不抽象 | 判据：这里有 IO，或这里有可替换实现。不满足就不加 trait |
| **R4** | **状态所有权排他**：一块状态只有一个能力写，别人只读它的 `api` | 跨能力读改写必须经 `api`，不得 `pub` 字段 |
| **R5** | **并发模型不变式**：单线程命令队列 + 能力间同步调用，**核心状态不加锁** | 这是跨能力读改写天然原子的前提（见 §3.6）；不得改成每能力一线程 |
| **R6** | **共享内核唯一归属**：事实类型只属于 `kernel`，禁止各业务复制 DTO | 见 §3.1 的 `kernel` 清单 |
| **R7** | **不保留兼容层**：项目是 GREEN FIELD，迁移是**搬家 + 删旧**，不留转发壳 | 同 `AGENTS.md` 项目不变量 |
| **R8** | **文档同步**：改结构必须同一次改 `ARCHITECTURE.md` / `module-map.md` / 相关细则 / `AGENTS.md` 路由表 | 同 `AGENTS.md` 文档分层与同步 |
| **R9** | **跨平台与路径**：一律 `PathBuf` 组件拼接；对外用 `/`；不假设平台 | 同 `ARCHITECTURE.md` §八 |
| **R10** | **测试跟着业务分区走**：测试目录与业务目录同构，单文件 ≤ 2000 行 | 见 §四.4 |
| **R11** | **测试入口 = 生产入口**：禁止 `#[cfg(test)]` 专用语义入口 | 现状 `Core::single_say` 是反例 |

---

## 二、怎么区分业务边界

### 2.1 三个必要条件（必须同时成立）

一个候选只有同时满足这三条，才配独立成一个能力：

1. **有自己的状态所有权**——这块数据只归它写，别人只经它的 `api` 读；
2. **可独立替换**——换掉它不影响别的能力的内部；
3. **可独立测试**——不需要拉起别的能力的真实实现。

只满足 1、2 不满足 3（或反之）→ 它还是某个能力的一部分，不是能力。

### 2.2 两个判据测试

**① 不变式测试**（决定"是不是业务"）

> 这个横切关注点，有没有一个**没有任何参与方拥有**的不变式？

- 有 → 它是**业务**（协调型）。判据还包括：有自己的协议/落盘形态、有用户可见后果、`api` 用**领域词**表达。
- 没有，只是"按这个顺序调那 5 个" → 它是**脚本**，收进唯一的协调者，不配独立。
- 完全没有领域语义 → 它是**内核**。

**② 领域词测试**（决定"业务还是内核"）

> 这个东西会不会需要知道"什么是回合、什么是回复、什么是工具执行"？

- 会 → **业务**。
- 不会 → **内核**。

### 2.3 三分法

| 类型 | 判据 | 归属 | 依赖方向 |
| --- | --- | --- | --- |
| **协调型业务** | 拥有跨参与方的不变式 + 自己的协议 + 用户可见后果 + 领域词 API | 独立业务（与领域型平级） | 向下调参与方 `api`；**参与方不得反向调它** |
| **领域型业务** | 拥有自己的状态与端口 | 独立业务 | 向下调 `kernel` |
| **机制型内核** | 无领域语义、无领域状态、API 不含领域词 | `kernel/` | 只被依赖 |
| **伪横切（脚本）** | API 就是"按顺序调这几个" | 收进**唯一**协调者 | 否则五个"管理业务" = `Core` 换马甲 |

### 2.4 反例：什么不配切成业务

- **纯机制**：线程、锁、定时、文件句柄、HTTP 客户端 → 内核或 `adapters`。
- **纯逻辑**：解析器、状态派生、提示词渲染 → 所属业务的 `domain/`，**不加 trait**。
- **脚本**：只有调用顺序、没有自己不变式的编排 → 归唯一的协调者，不新增业务。
- **DTO 中转站**：把两个能力的类型拼一起的"适配业务" → 这是新的水平层，禁止。

### 2.5 粒度上限

- 一个能力下任一文件 ≤ **800 行**；
- 一个能力的 `api.rs` ≤ **20 个方法**（超过说明它其实是两个能力）；
- 一个能力只允许依赖**数量有界**的其它能力，超出即重新审视边界。

---

## 三、业务能力清单与大体方案

### 3.1 全景表

| 能力 | 类型 | 现有文件 | 状态所有权 | 端口 | 状态 |
| --- | --- | --- | --- | --- | --- |
| **kernel** | 内核 | **已落位** `src/kernel/`（`jobs` / `log` / `types`） | 生成中作业表（取消标志） | — | **已完成**（批次 1；`bus` 与运行态合并推迟到批次 9，见 §3.2） |
| **session** | 领域 | `core/session.rs`、`history.rs`、`collab_state.rs`、`events.rs` | 对话、转录行、行索引 | `HistoryStore` | 未开始 |
| **llm** | 领域 | `core/providers.rs` 的 Channel 侧、`ports.rs` 的通道族 | 选型解析 | `Chat` `ChatGateway` `ModelCatalog` | 未开始 |
| **tools** | 领域 | `core/systool.rs`、`patch.rs`、`schema.rs`、`roles.rs`、`fence.rs`、`workspace.rs` | 观察账本、围栏策略、工具面 | `SysIo` `ToolRunner` `FenceHost` `Workspace` | 未开始 |
| **prompt** | 领域 | `core/prompt.rs`、`refs.rs` | 提示词册 | `PromptSource` | 未开始 |
| **registry** | 领域 | `core/agents.rs`、`providers.rs` 的登记处侧 | 四份 yaml 的内存形态 | `SettingsStore` | 未开始 |
| **workspace** | 领域 | `core/module.rs`、`packages.rs`、`exec.rs` | 清单快照、执行计划 | `ModuleSource` `PackageSource` | 未开始 |
| **rewind** | 协调 | 散布 5 处（见 §3.6） | 只持自己的日志，**不持会话数据** | — | 未开始 |
| **collab** | 协调 | `core/collab.rs`、`chain.rs`、`engine.rs` | 讨论游标、任务链、待裁决 | — | 未开始 |
| **presentation** | 呈现 | `presentation/` | 界面状态 | — | 未开始 |

### 3.2 kernel（机制型内核）

只放**窄**接口，**不做通用"并发管理器"**。**实际落位（批次 1 已完成）**：

- `kernel/jobs`：`JobRegistry`——取消标志 + 生成中作业表；
- `kernel/log`：`Log` trait + `NoopLog`；
- `kernel/types`：`SessionId`（跨业务共享、**无领域逻辑**的事实类型，R6）。

**命令队列不进内核的 trait**：它是 R5 的**不变式**，一旦做成可替换点，跨能力读改写的原子性就没了。

**三处按项目自己的判据推迟**（不是漏做）：

- **`bus` 不进 kernel**：`EventBus` 的载荷是 `SessionEvent`，且 `tail_excluding` 直接调
  `SessionEvent::to_json`——按 §2.2 的**领域词测试**它会"需要知道什么是回合/回复/工具执行"，
  所以它**不是内核**；按 R3（DIP 只画在 IO 或可替换点上），为它加泛型也是无谓抽象（只有一个实例化）。
  它属于**事件的线格式**，随 `session`（批次 9）一起落位。
- **运行态的两份真相暂不合并**：`Core.running`（对象被工作线程取走）与 `JobRegistry.running`
  （有可取消的作业）**语义不同**，只是今天的可观察区间重合；合并要先定"对象被取走算不算运行中"，
  那是 `session` 的状态所有权问题，随批次 9 一起定。
- **`Msg` / `Completion` / `Chunk` / `LlmOpts` / `CompleteOpts` 不进 kernel**：它们是**模型通道协议**，
  归 `llm`（§3.3）。kernel 只收没有领域语义的类型。

### 3.3 领域型业务

| 能力 | 边界要点 |
| --- | --- |
| **session** | 只管"一个会话的对话与转录"：`SessionParams`、`AgentSession`、行 id/turn/reply 簿记、`build_round_lines`、`collab_state` 派生、`events` 线格式，**以及上下文压缩（compact）**——压缩改的是本会话的**发送视图**，属 session 的状态所有权（见 §3.5）。**不负责回档**（那是 `rewind`），**不负责编排**（那是 `collab`） |
| **llm** | `providers.rs` 必须先拆：`Channel`/模型解析归 `llm`，`Settings`/`AppSettings`/`ModelEntry`/`Provider` 归 `registry`。这是 `llm` 的前置 |
| **tools** | 见 §3.4：工具声明/目录/按角色发放/参数校验/寻址/账本/调度/回执**全锁在内部**；机制（`SysIo`/`ToolRunner`/`FenceHost`）留端口后；**故障文案**（`arg_fault_text`/`refuse`/`patch_fault`/`edit_fault_reason`/`block_fault`）归 `prompt` |
| **workspace** | `module`/`packages`/`exec`。**迁移要点**：`exec.rs` 的宿主探测（`PATH`/`SystemRoot`/`is_file`）是机制，下沉为端口 |
| **registry** | 四份 yaml 的内存形态与 CRUD 编排；`SettingsStore` 端口 |
| **prompt** | 渲染、引用改写、工具文案；缺键/缺变量**报错暴露**，不静默兜底 |

### 3.4 工具业务（tools）

**现状：工具不是"一个业务"，是"一份声明 + 四份散落的实现"。**
`systools/tools.yaml` 是工具**总表**（声明层），但实现分散在四个业务里：

| 工具 | 实现落点 | 今天实际归属 |
| --- | --- | --- |
| `read` `write` `edit` `patch` `list` `search` | `systool::execute` | tools |
| `submit_report` | 回执在 `systool.rs:305`，**消费**在 `mod.rs:529`（判节点完成） | tools 造 / session·collab 消费 |
| `say` `agree` `leave` `ask` | `envelope::Verb` + `collab_state.rs:111-117` + `engine.rs:1099-1117` | **collab** |
| `compact` | `session::compact_turn`（`session.rs:237`） | **session** |

两处硬耦合：**工具面 `MemberTools` 定义在 `engine.rs:86-117`**（还直接持 `repair`/`log`/`runner`/`io` 四个端口）；
**`session.rs:421-423` 伸手改工具内部状态**（`t.observations.clear()`）。

**目标边界**：

- **tools 独占（锁在内部）**：工具目录与声明、按角色发放工具面、参数校验与缺省值、寻址与越界判定、
  观察账本、调度与并发、回执文案。
- **机制留在端口后（不得锁进 tools）**：`SysIo`（文件读写）、`ToolRunner`（拉进程）、`FenceHost`（释放授权）
  —— 按 [ARCHITECTURE.md](../../ARCHITECTURE.md) §二，端口只画在 IO 与可替换点上。锁进业务就等于把
  `std::fs` 与进程启动搬回业务层。
- **只暴露事实**：有哪些工具 / 这个角色能用哪些 / 这次调用的结果。内部结构（`Observations`、`Sandbox`、
  补丁解析、schema 校验）不外露——包括**不再允许**别的业务伸手清账本。

**工作单元信号（本次讨论的结论）**：

- **不加 `progress` 声明位**。三条理由：
  ① **成本论不成立**——`Delta` 按分片推、频率远高于工具信号，且短暂事件本就不落盘
  （`mod.rs:209-217` 已把 `Delta`/`ToolCall` 列为短暂；`mod.rs:325` 明确 `Working` 是"运行态的唯一真相，短暂、不落盘"）；
  ② **声明位解决不了 `compact`**——它的耗时在**压缩回合**（整轮 LLM 调用），不在工具执行那一下；
  挂了 `progress` 也要等模型吐出工具调用才亮，等于**两套机制**并存；
  ③ **`parallel` 的类比不成立**——`parallel` 漏声明会导致**执行语义错误**（该并发的串行了），所以必须声明；
  信号漏发只影响观感。**要声明的是"漏了会错"的东西。**
- **改为工作单元级短暂事件**：每一次工具调用发一对「开始 / 结束」，不落盘、不进上下文、代码里**零特判**；
  压缩回合、节点派发、讨论回合这类长耗时**由发起方**发同一族信号。复用既有机制，不新增声明维度。

**落盘归属（已定）：tools 产出事实，`session` 落盘。**
`session/<名>/transcript.jsonl` 与 `meta.yaml` 是 `session` 的状态，转录行的 `id`/`turn`/`reply` 由 `session`
单调分配（`next_line`/`cur_turn`/`cur_reply`）；tools 不知道这些，直接写就会造出没有这些字段的行，
回放与回档立刻歪。落盘经 `HistoryStore` 端口（唯一实现 `adapters/fs_history.rs`），
tools 自持一个就等于绕过状态所有权——**直接写别人的文件是最典型的耦合（共享可变状态），不是解耦**。
项目已有先例：`Core::persister(sid).persist(&events)` 统一落盘，工具行也由 `session::line()` 造出后统一 persist。

**`compact` 的三方分工（横跨三个业务，不归任何单一方）**：

| 环节 | 归属 |
| --- | --- |
| 触发策略（阈值 / `/compact`） | `session`（只有它知道上下文长度） |
| 压缩回合驱动 + 发「压缩中」信号 | `session` |
| `compact` 工具声明 + 参数校验 + 取 `summary` | `tools` |
| 那次 LLM 调用 | `llm` |
| 摘要落盘（发送视图 + `compacted` 事件） | `session` |

**连带更新**：`docs/architecture/tools-and-roles.md` 的工具总表说明必须同步——
今天 `src/tests/core.rs:6783` 的注释把"实现不在 systool"写成**预期**，重构后这条注释与测试都要改。

### 3.5 会话内的上下文压缩（compact）

**`compact` 不是业务，是 `session` 的一部分**：它改的是**本会话的发送视图**（发给模型的消息列表），
属于 `session` 的状态所有权。`compact` 本身只是 `systools/tools.yaml` 里声明的一个**系统工具**
（`capability: none`），给模型一个"写下摘要"的出口；`compact_turn` 是一次特殊的会话回合
（身份 + **只声明 `compact`** + 对话 + 压缩提示）。

**手动与自动是同一机制的两个触发点**（实现只有 `AgentSession::compact` 一处：
手动 `CoreHandle::compact`、自动 `maybe_compact`），**只有时机不同，没有语义不同**——
任何修法与测试都必须对两条触发点同时成立。

**不变式（`session` 批次必须钉死）**：

1. **改的是"发给模型什么"，不是"留下什么"**——转录永远完整，旧内容用户照样能查看；
2. **发送视图 = `[身份][本回合工具面][最后一份摘要][压缩点之后的行]`**——
   **永远只发送最后一个 compact（含它自己）和之后的内容**；
3. **摘要覆盖"以上全部内容"（含上一份摘要）**，所以压缩后发送视图**只剩这一份摘要**，
   之后新产生的行才跟在它后面。"多次压缩 = 滚动摘要"是这条的推论，不是另一条规则；
4. **触发只有两条**：系统阈值（模型窗口 × `compact_at_percent`）与用户 `/compact`；
   **AI 不能在普通回合自助调 `compact`**（行为可预测）。阈值**按模型窗口现算**，
   不依赖会话是"何时建的"——预算属于会话机制，不属于建立流程（见下方"覆盖范围"）；
5. **压不动就如实说**：没给出摘要 → 记一条通知 + **继续用完整上下文**，不静默降级、不假装压过。

**`marks` 的双消费者（这是 §3.6 方案 A 的关键理由）**：`marks`（"行 → 该行完成时的历史长度"）
不只服务回档——`compact` 也要按它找截断点，并在插入摘要后把整份索引整体 +1。
**两个消费者用不同的变异规则**（回档是 `truncate`，压缩是 `insert` + 整体偏移），
**所以索引不能归任何一个消费者**，必须留在 `session`，作为"行 ↔ 历史长度"映射对外只读暴露。

**压缩 × 回档（已定规则）**：回档**跨越压缩点**时，发送视图必须回到"压缩前"——而压缩已经把对话内容
销毁了，**内存里补不回来**。所以规则是：**跨越压缩点的回档走重建路径**（`rebuild_session` 从转录重放，
转录永远完整），不走内存精确截断（`AgentSession::rewind`）；"是否跨越"按转录里的压缩行 id 比。
不得依赖"截断长度越界时 `Vec::truncate` 是 no-op"这类实现副作用。

**当前实现的偏差（`session` 批次收口前必须清零）**：

- **F1 + F2 是同一个缺陷的两半，必须一起修。** `compact` 用 `truncate(hist)`（**保留前缀**），
  而语义要求**移出前缀**（被总结掉的那一段不再发给模型）；两个触发点传的 `up_to` 又不同
  （手动因取用顺序意外得 0、自动得 `next_line`），恰好互相掩盖：
  手动 `up_to = 0` ⇒ `truncate(0)` = 清空 ⇒ 看起来对；
  自动 `up_to = next_line` ⇒ `hist = marks.last() = dialogue.len()` ⇒ `truncate` 是 no-op ⇒ 一条都没移出。
  **只修一半必然坏事**：只修 F2（让手动拿到真实 `up_to`）会让手动也退化成"只加摘要不删内容"；
  只修 F1（保留前缀 → 移出前缀）会让手动 `up_to = 0` 变成"什么都不移出"。两者必须同时改，
  并**补一条断言"被总结的内容确实消失"的测试**（现有自动压缩测试只断言摘要出现）。
- **F3（`compacted` 重放）**：该事件**确实落盘**（不在短暂事件名单里，`mod.rs:215-221`），
  但 `rebuild_session`（`mod.rs:2654-2662`）只读 `type == "transcript"` 的事件、**跳过它**，
  全仓再无第二个读取方；而摘要本身**不是转录行**（`compact()` 只改对话，不调 `line()`）。
  后果：**压缩只在当前这次运行内有效，重启/回档后发送视图回到完整对话**。
  二选一：(a) 实现——重放到 `compacted` 时把发送视图重置为"摘要 + 之后的行"；
  (b) 认账——改 [session-model.md](session-model.md) §六 并记入 `tests/gaps.yaml`。
- **身份块（已定）**：压缩回合带**该会话自己的身份块**（自动那条已经如此）；手动那条现在是**空串**
  （`compact_plan` 落到 `_ => (0, String::new())`），属缺陷，修法就是让它在会话被取走**之前**读到身份。
  **不把压缩交给核心 AI**，四条理由：①`compact_prompt` 要留下"以后还要用的"，而"以后需要什么"只有
  该会话自己知道——核心对 agent 的私有工作史只有 meta 里那点认知（[PHILOSOPHY.md](../../PHILOSOPHY.md)：
  "核心不认识任何业务，不知道任何模块的内部"）；②核心通道与 agent 通道可以是**不同模型**
  （`settings.core_model` vs `agent.model`），让核心压等于把 N 个会话的上下文灌进核心模型；
  ③**它不解决 F2**——取用顺序问题照旧，只是换了 identity 的来源；④"状态私有"——子会话的内容属于它自己。
- **覆盖范围（新发现，比身份块更该先补）**：`set_compact_budget` 全仓**只有一处调用**
  （`mod.rs:1993`，在 `build_single` 里；而 `build_single` 只由 `create_work` 调，`mod.rs:1742`）。后果：
  ①单 agent 会话**重启后预算丢失**（`ensure_session` → `rebuild_session` → `restore` 把 `compact_at` 置 0，
  `session.rs:199`）⇒ 自动压缩失效；
  ②协作**子会话**经 `spawn_agent_session` 建（`mod.rs:913`），从没设过预算 ⇒ 无自动压缩；
  ③协作**主会话**既无预算，`/compact` 也直接报错（`take_single` → "该会话不是单 agent 模式"）。
  ⇒ 预算应由**会话机制按模型窗口现算**，而不是在 `create_work` 那一刻设一次；
  这样重建与子会话都自然覆盖。
- **摘要的落档与角色（已定）**：摘要按 **`note_task` 的形状**落档——**转录里是系统行，发给模型的上下文里是
  `user` 角色**。注意 `note_system` **不是**这个形状：它 push 的是 `Msg::system`，上下文里是 system 角色
  （`session.rs:291-295`）；`note_task` 才是 `Msg::user` + `kind:"system"`（`session.rs:301-307`），
  其注释写明"全是 system 的请求会被供应商整条拒收"。
  这条**顺带修掉 F3**：摘要成为**真正的转录行**后，`rebuild_session` 才重放得出它。
  **不复用 `task: true` 标记**（它的语义是"派发行"）——按"行上的判定走结构化字段"新增一个标记
  （例如 `compact: true`），由 `rebuild_session` 认它并把角色映射成 `user`。
- **主协作会话（已定：不叠 `compact`）**：它的"上下文"是讨论转录，不是消息列表，压缩语义与
  `AgentSession` 不同。讨论有自己的收束路径（全员同意即收束，收敛后由 `synthesize` 整理成方案 + 任务链，
  这一步已经在用核心 AI）。若讨论转录仍会无限增长，那是 `collab` 自己的职责，不是 `compact` 的职责。
- **讨论上限（已定：全部保留，重构不碰）**。`MAX_ROUNDS` **不是**防长上下文的机制，而是讨论的
  **收敛兜底 + 用户裁决触发点**：`engine.rs:1023-1033` 一轮走完后，全员同意即收束，否则超过
  `MAX_ROUNDS` 也强制收束；`collab.rs:1036-1042` 据此发 `[上限]` 通知与 `DiscussionDone { over_cap }`。
  **它不打断流程**——交出去的是审查关卡（用户点同意方案才开工）。它是 [PRODUCT.md](../../PRODUCT.md)
  的四条行为不变量之一（`PRODUCT.md:185`"上限必生效"、`PRODUCT.md:161` 终止条件）。
  **提醒次数上限**（`discuss_remind_cap`，设置项，默认 3）同样**保留**：它是循环的**前进条件**——
  `engine.rs:939-940` 原文："没表态时由 after_turn 计数（提醒 → 放过），否则假模型瞬间返回
  会把这里变成紧凑死循环（真烧过 CPU）"。

**处理方式（已定）**：按 `AGENTS.md` 记入 [tests/gaps.yaml](../../tests/gaps.yaml)——
`session.compaction-send-view-not-truncated`（F1+F2）与 `session.compaction-not-replayed`（F3），
**与批次 9（`session`）一并修**，不单独改两遍。

### 3.6 rewind（协调型业务）

**它拥有的不变式（没有任何参与方单独拥有）**：

1. **按 reply 原子截断**——截在一次回复内部会留下"孤儿工具结果"，协议要求结果紧跟发起它的助手消息；
2. **主/子会话按 turn 同步截断**——子会话不在主会话流水里，得各自回档；
3. **落盘只追加一条 `{"type":"rewind"}`，不物理删行**——转录即内容；
4. **回档后工具账本作废**——宁可让模型重读一遍；
5. **用户可见后果**——"回档删掉了其后 N 次工具执行，副作用不会回滚"。

**为什么它是业务而不是脚本**：这 5 条跨越 `session` / `tools` / `history` / `collab_state` 四方，
但没有一条属于其中任何一个；且它**已被当作原语复用**——`Core::update_task`（改需求）的实现就是
"回档到需求行 + 追加新需求"（`mod.rs:2551`）。

**现状散布（5 处）**：

| 位置 | 内容 |
| --- | --- |
| `core/mod.rs` | `rewind`、`turn_of_line`、`last_line_within`、`rewind_children`、`truncate_events`、`cut_before_line`、`align_keep`、`line_reply_of`、`find_line_id`（≈250 行） |
| `core/session.rs` | `rewind`、`keep_whole_replies`，及 `marks`/`line_reply`/`next_line` 字段与 15 处簿记 |
| `core/collab_state.rs` | `tool_runs()`——算"删掉了几次工具执行" |
| `core/history.rs` | append-only 的 `rewind` 记录协议 |
| `core/mod.rs` | `rebuild_session`（177 行）——协作会话回档走整段重建 |

**越界耦合（切出来正好消掉）**：`AgentSession::rewind` 里 `t.observations.clear()`
（`session.rs:421-423`）——会话的回档操作伸手改了**工具能力**的内部状态。

**约束**：

- `api` 必须是领域词：`rewind(sid, keep_id)` ✔；`set_dialogue_length(sid, n)` ✘；
- **无状态**（或只持自己的日志），不得持有会话数据；
- 参与方**不得反向调用它**——现状的边是干净的（调用点只有 `api.rs:1103` 入站与 `mod.rs:2551` 内部复用），切分不会引入环 ✔。

**`marks`/`line_reply` 归谁（已定：方案 A）**

| 方案 | 做法 | 代价 |
| --- | --- | --- |
| **A（采用）** | 索引留 `session`，暴露只读的 `lines()`（id/turn/reply/history_len）；`rewind` 算完再调 `session.truncate_to(hist_len)` | `rewind` 是**读改写**，中间有窗口 |
| B | 索引归 `rewind`，`session` 通过行事件喂它 | `rewind` 复制了 `session` 的内部账，两边会漂移 |

理由有两条：①`marks` 是"行 → 该行完成时的历史长度"，而**历史长度只有 `session` 知道**，
搬出去等于让 `rewind` 维护一份镜像；②它**已被 `compact` 共用**（见 §3.5），
两个消费者用不同的变异规则，索引归谁都会让另一方失去一致性。
方案 A 的安全性**完全依赖 R5**（单线程命令队列 ⇒ 读改写无交错）。

### 3.7 collab（协调型业务）

协作状态机、讨论泵、任务链、审查关卡、节点验收、总验收。它是依赖最多的能力，**最后迁**。
前置：`engine ⇄ session` 的环必须先解（`engine` 引 `session::build_round_lines`/`stream_piece`；
`session` 引 `engine::assemble`/`converse_with`/`Round`）。

### 3.8 目标依赖图（必须无环）

```text
presentation ──▶ {session, llm, tools, prompt, registry, workspace, rewind, collab, kernel} 的 api
rewind       ──▶ session, tools
collab       ──▶ session, llm, tools, prompt, registry, workspace
tools        ──▶ prompt, workspace, kernel
session      ──▶ kernel
llm          ──▶ kernel
registry     ──▶ kernel
workspace    ──▶ kernel
prompt       ──▶ kernel
kernel       ──▶ （无）
```

`rewind` 与 `collab` 同层，允许 `collab → rewind`（改需求复用回档），但**禁止反向**。

---

## 四、迁移流程（绞杀模式）

### 4.1 总则

1. **一次只绞杀一个能力**。每个批次独立可验收、结束后仓库全绿，**不允许跨批次半成品共存**。
2. **门禁先行**：批次 0 先立依赖方向门禁，把现状的违规边记为**基线豁免**；之后每拆一个能力就删掉对应豁免，
   **豁免清零 = 重构完成**。这样"禁止耦合"从第一天起就是机器判定的，而不是靠自觉。
3. **搬家不改语义**：批次内只允许"移动 + 改可见性 + 删旧路径"。任何行为变更**另开批次**。
4. **每个批次完成即销账**：把 §4.2 表里的状态改为「已完成」，并同步文档（R8）。
5. **测试跟着走**：每个批次把该业务的测试从 `src/tests/core.rs` 拆到同构文件（§4.4）。
6. **不留兼容层**（R7）：旧路径**删除**，不做 `pub use` 转发。

### 4.2 批次表（销账表）

| 批次 | 目标 | 现状 | 前置 | 状态 |
| --- | --- | --- | --- | --- |
| **0** | **依赖方向门禁**：T0 加 `use` 边检查 + 基线豁免清单 | 无门禁 | — | **已完成**（`run-tests.js` 的 T0 结构审查 + `tests/dependency-baseline.json`） |
| **1** | **kernel**：`jobs` / `log` / `types`（`bus` 与运行态合并按 §3.2 推迟到批次 9） | `api.rs` 的 JobRegistry；`ports.rs` 的 Log；`core/mod.rs` 的 SessionId | 0 | **已完成**（`src/kernel/`） |
| **2** | **修两处违约**：`exec.rs` 宿主探测下沉为 `HostProbe` 端口（4 处 IO）；`presentation` 不再持 `Log`/`ProbeOutcome`（改经入站能力面 `LogOps`） | `exec.rs` 的 `std::env`/`is_file`/`is_dir`；`web.rs` 的 `Log`/`ProbeOutcome` | 0 | **已完成**（基线 9 → 7 条） |
| **3** | **prompt** | `prompt.rs` `refs.rs` | 1 | 未开始 |
| **4** | **workspace**：`module` / `packages` / `exec` | `module.rs` `packages.rs` `exec.rs` | 2,3 | 未开始 |
| **5** | **registry**（含拆 `providers.rs`） | `agents.rs`；`providers.rs` 登记处侧 | 1 | 未开始 |
| **6** | **llm**：`Channel` 解析 + 通道端口族 | `providers.rs` 的 Channel 侧；`ports.rs` 通道族 | 5 | 未开始 |
| **7** | **tools**（钉死 §3.4：实现锁内部、机制留端口、工具调用发一对短暂事件、**产出事实不落盘**）：`systool`/`patch`/`schema`/`roles`/`fence`/`workspace`；文案归 `prompt` | 6 个文件 | 3,4 | 未开始 |
| **8** | **解 `engine ⇄ session` 环**：定清 `build_round_lines`/`stream_piece`/`assemble`/`converse_with` 的归属 | 双向依赖 | 7 | 未开始 |
| **9** | **session**（含压缩 `compact`：钉死 §3.5 的五条不变式与压缩×回档边界） | `session.rs` `history.rs` `collab_state.rs` `events.rs` | 8 | 未开始 |
| **10** | **rewind**（协调型；方案 A） | 散布 5 处 | 9 | 未开始 |
| **11** | **collab** | `collab.rs` `chain.rs` `engine.rs` | 10 | 未开始 |
| **12** | **presentation 收口 + 前端分区**：只 `use` 各业务 `api`；`app.js` 分区 | `cli.rs` `web.rs` `intent.rs`；`app.js` 2697 行 | 11 | 未开始 |

**豁免清零判据**：`tests/dependency-baseline.json` 的**三个数组全部清空**（`reverse` / `presentation` / `coreCycles`），
且 `Core` 这个类型不再存在。门禁对**新增**与**过期**都报失败，所以销账不靠自觉——
拆掉一条边不删条目，构建就红。

### 4.3 每个批次的完成定义（DoD）

一个批次只有**全部满足**才可销账：

1. 新目录建立，代码迁入，**旧路径已删除**（不留转发壳）；
2. `cargo test` 与 `node run-tests.js` 全绿（T0 质量门禁 + 业务测试都过）；
3. 依赖方向门禁通过，**该批次的基线豁免已删除**；
4. 该能力的测试已按 §4.4 拆到同构文件，单文件 ≤ 2000 行；
5. 文档同步：`module-map.md` 对应行已改；受影响的门户/细则已改（R8）；
6. §4.2 表状态改为「已完成」，并写明批次号。

### 4.4 测试迁移规则

- **目录同构**：业务 `capabilities/<name>/` ↔ 测试 `src/tests/<name>/`；`src/tests/core.rs`（8222 行）按能力拆空后删除。
- **搬家不改断言**：拆分批次内只移动测试与改路径，**不动断言**。要改断言语义 → 另开批次并写明理由。
- **消除测试专用入口**（R11）：`Core::single_say`（`#[cfg(test)]`）这类"测试路径 ≠ 生产路径"的双轨，
  在 `session` / `collab` 批次里改为走生产入口（`CoreHandle`）。
- **替身跟着端口走**：`src/tests/doubles.rs` 按端口归属拆到各能力，端口的真实适配器覆盖范围见 [doubles.md](../testing/doubles.md) 三。
- **每批次保留一条行为不变验收**：至少一条端到端用例证明该批次"只搬家、没改行为"。

---

## 五、验收标准

1. `core/` 消失，替换为 `capabilities/` + `kernel/`；每个能力有 `api.rs` / `ports.rs` / `domain/` / `detail/`。
2. 依赖图为**无环**，由 T0 门禁机器判定，**零豁免**。
3. `Core` 这个类型不存在；任一能力文件 ≤ 800 行，`api.rs` ≤ 20 个方法。
4. `presentation` 只 `use` 各业务的 `api`：**零** `use ...::ports::`、零 `use ...::domain::`。
5. 端口对象在**恰好一处**被持有（组合根），不再手工穿层。
6. 运行态**只有一份真相**（`kernel/jobs`）。
7. 测试按能力分文件，单文件 ≤ 2000 行；**测试入口 = 生产入口**。
8. 架构文档（`ARCHITECTURE.md` + `module-map.md` + 相关细则 + `AGENTS.md` 路由表）与代码一致，无过期描述。
9. 本文删除。
