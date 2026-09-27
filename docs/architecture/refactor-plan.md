# 重构方案：按业务能力垂直切分

> 本文是**重构的迁移账**：目标形态、业务边界判据、能力清单与迁移批次。
> **未实施的条目一律记「未开始」，不得写成当前能力**（见 [AGENTS.md](../../AGENTS.md) 文档分层）。
> 全部分区完成后**删除本文**，把当时的当前状态收回 [ARCHITECTURE.md](../../ARCHITECTURE.md)。
> 分层规则见 [ARCHITECTURE.md](../../ARCHITECTURE.md)，逐文件职责见 [module-map.md](module-map.md)，
> 入站契约见 [contracts.md](contracts.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 零、现状与动机（**批次 0 时的起点快照**，不是当前状态；当前状态见 §4.2 销账表）

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
   协调型业务（collab） ──▶ 领域型业务（session / llm / tools / registry / workspace / prompt）
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
  api.rs       **入站能力面 = 本能力的 trait + DTO**（**不含状态**）。其它能力与呈现层只准用这个
  service.rs   **本能力的状态与用例**：持有自己的状态与端口，实现 api 的 trait。
               `core` 只持有 `Arc<dyn …>`（按 trait 用），**看不见它的字段**（R4 的落地处）
  ports.rs     出站端口：本能力定义的抽象，由 detail 或别的能力实现
  domain/      纯逻辑：状态机、解析、派生。不加 trait
  detail/      **细节实现 = 该能力自己的适配器**：文件读写、HTTP/TLS、拉进程、平台围栏…
               **组合根（入口层）是唯一构造它的地方**，它在这里 new 出 service 并注入 core
```

**状态归属的判据（R4 的落地）**：一块状态（登记处的四份 yaml、会话历史、模块清单…）**只由一个能力写**。
写它的操作跟着状态走，放在那个能力的 `service.rs`；`core` 只按 `api` 的 trait 调用，拿不到字段。
**例外**：`core` 自己的状态（会话中心：会话表、命令队列、运行态）由 `core` 持有——它是应用服务，不是能力。

**已落地的两块样板（批次 17）**：

- **`registry`（有端口的状态）**：`service.rs` 持四份 yaml 与 `SettingsStore` / `ChatGateway` / `ModelCatalog`，
  实现 `api.rs` 的 `Registry`；`core` 只剩 `registry: Box<dyn Registry>`（`registry()` 读、`registry_mut()` 写），
  `settings` 字段与那 17 个方法已删。读事实用 `app()`（借用，不复制）、`resolve()` / `tool_mode()` /
  `context_of()` / `channel()` / `snapshot()`；**写只有 `RegistryService` 一处**。
- **`prompt` / `tools`（纯数据的状态，没有端口）**：状态就是 `domain` 的 `Prompts` / `SystemTools`，
  `service.rs` 把能力面（`Prompt` / `Tools`）挂在它身上、并给组合根一个装载入口（加载器经
  `PromptSource` / `SystoolsSource` 端口）；`core` 只剩 `prompt: Arc<dyn Prompt>` 与 `systools: Arc<dyn Tools>`。
  **别的能力不点字段路径**：`prompt` 按名字取段（`text` / `render` + `Segment`）或拿走两块共享记录
  （`tools()` / `refs()`，都是 `Arc`）；`tools` 按角色发放（`tool_face` / `role_face`）或拿走总表（`book()`）。
  这一条清掉了 `prompts.core.*` 的 `42` 条跨能力路径、每会话一份的册子深拷贝，以及每回合一份的 `SystemTools` 克隆。

### 1.3 硬要求清单

| 编号 | 要求 | 依据 / 落地判据 |
| --- | --- | --- |
| **R1** | 业务之间**只经对方的 `api` 交流**（入站契约：trait + DTO）。**不准引 `ports`**——端口是"对方与它自己 `detail` 之间的事"，只能由**定义它的能力**持有；**禁止 `use` 别人的 `domain` / `detail`**；**禁止给别的能力的类型写 `impl`**（那是另一种"互相引入"，门禁原来只看 `use` 边，漏了）。`::detail` **只有入口层（组合根）能碰** | 门禁按 `use` 边 + **`impl` 边**判定；迁移期仍引 `ports` 的点先记基线，批次 20 清零 |
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
| **R12** | **端口只有一个持有者：定义它的那个能力**（它的 `service.rs`）。别人只拿 `api` 面；组合根只注入"每个能力**自己的**端口 + 别人的 **`api` 面**"。**例外**：`kernel` 的机制端口（`Log` / `HostProbe`）是全项目共享的机制接口，任何能力/适配器都可持有 | R1 的推论；§五.5"端口一处持有"据此改写成可判定的口径；门禁只查 `capabilities/*/ports`，kernel 机制端口不在其列 |
| **R13** | **跨能力的同形重复不许存在**：同一形状在 ≥2 个能力里各自实现 ⇒ 要么收进唯一所有者，要么**独立成一个业务**（有 `api`、有主人、门禁同样对待） | 审查判据。已知反例：拟名单（`core::suggest_models` 与 `collab::draft_slate` 各写一遍）、"核心操作回路"调用侧包装 5 处 |

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

**①' 状态归属测试**（决定"这块状态归谁"）——回答"是不是所有状态都该抽进协调业务"：

> 这块状态，**有没有任何一个参与方拥有它的不变式**？

- **有** → 归**那个业务**（它的 `service.rs` 持有、别人只经 `api` 读改）；
- **没有，且多个参与方共同依赖** → 归**协调业务**（跨参与方的状态与编排）；
- **只被一个参与方用、自己又没有不变式** → 它不是状态，是**派生值**（留在 owner 的 `domain/`，现算）。

**反例警告（这是旧 `Core` 长成巨石的机制）**：不能因为"这块状态看起来能抽出来"就抽进协调业务。
判据是**不变式归属**，不是"能不能抽"。抽出来却没有跨参与方不变式的，就是 §2.4 的"脚本 / DTO 中转站"，
只会把协调业务喂成新的巨石。**协调业务自己也要满足 §2.1 三条**（它有不属于任何参与方的不变式）。

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

**第三种形态：纯领域业务**（批次 20 新增）：**有不变式、没有端口**——状态是一个**值对象**，规则全在 `domain/`，
`api` 只导出"查询与派生"；**不需要 `service.rs`**（没有 IO 可编排）。样板：**任务链**（阶段派生 / 就绪 / 验收判定）。

### 2.4 反例：什么不配切成业务

- **纯机制**：线程、锁、定时、文件句柄、HTTP 客户端 → 内核或 `adapters`。
- **纯逻辑**：解析器、状态派生、提示词渲染 → 所属业务的 `domain/`，**不加 trait**。
- **脚本**：只有调用顺序、没有自己不变式的编排 → 归唯一的协调者，不新增业务。
- **DTO 中转站**：把两个能力的类型拼一起的"适配业务" → 这是新的水平层，禁止。

### 2.5 粒度上限

- 一个能力下任一文件 ≤ **800 行**；
- 一个能力的 `api.rs` ≤ **20 个方法**（超过说明它其实是两个能力）——**按"一个调用角色的面"计数**：
  同一批用例面向两类调用方（呈现层走命令队列、其它能力同步读事实）时会各成一个 trait，分别计数。
  `registry` 是已知的宽面（`Registry` 29 个方法：18 个只读事实 + 11 个用例），**它仍是一个能力**——
  判据是 §2.1 的三条必要条件，不是方法数；
- 一个能力只允许依赖**数量有界**的其它能力，超出即重新审视边界。

---

## 三、业务能力清单与大体方案

### 3.1 全景表

| 能力 | 类型 | 现有文件 | 状态所有权 | 端口 | 状态 |
| --- | --- | --- | --- | --- | --- |
| **kernel** | **业务（机制型）**——它有状态（取消表）、有端口（`Log`/`HostProbe`）、有机制（路径）、有事实类型，按 §2.1 三条够格当一个能力；只是它的领域词最少 | **已落位** `src/kernel/`（`jobs` / `log` / `types` / `path` / `host`） | 生成中作业表（取消标志） | `Log` `HostProbe` | 批次 1 落位；**批次 20 补齐能力形状**（`api`/`ports`/`domain`/`detail`，**适配器从 `adapters/` 归位**，`chain` 搬出） |
| **taskchain** | 领域（**纯**：有不变式、无端口） | 现 `kernel/chain.rs`（232 行，批次 5 为断环搬入 kernel） | **任务链本身**（值对象 + 派生规则：阶段、就绪、验收） | — | **批次 20 独立成业务**：`collab` 触发、`session` 线格式携带、呈现层渲染，三方都要经它的 `api`；留在 kernel 里等于"环只是藏进了一个叫内核的地方" |
| **conductor**（协调业务） | 协调 | **已落位** `capabilities/conductor/`（`api.rs` + `service.rs`） | **会话在世表 + 命令队列 + 运行态**（跨参与方的状态，没有任何参与方拥有它的不变式 ⇒ 按 §2.2 ①' 归协调业务） | — | **批次 20 从 `core` 独立成业务**：只持各能力的 **`api` 面**；`Ops` 组装、事件台、队列代理随它；**它和其他能力受同一条 R1/R12 约束** |
| **session** | 领域 | **已落位** `capabilities/session/`（`session` + `history` + `events`）（`collab_state.rs` 已改判归 `collab`——它派生的是**协作**状态） | 对话、转录行、行索引 | `HistoryStore` | **已完成**（批次 12） |
| **llm** | 领域 | **已落位** `capabilities/llm/`（`ports` 的通道族 + `domain/envelope`） | 通道协议与回复解析 | `Chat` `ChatGateway` `ModelCatalog` `EnvelopeRepair` | **已完成**（批次 9） |
| **tools** | 领域 | **已落位** `capabilities/tools/`（`api` + `service` + `domain/{systool,patch,schema,roles,fence}` + `detail`） | 观察账本、围栏策略、工具面；**工具总表与角色表只由 `service.rs` 持有** | `SysIo` `ToolRunner` `FenceHost` `SystoolsSource` | **已完成**（批次 11；两张表随批次 17 归位 `service.rs`） |
| **prompt** | 领域 | **已落位** `capabilities/prompt/`（`api` + `service` + `domain/prompt` + `domain/refs` + `detail/yaml_prompts`） | 提示词册（**只由 `service.rs` 持有**） | `PromptSource` | **已完成**（批次 7；适配器见批次 16；状态与取用面随批次 17 归位 `service.rs` + `Segment`） |
| **registry** | 领域 | **已落位** `capabilities/registry/`（`api` + `service` + `domain/providers` + `domain/agents`） | 四份 yaml 的内存形态（**只由 `service.rs` 写**） | `SettingsStore`（自己的）；另经 `llm::api` 的 `ChatGateway` / `ModelCatalog` 做探测与发现 | **已完成**（批次 8；状态与用例随批次 17 归位 `service.rs`） |
| **workspace** | 领域 | **已落位** `capabilities/workspace/`（`module` + `packages` + `exec` + `workspace` 沙箱数据） | 清单快照、执行计划、沙箱寻址 | `ModuleSource` `PackageSource` `Workspace` | **已完成**（批次 10） |
| ~~**rewind**~~ | ~~协调~~ | **改判：不是独立能力**——无独立状态所有权，归 `session`（见 §3.6） | — | — | **已并入批次 13** |
| **collab** | 协调 | **已落位** `capabilities/collab/`（`collab` + `collab_state` + `engine`）（任务链**待批次 20 搬去 `taskchain`**） | 讨论游标、待裁决（~~任务链~~ → 批次 20 归 `taskchain`） | — | **已完成**（批次 14）；**批次 20 补内部层次**（`engine`/`collab` 是编排，要落 `service/`） |
| **presentation** | 交付机制（**不是能力**：无状态所有权、无独立不变式） | **已落位** `cli/` + `web/`（各自独立、无共享层） | — | — | **已完成**（批次 15 收口第 3 步） |


**`detail/` 归谁（本批次的依据）**："细节实现"就是**该能力自己的适配器**。证据是逐条可判——
`adapters/` 现有 24 个文件里，**20 个恰好只实现一个能力的端口**：

| 能力 | 它的 `detail/`（现在散在 `adapters/`） |
| --- | --- |
| **prompt** | `yaml_prompts.rs` |
| **registry** | `yaml_settings.rs` |
| **session** | `fs_history.rs` |
| **llm** | `http_chat.rs` / `http_probe.rs` / `repair.rs` / `http_agent.rs` / `endpoint.rs` / `model_catalog.rs` / `fake_chat.rs` |
| **workspace** | `fs_modules.rs` / `fs_packages.rs` / `fs_workspace.rs` |
| **tools** | `confine/*`（5 个平台文件）/ `proc_tools.rs` / `sys_io.rs` |

**留在 `adapters/` 的 4 个（批次 20 归位后目录归零）**：`log.rs` / `host_probe.rs` → **`kernel/detail/`**
（kernel 业务化后它有自己的 `detail`）；`root.rs` → **入口层**（`main.rs` / `diagnostics` / `guard` 共用）；
`mod.rs` 随目录消失。**归零后"谁能碰 `::detail`"只剩入口层一档**，门禁少一个特例。

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
| **llm** | `providers.rs` 已拆：登记处侧（`Settings`/`AppSettings`/`ModelEntry`/`Provider`/`ToolMode`/`Channel`/视图）已在 `capabilities/registry/`；`llm` 剩通道端口族与 `envelope`。`Channel` 暂留 registry（它是「解析结果」这一事实），`llm` 经 `registry::api` 用它 |
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

两处硬耦合：**工具面 `MemberTools`**（批次 12a 已从 `engine.rs` 搬到 `capabilities/session/domain/session.rs`，它的**行为**仍留在引擎；仍直接持 `repair`/`log`/`runner`/`io` 四个端口）；
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
回放与回档立刻歪。落盘经 `HistoryStore` 端口（唯一实现 `capabilities/session/detail/fs_history.rs`），
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
原 `src/tests/core.rs` 里那条"实现不在 systool"的注释把偏差写成**预期**（批次 19 拆到 `src/tests/tools.rs` |
    `session.rs`），重构后这条注释与测试都要改。

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

### 3.6 rewind（**已定：归 `session`，不是独立能力**）

**它要保证的不变式**（仍然成立，但由 `session` 的 api 与 `Core` 的编排共同保证）：

1. **按 reply 原子截断**——截在一次回复内部会留下"孤儿工具结果"，协议要求结果紧跟发起它的助手消息；
2. **主/子会话按 turn 同步截断**——子会话不在主会话流水里，得各自回档；
3. **落盘只追加一条 `{"type":"rewind"}`，不物理删行**——转录即内容；
4. **回档后工具账本作废**——宁可让模型重读一遍；
5. **用户可见后果**——"回档删掉了其后 N 次工具执行，副作用不会回滚"。

**为什么不是独立能力**：它读写的**全部状态**（转录行、`marks` / `line_reply` / `next_line`）都归 `session` 所有——它自己没有状态，不满足 §二「独立状态所有权」这条必要判据。下面的方案 A（索引留 `session`）本来就已经承认了这一点。

**切法（批次 13 已完成）**：

| 位置 | 内容 |
| --- | --- |
| `capabilities/session/domain/rewind.rs` | **纯行 / 事件算术**：`turn_of_line`、`last_line_within`、`truncate_events`、`cut_before_line`、`align_keep`、`line_reply_of`、`find_line_id` |
| `capabilities/session/domain/session.rs` | `AgentSession::rewind`、`keep_whole_replies`，及 `marks` / `line_reply` / `next_line` 与簿记 |
| `capabilities/collab/domain/collab_state.rs` | `tool_runs()`——算"删掉了几次工具执行"（经 `collab::api`） |
| `capabilities/session/domain/history.rs` | append-only 的 `rewind` 记录协议 |
| 协调业务（`conductor`，批次 20 从 `core` 独立） | `rewind` 编排、`rewind_children`（撤子会话）、`rebuild_session`（整段重建）——**跨会话**所以归协调（§2.2 ①'：这块不变式不属于任何单个会话）。**它只经 `session::api` 调**，不再自己持 `HistoryStore`（R12）；装配（造适配器）在 `main.rs` |

**越界耦合已消**：`AgentSession::rewind` 里 `t.observations.clear()` 伸手改工具账本——`MemberTools` 已在批次 12a 归 `session`，所以这不再是跨能力越界。

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

协作状态机、讨论泵、审查关卡、节点验收、总验收。它是依赖最多的能力，**最后迁**。

**批次 20 的两条修正**：① **任务链搬去 `taskchain`**（它是自足的纯领域业务，见 §3.1；三个消费者都要经它的 `api`）；
② `domain/{engine,collab}.rs` 是**编排**（持 7 个端口、驱动 IO）⇒ 按 R12/R1 落 `service/`，`domain/` 只留纯派生（`collab_state`）。
前置（**批次 12a 已完成**）：`engine ⇄ session` 的环已解——做法是**循环反转**：把回合驱动（`say`/`dispatch_task`/`discussion_turn`/`continue_reply`/`compact_turn`/`run_rounds`/`run`）与行构造从 `session.rs` 搬进 `engine.rs`（以 `impl AgentSession` 写在引擎里，调用点零改动），并把回合词汇（`MemberTools`/`ModuleTools`）搬进 `session.rs`。依赖方向因此是单向 `engine → session`。

### 3.8 目标依赖图（必须无环）

**目标**：

**批次 20 起，边只可能是 `api`**（R1）：`A ──▶ B` 读作"A 调 B 的 `api`"，**不表示 A 持 B 的端口**（R12）。

```text
presentation ──▶ {conductor, session, llm, tools, prompt, registry, workspace, collab, taskchain, kernel} 的 api
conductor    ──▶ 其余能力的 api（跨会话/跨能力编排；**不持任何别人的端口**）
collab       ──▶ conductor, session, llm, tools, prompt, registry, workspace, taskchain
session      ──▶ llm, tools, prompt, taskchain, kernel
registry     ──▶ llm, prompt, workspace, kernel      # 探测/发现改为调 llm::api 的用例
tools        ──▶ llm, prompt, workspace, kernel
workspace    ──▶ prompt, kernel
llm          ──▶ kernel
prompt       ──▶ kernel
taskchain    ──▶ kernel                              # 纯领域：无端口、无 service
kernel       ──▶ （无）
```

**端口**（`ports`）不出现在这张图里：它是**各能力与自己 `detail` 之间的事**，跨能力引用一律不许（R1/R12）。

**现状（批次 15 结束时）：零环** ✔。四处切法：

1. `ToolMode`（通道的工具调用形态）与回放探测结论（`ReplayShape` / `ReplayReport`）从 `registry` 移到 `llm`——
   它们是「关于通道的事实」，随通道走；切掉 `session → registry` 与 `workspace → registry`；
2. `Channel` 从 `registry` 移到 `llm` 并**摊平**（`base_url` + `api_key` + `model`，不再嵌 `registry::Provider`）；
   `ModelCatalog::list_models` 改为收端点与密钥——切掉 `llm → registry`；
3. `env_block`（渲染 `SessionParams`）从 `tools` 移到 `session`——切掉 `tools → session`；
4. 「清单 → 工具面」的逻辑（`ToolDecl::schema` / `check_tools` / `module_tools` / `module_tool_params`）
   从 `workspace` 移到 `tools`；参数**声明形态**（`Param` / `ParamType`）留在 `workspace`（它是 `module.yaml` 的字段）
   ——切掉 `workspace → tools`。

`rewind` 不再是能力（见 §3.6），`collab` 的改需求复用回档改为经协调业务的门面。

---

## 四、迁移流程（绞杀模式）

### 4.1 总则

1. **一次只绞杀一个能力**。每个批次独立可验收、结束后仓库全绿，**不允许跨批次半成品共存**。
2. **门禁先行**：批次 0 先立依赖方向门禁，把现状的违规边记为**基线豁免**；之后每拆一个能力就删掉对应豁免，
   **豁免清零 = 重构完成**。这样"禁止耦合"从第一天起就是机器判定的，而不是靠自觉。
3. **搬家不改语义**：批次内只允许"移动 + 改可见性 + 删旧路径"。任何行为变更**另开批次**。
4. **每个批次完成即销账**：把 §4.2 表里的状态改为「已完成」，并同步文档（R8）。
5. **测试跟着走**：每个批次把该业务的测试拆到同构文件 `src/tests/<能力>.rs`（§4.4）。
6. **不留兼容层**（R7）：旧路径**删除**，不做 `pub use` 转发。
7. **先切边，再搬能力**（批次 3 的实测结论）：`core` 的 20 个有出边的模块里 **16 个同属一个强连通分量**，
   **没有任何一个能力是叶子**——每个能力都被至少一条 `core` 依赖挡住。所以顺序不是"挑叶子先搬"，
   而是**先切掉那些数据依赖，再搬**。切边的判据只有两条：
   ① **数据聚合**（把两个能力的册子/表焊在一个结构体里，例如 `Prompts` 曾同时装文本与工具总表）；
   ② **机制错位**（纯机制住在某个业务里，例如 `slash` 曾住在 `workspace`）。

### 4.2 批次表（销账表）

**阶段 A：切边（行为不变，只搬家 / 拆类型）**

| 批次 | 目标 | 切掉的边 | 状态 |
| --- | --- | --- | --- |
| **0** | **依赖方向门禁** + 基线账本 | — | **已完成**（`run-tests.js` 的 T0 结构审查 + `tests/dependency-baseline.json`） |
| **1** | **kernel**：`jobs` / `log` / `types`（`bus` 与运行态合并按 §3.2 推迟到阶段 B 的 session） | — | **已完成**（`src/kernel/`） |
| **2** | **修两处违约**：`exec.rs` 宿主探测下沉为 `HostProbe` 端口（4 处 IO）；`presentation` 改经入站能力面 `LogOps` | `core` 里的环境变量与文件系统调用；`presentation → ports` / `presentation → kernel` | **已完成**（基线 9 → 7 条） |
| **3** | **切边 A1 + A2**：`slash` → `kernel::path`（纯机制）；**拆册子**——`Prompts` 只留提示词文本，`SystemTools` / `ToolBook` 由 `Core`、协作会话与讨论直接持有 | `refs → workspace`；`prompt → roles`；`prompt → schema`；**并解开 `prompt ⇄ tools` 本质环** | **已完成**（核心环 16 → 14 个模块） |
| **4** | **切 `workspace → schema`**：`builtin_tools` 从 `Sandbox` 移到 `MemberTools`（`Sandbox` 从未读它，是死重） | `workspace → schema` | **已完成**（核心环 14 → 12；`workspace` 只剩 `→ prompt`，属**合法业务间依赖**，不必切） |
| **5** | **其余三条数据聚合**：① `chain.rs` → `kernel`（**一次切断 `events → chain` 与 `collab_state → chain`**，即未来的 `session ⇄ collab`）；② `Tier` → `kernel/types`（切断 `providers → exec`，即 `registry ⇄ workspace`）；③ `history → exec` **判定为合法业务依赖，不切**（会话元信息本来就要记执行选型） | `events → chain`、`collab_state → chain`、`providers → exec` | **已完成**（核心环仍是 12——`chain` 本就是叶子，切掉的是**未来的能力级环**） |
| **6** | ~~拆 `ports.rs`~~ **作废**：实测每条 trait 引用的类型**恰好都是它自己能力的**（`FenceHost`/`ToolRunner`→`fence`、`PackageSource`/`Workspace`→`workspace`、`ChatGateway`→`providers`、`EnvelopeRepair`→`envelope`、`HistoryStore`→`history`、`ModuleSource`→`module`、`PromptSource`→`prompt`、`SettingsStore`→`providers`）。所以**拆文件不切边**——枢纽是**被搬空的**，随各能力一起走 | — | **作废**（并入阶段 B） |

**阶段 B：搬能力（按切边后的图重排）**

| 批次 | 目标 | 前置 | 状态 |
| --- | --- | --- | --- |
| **7** | **prompt 能力落位**：`capabilities/prompt/`（`api` / `ports` / `domain`）；`PromptSource` 随能力迁出 `core/ports.rs` | 4, 5 | **已完成**（门禁已认 `capabilities/` 层、`kernel` 不依赖能力、以及「业务之间只经对方的 `::api`」；`ports.rs` 枢纽 8 → 7 条边；1 条反向边 `→ core::envelope` 记入基线，随 `envelope` 落位清零） |
| **8** | **registry 能力落位**：`capabilities/registry/`（`api` / `ports` / `domain`），`providers.rs` + `agents.rs` 一起搬出；`SettingsStore` 与 `DEFAULT_LLM_TIMEOUT_SECS` 随之下沉 | 5 | **已完成**（**呈现层 3 条豁免自动过期**——它现在走 `registry::api`；`registry` 进环，环 12 → 11；2 条反向边 `→ core::{history, module}` 记入基线） |
| **9** | **llm 能力落位**：`capabilities/llm/`（`api` / `ports` / `domain/envelope`）；`envelope.rs` 随能力搬出 | 8 | **已完成**（`prompt → core::envelope` 那条基线豁免**自动清零**；⚠️ 环从 11 涨到 15——见批次 10 的前置项） |
| **10** | **workspace 能力落位**：`module` / `packages` / `exec` / `workspace`（沙箱数据）+ 三个端口；**前置（批次 9 暴露的桥）已先切**：把协议→文案的映射移进 `llm`（模板仍留 `prompt`） | 6, 9 | **已完成**（`prompt` 零外部依赖、彻底脱环；环 15 → **12**；两处自动清零：`registry → core::module`、`presentation → core::exec`；`HostProbe` 下沉 `kernel/host.rs`） |
| **11** | **tools 能力落位**：`capabilities/tools/`（`systool` / `patch` / `schema` / `roles` / `fence`）+ 三个端口；钉死 §3.4（实现锁内部、机制留端口、产出事实不落盘） | 10 | **已完成**（环 12 → **8**；批次 10 的两条反向边自动清零；1 条新反向边 `→ core::session`；`core/ports.rs` 只剩 `HistoryStore`） |
| **12** | **session 能力落位**：`capabilities/session/`（`session` + `history` + `events`）+ `HistoryStore`。**12a** 循环反转切掉 `engine ⇄ session`；**12b** 提取能力 | 11 | **已完成**（**反向边基线清空**——没有任何能力再依赖 `core`；环 7 → **5**，且 5 个节点全是能力、`core` 完全脱环；`core/ports.rs` 消失） |
| **13** | **rewind 归位**：纯行 / 事件算术进 `capabilities/session/domain/rewind.rs`；`Core` 保留编排（`rewind` / `rewind_children` / `rebuild_session`） | 12 | **已完成**（`marks` 归属方案 A 照旧；无新增依赖边） |
| **14** | **collab 能力落位**：`capabilities/collab/`（`collab` + `collab_state` + `engine`）。**执行顺序调整**：先做 14 再做 13——`rewind` 的回档重建要同时碰 `session` 与 `collab` 两侧，两边就位后才切得干净（已获用户同意） | 12 | **已完成**（环不变——`capabilities/collab` **不在环里**：没有任何它依赖的能力反过来依赖它；`core/` 只剩 `api.rs` + `mod.rs`） |
| **15** | **断环（已完成）** → 能力图零环；**收口 1（已完成）**：入站词汇归 `core/api.rs` → 基线全空；**收口 2（已完成）**：`intent.rs` 规则下沉；**收口 3（已完成）**：`Action`/`Acted` 与分发收进 `core::api`（`SessionOps::act` 默认方法）、`split_names`/`NO_AGENTS` 归 CLI、**`intent.rs` 删除**、`presentation/` 拆成 **`cli/` + `web/`** 两个独立顶层目录（静态资源随 `web/assets/`）；**收口 4（已完成）**：`main.rs` 拆四件事——组合根留 `main.rs`、机器可读探针进 `diagnostics/`、围栏守门进程进 `guard/`（**第二个程序入口**）、路径机制下沉 `adapters/root.rs`；门禁新增**入口层**并禁止任何人依赖它 | 14 | **全部完成** |

| **16** | **适配器归位**：`adapters/` 里**能力私有**的 20 个实现 → 各能力 `detail/`；`adapters/` 只剩 `log` / `host_probe`（内核端口）与 `root`（入口层） | 15 | **已完成**。**重要发现**：搬进能力后暴露出一处被"适配层"挡住的真环 `tools ⇄ workspace`——`fs_modules` 校验清单时反向问了 `tools` 的保留名。解法：**校验归清单主人（workspace）、名字空间归工具（tools），保留名表由组合根装配期注入** |
| **17** | **编排与状态归位（能力服务化）**：每块状态连同写它的操作一起搬进该能力的 `service.rs`。**只搬"状态 + 写它的操作"，不搬脚本**（§2.4） | 16 | **已完成**。`registry`：`service.rs` 持四份 yaml 与三个端口，`core` 删掉 `settings` 与 17 个方法。`prompt`：`service.rs` 把 `Prompt` 面挂在册子上，`core` 只剩 `Arc<dyn Prompt>`，`collab`/`Sandbox`/`ProcTools`/`AgentSession` 的深拷贝改共享 `Arc`。`tools`：`Tools` 面 + `SystoolsSource` 端口，`core` 只剩 `Arc<dyn Tools>`。`core` 至此**不再有别人的状态字段**（剩下的是端口与它自己的会话中心）。**留下的判断失误（批次 20 修正）**：当时只按"状态 + 写它的操作"找，把 `workspace` / `session` / 代拟记成"无状态可搬"——**漏了"不写状态、但要用状态与端口去编排"的那一层**（工作区扫描/沙箱/报告/文件视图、历史 CRUD、回档的会话内部分、代拟、工具环境装配）。这些用例仍留在 `core`，批次 20b/20f 按 R1/R12 归位 |
| **18** | **入站接口归位**：`core::api` 的能力接口 → 各能力 `api`；`cli` / `web` 改经各能力的**声明面**；`contracts.md` 的路由表与 `tests/api.rs` 跟着改 | 17 | **已完成**。`RegistryOps` → `registry::api`、`HistoryOps` → `session::api`、`WorkspaceOps` → `workspace::api`（新，收 `roster`）；**留在 core 的三个各归其位**：`SessionOps`（会话中心：会话生命周期 + 动作分发 + 文件视图 + **`session_views`**——"在世会话 × 历史的并集"只有它两个都知道）、`CoreOps`（`runtime_report` / `suggest_models`：**编排脚本**，§2.4）、`LogOps`（埋点门面；门禁只许 `core::api` 或能力 `::api`）。`DiscoveryOps` 因此解散。**①已收口**：`RegistryOps`（队列面，全 `&self`）与 `Registry`（能力面，写取 `&mut self`）**保持两个 trait**，理由写进 `registry/api.rs`——单写者是编译期事实（`&mut self`），而呈现层持的是可克隆句柄、队列独占在核心线程那一侧（R4/R5）；收口判据不是"并成一个"，是"**定义归位**"。**②仍未达成**：`ChatGateway` 由 `core` 与 `RegistryService` 各持一份 `Arc`，§五.5「端口对象恰好一处被持有」 |
| **19** | **测试按业务分区（R10）**：`tests/core.rs`（8408 行）拆开，分区与 `capabilities/` 对齐 | 18 | **已完成**。`src/tests/core.rs`（161 个用例）按**用例钉住的不变式归属**切成 `kernel` / `prompt` / `registry` / `llm` / `workspace` / `tools` / `session` / `collab` + 留在 `core.rs` 的**应用服务**用例（会话中心与编排）；非测试脚手架（替身 + 造会话/造名单辅助）搬进 `builders.rs`，原 imports 集中成 `prelude.rs`。**最大文件 1877 行**（`tools.rs`），全部 ≤ 2000；用例数不变（287 passed）|

**阶段 C：边重新划线（只准 `api` + 能力内部水平分层）**

| 批次 | 目标 | 前置 | 状态 |
| --- | --- | --- | --- |
| **20a** | **门禁改判据**：① 跨能力**只准引用 `::api`**（`::ports` 不再允许，**任何非入口层**引用别人的端口都算违规——`core` 同样受限，R12）；② **不得给别的能力的类型写 `impl`**（`impl Trait for Type` 只看 `Type`，实现别人的 **api trait** 是正当的队列代理）；③ `capabilities/<c>/domain/**` **不得引用任何 `ports`**；④ `api.rs` **不得把本能力的 `ports` 再导出去**（入站用例面，R12） | 19 | **已完成**：四条判据已进 T0 结构审查（`apiOnly` 收紧 + 新增 `apiPorts` / `foreignImpl` / `domainPorts`），现状 **14 条**违规进基线：`apiOnly` 7（collab 3 / session 1 / core 3）、`domainPorts` 5、`apiPorts` 1（`llm/api.rs` 的重导出壳）、`foreignImpl` 1（collab 给 session 的类型写 impl） |
| **20b** | **每个能力补 `service.rs`，成为自己端口的唯一持有者**（R12）：先 `llm`（它的端口现在被 `registry`/`conductor`/`collab` 拿），再 `workspace` / `tools` / `session`；`api` 从"重导出壳"变成**入站用例面**（端口 trait 不再进 `api`） | 20a | **进行中**。**`llm` 已完成**：`api` 收下通道与协议词汇（`Chat` / `BoxedChat` / `Msg` / …）+ **`Llm` 用例面**；`ports` 只剩 `ChatGateway` / `ModelCatalog` / `EnvelopeRepair` 三条出站端口；新增 `llm/service.rs`（`LlmService`，唯一持有者）；`registry` / `conductor` / `collab` / `session` 全部改经 `llm::api::Llm`（`MemberTools.repair` → `MemberTools.llm`）；**基线 `apiPorts` 清零**。**`workspace` 已完成**：三个出站端口（`ModuleSource` / `PackageSource` / `Workdirs`——原名 `Workspace` 让给能力面）收归 `workspace/service.rs`；`api::Workspace` 立用例面；`conductor` / `collab` 改经它；基线里 workspace 的三条（apiOnly 2 + domainPorts 1）清零。**`tools` 已完成**：三个出站端口（`ToolRunner` / `SysIo` / `FenceHost`）收归 `tools/service.rs`（`ToolsService`，同时实现 `Tools` 与 `ToolExec`）；`ToolOutcome` 从 ports 归 `domain`（它是事实不是端口）、由 api 导出；`conductor` / `collab` 的 `tools`+`io` 两个字段 → 一个 `Arc<dyn ToolExec>`；`MemberTools.runner`/`io` → `MemberTools.tools`；引擎三处调用改经执行面。**`session` 已完成**：`HistoryStore` 收归 `session/service.rs`（`SessionService`）；`api::History` 立直连面（与端口一一对应——会话落盘没有别的不变式可编排，这一面的价值是唯一持有者）；导体的 25 处 `self.history.*` 零改动改道（方法名一致，只换字段类型）。**20b 四家（llm / workspace / tools / session）全部完成，基线 `apiOnly` 与 `apiPorts` 清零**；余下 `domainPorts` 1 条与 `foreignImpl` 1 条留 20f |
| **20c** | **协调业务独立**：`core` → `capabilities/conductor/`（会话在世表 + 命令队列 + 运行态 + 生成驱动 + 跨会话回档）；**只持各能力的 `api` 面**；`Ops` / 事件台 / 队列代理随它 | 20b | **已完成**：`Core` → `Conductor`、`CoreHandle` → `ConductorHandle`、`CoreOps` → `ConductorOps`；门禁的 `core` 层与节点退休（呈现层只认各能力的 `::api`）；**`Core` 这个类型不再存在** |
| **20d** | **kernel 业务化 + `adapters/` 归零**：`kernel` 补齐 `api`/`ports`/`domain`/`detail`；`log.rs`/`host_probe.rs` → `kernel/detail/`、`root.rs` → 入口层；**`chain` 搬出** → 独立业务 `taskchain`（纯领域：`api` + `domain`，无端口、无 `service`） | 20c | 未开始 |
| **20e** | **抽跨能力同形重复**（R13）：**拟名单**独立成业务（`core::suggest_models` 与 `collab::draft_slate` 两条合并）；**核心操作回路**的 5 处调用侧包装收进一处 `api` | 20d | 未开始 |
| **20f** | **内部水平分层收口**：`collab/{engine,collab}.rs` → `service/`；`session/domain/session.rs` 拆"纯簿记"与"回合驱动"；`tools/domain/systool.rs` 拆"纯规则"与"执行编排"；**全能力统一 `domain/` = 纯逻辑（不持端口、不做 IO）** | 20e | 未开始 |
| **20g** | **收口**：删本文，把当前状态收回 [ARCHITECTURE.md](../../ARCHITECTURE.md) | 20f | 未开始 |

**规模不是硬验收**：拆到「能安全验证」为止。`core/mod.rs` 与 `collab/domain/engine.rs` 的进一步拆分随批次 20 暴露的接缝走，不为凑行数而拆。

**豁免清零判据**：`tests/dependency-baseline.json` 的**全部数组清空**（`reverse` / `presentation` / `coreCycles` /
批次 20a 收紧的 `apiOnly` 与新增的 `apiPorts` / `foreignImpl` / `domainPorts`），且 `Core` 这个类型不再存在（→ `conductor`）。
门禁对**新增**与**过期**都报失败，所以销账不靠自觉——拆掉一条边不删条目，构建就红。

### 4.3 每个批次的完成定义（DoD）

一个批次只有**全部满足**才可销账：

1. 新目录建立，代码迁入，**旧路径已删除**（不留转发壳）；
2. `cargo test` 与 `node run-tests.js` 全绿（T0 质量门禁 + 业务测试都过）；
3. 依赖方向门禁通过，**该批次的基线豁免已删除**；
4. 该能力的测试已按 §4.4 落到同构文件，单文件 ≤ 2000 行；
5. 文档同步：`module-map.md` 对应行已改；受影响的门户/细则已改（R8）；
6. §4.2 表状态改为「已完成」，并写明批次号。

### 4.4 测试迁移规则

- **分区同构**：业务 `capabilities/<name>/` ↔ 测试 `src/tests/<name>.rs`（一个能力一个文件；超 2000 行再拆目录）；`src/tests/conductor.rs` 只留**协调业务自己的**用例（会话中心与跨能力编排），其余按能力搬走（批次 19 已完成）。
- **搬家不改断言**：拆分批次内只移动测试与改路径，**不动断言**。要改断言语义 → 另开批次并写明理由。
- **消除测试专用入口**（R11）：`Core::single_say`（`#[cfg(test)]`）这类"测试路径 ≠ 生产路径"的双轨，
  在 `session` / `collab` 批次里改为走生产入口（`CoreHandle`）。
- ~~**替身跟着端口走**：`src/tests/doubles.rs` 按端口归属拆到各能力~~ —— **实测改判（批次 19）**：替身与装配脚手架**留在共享文件**（`doubles.rs` 端口替身 + `builders.rs` 测试装配）。理由：它们本来就不属于某个能力（同一份内存装配被多个能力的用例复用），按端口拆散只会把同一份装配复制若干份、并把"改一处替身要改几处"引回来。端口的真实适配器覆盖范围见 [doubles.md](../testing/doubles.md) 三。
- **每批次保留一条行为不变验收**：至少一条端到端用例证明该批次"只搬家、没改行为"。

---

## 五、验收标准

1. **业务全在 `capabilities/`**：每个能力有 `api.rs` / `domain/` /（有 IO 或可替换点的才有 `ports.rs` / `detail/` / `service.rs`）；
   **纯领域业务**（如 `taskchain`）只需 `api` + `domain`；**`adapters/` 目录消失**（内核端口实现进 `kernel/detail/`，入口层机制留入口层）。
2. 依赖图为**无环**，由 T0 门禁机器判定，**零豁免**。
3. **协调业务 `conductor`**（不是特权层）：持"会话在世表 + 命令队列 + 运行态 + 生成驱动 + 跨会话回档 + 审查关卡推进"，
   **只经各能力 `api` 编排**；登记处 / 工作区 / 历史 / 回档的会话内部分 / 代拟 / 工具环境的**用例都归各自能力**。
   协调业务与其它能力受**同一条** R1/R12 约束（门禁同等对待）。
4. 前端（`cli/` + `web/`）只 `use` 各业务的**声明面**：零 `use ...::ports::`、零 `use ...::domain::`、零 `use ...::detail::`。
5. **端口只由定义它的能力持有**（R12）：跨能力一律不引 `ports`；会话对象也只带 `api` 面与已解析的纯数据。
   组合根只做两件事：`new` 出各能力的适配器并注入**它自己**的端口、以及把各能力的 **`api` 面**交给 `conductor`。`::detail` 只有入口层碰。
6. 运行态**只有一份真相**（`kernel/jobs`）。
7. 测试按能力分文件（`src/tests/<能力>.rs`），单文件 ≤ 2000 行；**测试入口 = 生产入口**。
8. 架构文档（`ARCHITECTURE.md` + `module-map.md` + 相关细则 + `AGENTS.md` 路由表）与代码一致，无过期描述。
9. 本文删除。
