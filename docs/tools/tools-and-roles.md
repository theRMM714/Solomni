# 工具、流程段与角色

> 定位：系统工具、流程段与角色（身份）与"谁能用哪些工具"的唯一权威。
> 效力：规范｜核对：2026-11-25｜依据：`systools/tools.yaml`、`systools/loops.yaml`、`src/units/business/session_model/domain/mechanism.rs`

## 适用与判据
- 声明系统工具、流程段与角色和越权校验时以本文为准（`docs/tools/tools-and-roles.md`）。
- 模块工具在各自 `module.yaml` 声明（`MODULE_SPEC.md`），本文只管系统工具。
- 违反由 T0 结构审查与 `src/tests/detail/` 的悬空引用用例判定。

## 一、两张表与字段

- 工具**是什么**在 `systools/tools.yaml`（一句话说明、参数契约、能不能并发）；**谁能拿到它**不在这张表里——由若干**工具来源**各自交 id（见 §二）。
- 来源的端口在 `src/units/business/session_model/domain/tools.rs`（`ToolSource`：`id` + `offer(TurnCtx)`）；会话类型表（`systools/profiles.yaml`）只声明**挂哪几个来源**与**系统工具**的 id，`systools/names.yaml` 是布局的第三份（`data/session/<工作>/` 的保留目录名），与工具面无关。
- 代码里**不得**出现"哪个角色能调哪个工具"的名单：只有三个动作——**来源按上下文自己交 id**（`src/units/business/agentloop/domain/sources.rs` 的五个实现）、**装配机把来源交的 id 去重成面**（`src/units/business/agentloop/domain/assemble.rs` 的 `assemble`，唯一实现）、**按面校验调用**（`src/units/business/agentloop/engine/tools.rs` 的 `face_has`）。

```yaml
tools:
  read:
    desc: 读取文本文件（UTF-8）。
    parallel: true        # 同一回复里的多个调用能否真的并发跑
    params:               # 既用于生成声明，也用于校验调用参数
      path:
        type: string
        required: true
        desc: 要读取的路径（用【工作环境】的写法）
```

- **动作表只说"这个工具长什么样"**：没有 `capability` / `publish` / `callers` / `effect` / `reach` / `scope`。一张表同时回答"谁能调"在来源变多以后必然漂（同一动作既给人、又给会话内若干来源），所以谁能拿到归**来源**，授权归**权限端口**。
- **没有工具卡**：权限侧不产生裁决卡，也没有逐次「允许 / 拒绝 / 本轮放行」的确认链路；需要人定夺的是核心各关卡（建会话、验收、定版）与工具层的围栏裁决（授不上必要落点时问一次，见本文件「授权面」）。
- **人的命令面单独声明**：`src/units/business/tools/domain/systool.rs` 的 `HUMAN_ACTIONS` 列出人经呈现层能用的动作（`is_human_action`），它是呈现层的口径、不是授权；不在其中的动作被人调用会如实拒绝（「这个动作只在会话里用」）。
- **共享区写权不再是逐工具标记**：谁能发布 `work_*` 由 agent 的能力组 `shared-write` 决定（`workspace` 来源只在有工作区且带这个能力时交那 11 个 id）。
- **列目录是"核实"的前提**：`read` 只读文件、`grep` 按模式找行，**没有 `list` 就确认不了"资料齐不齐"**；`read` 遇到目录会如实引导到 `list`。
- **`grep` 的默认过滤是检索面口径**：目录递归时按 `.gitignore` + 版本库目录里的 `.workignore`、点号开头的隐藏文件、二进制（前 8 KiB 含 NUL）剪枝，不跟符号链接；`hidden` / `no_ignore` 是逃生口，显式给单个文件路径时不过滤。过滤名单只有一份，与「哪些文件算工作区内容」共用，维护入口是 `work_ignore`（见 [workspace](../workspace/README.md)）。
- 参数不合法 → **如实拒绝并落工具行**。

```rust
// 角色：身份（提示词 + 是否核心通道），声明在 src/units/business/session_model/domain/mechanism.rs
Role { id: "discussant", prompt: "roles/discussant", core: false }
// 角色位：某一段里的这个角色要几员、交哪些机制动作
Cast { role: "discussant", min: 2, tools: &["say", "agree", "leave", "ask"] }
// 流程段：回合身份的来源；builder 是实现名（写了才能当流程串），cast 按声明顺序（核心位与成员位混排）
Loop { id: "discuss", builder: Some("discuss"), cast: vec![Cast { role: "core_proxy", .. }, Cast { role: "discussant", .. }] }
```

- **角色位 = 绑在段上的身份位**：`discussant` 只在 `discuss` 段出场、`orchestrator` 只在 `execute` 段出场；人数与机制动作都在角色位上，**同一个角色在不同段可以交不同的工具**，所以不需要"阶段 → 工具"的第二张表。
- **流程段 = 回合身份的来源**：写了实现名 `builder` 的段能当流程串进会话（`discuss` / `execute` 各自绑一个实现），没写的段是没有流程的会话坐的**静止段**（`standard` 坐 `solo`、`proxy` 坐 `core_proxy`）。段与角色声明在 `systools/loops.yaml`；会话类型表（`systools/profiles.yaml`）的 `flows` 列流程段，`loop` 写静止段。
- **工具只认「段 + 角色」**：落盘的 `SessionSpec` 带 `loop_id` 与 `role`，装配期按这两个值查出角色位再拍平成 `loop` 来源要交的 id；换段时两者一起换（`Identity`），不许只换一个。文件面与执行面归 agent 的能力组，模块面归 agent 装了哪些模块，共享区面归有没有工作区与 `shared-write`，系统工具归会话类型——角色位只声明人数与机制动作。
- **角色 id 全局唯一**：流程段只引用 id，不复制声明；装配期 `loop_check::problems` 断言角色在册、角色位交的动作都在总表里。
- **核心角色是数据不是特判**：`core: true` 由会话的核心通道出演（核心代理 / 计划 / 验收），不进成员表；`core: false` 才是成员会话。哪一段里有核心位由该段的 `cast` 决定，这条约束由 `loop_check::problems` 断言。
- **不做角色继承**：重复列几个 id 没有代价，继承会引入隐式耦合。

## 二、可用工具面（运行时合成）

```text
本回合工具面 = dedupe( ⋃ 来源.offer(这一回合的上下文) )   # 来源由会话类型声明，去重按来源顺序
```

| 来源 | 发哪些 | 判据 |
| --- | --- | --- |
| `agent` | `read` `list` `grep`、`write` `edit` `patch` `delete`、`shell` | agent 的能力组 `fs-read` / `fs-write` / `exec`（`agents.yaml` 的 `abilities`；不写 = 全给） |
| `module` | 该 agent 的模块工具，限定 id `<模块>.<工具>` / `<模块>.<服务>.<操作>` | agent 装了哪些模块（`agents.yaml` 的 `modules`） |
| `loop` | 当前角色位的机制动作（`say` / `submit_report` / `plan` / `node_verdict` …） | 这一回合的段与角色（装配期用 `(loop_id, role)` 在段表里查出 `Cast.tools`，之后来源只持有这份 id 清单） |
| `workspace` | 11 个 `work_*`（`workspace::api::WORK_TOOLS`） | 有工作区，且能力组含 `shared-write` |
| `profile` | 会话类型声明的系统工具（代理类型的六个会话控制动作） | 会话 meta 里的 `system_tools`（建会话时按类型表展开） |

- 会话类型表（`systools/profiles.yaml`）的 `sources` 决定挂哪几个：`single` = `[agent, module, workspace]`、`collab` = `[agent, module, workspace, loop]`、`proxy` = `[profile]`（六个系统工具）。
- **来源只回答"它在不在场"，不回答"准不准"**：越权（面里没有）由装配后的面拦，准不准归权限端口；这条分界让"可见但需批准"能作为同一个端口上的第三种结论加进来（当前只有准 / 不准）。
- **装配不做解析**：面里只有 id，执行时按 id 找实现（模块 / 服务 / 系统工具各一处），找不到如实回执「没有这个工具」。

- **为什么 `shell` 在 `agent` 来源的读面里**：围栏恒为「私有沙箱 rw + 模块目录只读 + 主机只读根只读」，共享区主副本永不进容器——它改不到共享区，发不发不构成安全问题（`src/units/business/tools/domain/fence.rs`）。
- **写面不看角色**：能力组只属于 agent，同一个 agent 换到哪一席，交出的文件面都相同。核心席（`planner` / `orchestrator` / `core_proxy`）只做核实与判定，压住它调写面与发布面的是**授权端口**（按流程段声明的抑制表判），不是来源。
- 越权防线分两段：**来源合成的面**决定这一刻有哪些工具（在不在场），**授权端口**决定其中哪些还能用（准不准）。两段缺一不可：面里没有的调用一律拒绝（`face_has`），面里有而这一段抑制的由端口拒绝——两道判定都在 `src/units/business/agentloop/engine/tools.rs` 的 `run_branch` 里，先于任何执行。

- **注入口径（唯一）**：**总表不进提示词**。每回合由核心查"这一回合的身份能用的工具"：信封通道把说明与参数作为一条系统消息注入；原生通道由**声明槽**下发（名字 / 说明 / JSON Schema），不再重复文字清单，只保留 patch 这类声明表达不了的写法。
- **回执预算（唯一）**：所有工具回执（read / grep / list / shell / 模块工具 / 核心操作的只读核实）送回模型前受**两级预算**。第一级是单条静态上限 16k 字符（开头约 1/4、结尾约 3/4），超出时把正文换成「开头 + 结尾」。第二级按**本批**剩余上下文收紧：`ResultBudget` 取 `min(16k, (窗口 - 已用 - reserve) × 4 × 50% / 本批调用数)`，`reserve = min(2000 token, 窗口 / 10)`；算出的每条上限低于 1000 字符即视为上下文枯竭，**整批预检失败、任何工具都不执行**（避免 shell 已产生副作用却看不到回执）。只要没枯竭，生产者在生成回执时就按本批上限截断：read 只显示能放下的行，并只把实际显示的行记成观察事实；shell / 模块工具与核心操作的只读核实同受此限；会话层再用同一预算兜底，已按更大上限截过的回执允许二次收缩。截断加 `result.truncated`（原始字符数、实际保留数、本批上限）与 `result.truncated.context`（提示先压缩会话或分页读取）两条 `Note`；文件类续读给 read 的 offset/limit，执行类给「缩小命令输出 / 写到文件再 read」。实现唯一在 `src/units/business/tools/domain/budget.rs`，本机与虚拟机档都经过它；read 的续读 offset、shell 的状态行 / 末行都在尾部，因此保留。

| 回合 | 角色 | 这一份是什么 |
| --- | --- | --- |
| 讨论回合 | `discussant` | 读面 + `say` `agree` `leave` `ask`；`module` 来源只在成员装了模块时给（讨论席通常在 `collab` 的 `execute` 节点才干活） |
| 执行回合·单 agent 工作 | `solo` | 读面 + 写面 + 发布面 + 该 agent 的模块工具（**不发** `submit_report`） |
| 执行回合·协作节点 | `executor` | 与 `solo` 同面，另发 `submit_report` |
| 计划回合 | `planner` | 读面 + `plan` `slate` `verdict` |
| 推进回合 | `orchestrator` | 读面 + `node_verdict` `checklist` |
| 核心代理回合 | `core_proxy` | 会话类型声明的六个系统工具（`catalog_agents` / `create_session` / `send_session_message` / `observe_session` / `read_session_messages` / `control_session`）；`proxy` 类型只挂 `profile` 来源，因此**没有**文件面 |

- 发放与校验是**同一份判据**：列出来的就是这一刻真能调的；没拿到的既不在提示词里、也不在声明槽里，真去调会被如实拒绝。
- 发放只有一处：`src/units/business/agentloop/domain/assemble.rs`（造 `TurnCtx`、按会话类型挂的来源逐个问 `offer`、去重成面，再配说明块与共享区工具执行者）。来源与角色都不合成别人的名单。
- 回合只携带**段与角色**（`session::TurnRun.loop_id` / `TurnRun.role`）：驱动换身份时由会话现拼这一身份的面（`AgentSession::enter_identity` → `MemberTools::enter_identity`），跑完一并还原——同一个会话因此能用两种身份干活（讨论角色说话 / 执行角色干活）。节点 / 单 agent 回合在准备段就把身份装进会话，不靠会话出生时的面。

- **两条硬口径**（`systools/tools.yaml` 是唯一真相）：

- **agent 对系统的每个操作都要有对应工具 + 身份限制**：没有工具的操作 = 契约缺口（例如"要返工哪个节点"必须是 `checklist.items[].rework` 这类**字段**，不能塞进自由文本）（`systools/tools.yaml`）。
- **工具的"后果"写进声明**：会驱动核心动作的工具要写清"这个结论会让系统做什么"。
  - `node_verdict` 的 `ok=false` = 该节点退回待办、点上下文「恢复协作」只重派它；
  - `checklist` 的 `fail` 必须填 `rework` = 要返工的节点 id（`systools/tools.yaml`）；
  - `verdict.clear=true` 才照用户说的开工 / 放行。
- 写了表里没有的 id 或漏填 = 这次判定用不了，核心**要求重填**。

- 身份与环境同理：它们是**会话参数**（`session::SessionParams`），每次调用现渲染成系统消息，不占对话的位置（见 [session-model.md](../session/session-model.md) 的「会话参数与对话分开」）。

- 核心自己的操作（`planner` / `orchestrator`）不走 agent 会话：原生通道由声明槽给出它们的角色的面，信封通道由各自的角色提示词写清载荷形状——总表同样不进提示词。

## 三、越权校验

- **工具面与越权校验都出自来源合成**：

- 讨论回合坐 `discussant`：`loop` 来源只交 `say` / `agree` / `leave` / `ask`；
- 执行回合**按角色**：用户建的单 agent 工作用 `solo`，协作的节点子会话用 `executor`，两者只差回报工具；
- 计划 / 推进回合坐 `planner` / `orchestrator`：`loop` 来源交各自那三个 / 两个动作；
- 核心代理回合坐 `core_proxy`：它的机制动作是空的，工具全来自会话类型的 `system_tools`。

- 代码里**没有任何**"哪个角色能调哪个工具"的名单，加一个工具只改一处（工具总表）；要让它出现在某个面上，由对应来源自己交 id。

- 授权判定的约束：

- **在不在场与准不准是两个端口**：来源只交 id（`ToolSource::offer`），准不准问 `Policy::decide`（`src/units/business/agentloop/domain/policy.rs`），请求带工具 id、交它的来源、当前段与角色。默认实现 `LoopPolicy` 只按段判：段声明抑制哪几组能力（`Loop.deny`，可选 `fs-read` / `fs-write` / `exec` / `shared-write`），这一段里那几组工具就不能调，回执说清是哪一段抑制了哪一组能力。结论有三种：`Allow`（放行）、`Ask`（先问用户，见下一条）、`Deny`（拒绝）。
- **抑制写在段上，不写工具名**：`deny` 写能力组，能力组到 id 的映射只有一份（`policy::ids_of`，与来源发放共用），所以加一个写工具不需要动任何抑制声明。
- **要点头的动作也写在段上，按工具 id 写**：`Loop.ask` 列的是这一段里要用户点头才执行的动作 id——不是能力组（能力组里混着只读动作，按组问会把一次查询变成弹窗）。引擎把它翻成一张 `kind = policy` 的卡（`policy::ask_card`，选项固定 `allow` / `deny`），走会话的裁决通道；**只有答了 `allow` 才执行**，`deny` / 没人答 / 被停止 / 构不出选项 / 这一趟没有可回答的前端，一律当没放行（回执写明这一次没有执行，`src/units/business/agentloop/engine/tools.rs` 的 `gate`）。生产声明现在四个段都是空的。
- **引擎每次调用都过一遍授权**：`src/units/business/agentloop/engine/tools.rs` 的 `run_branch` 先按面判在场，再问 `Policy`，然后才进内置 / shell / 模块 / 核心自有工具四条执行路（授权拒绝不产生副作用、不当表态）；要问的动作在这一步停下来等答复，答了才继续。

- 越权判定的约束：

- **原生模式与信封模式走同一个校验点**；两者**互斥但等价**——等价性一破，回放就与实时不一致。
- 越权调用 → **如实拒绝 + 落工具行**（用户要能看到"它越权了"）。
- **悬空引用**（角色位或会话类型引用总表里不存在的 id）→ **装配期 `loop_check::problems()` 硬失败**（`src/tests/detail/` 锁死）。`SystemTools::problems()` 另管总表自身：id 非空且无空白。
- `--audit=structure` 只比文件与文档结构，不解析这些声明；改角色 / 工具表要跑全量入口（或至少 `src/tests/detail/` 的用例）。
- 可解释性的约束：

- **工具面能被问出来**：`explain_tools`（`session_id` 必填，`agent` / `loop` / `role` 可选）按会话 meta 重算面并回一份报告——每个来源交了哪些 id、合并后的面、每个 id 的授权判定与理由（`Allow` / `Ask` / `Deny`）、面里有但总表与模块工具表都没有的孤儿 id（`src/units/business/agentloop/domain/explain.rs`）。
- **解释与装配同源**：报告走同一条取数（`tool_table` → 角色位动作 → `build_sources` → 逐个 `offer` 再合并，`offers_of` / `merge_offers` 由 `face_of` 与解释共用），所以解释不会与真实放行不一致。
- **拒绝也说得清上下文**：越权（`note_unauthorized`）与未知工具（`check_call`）的回执写明当时在哪个段 / 角色、这一回合的面里实际有哪些 id（`src/units/business/agentloop/engine/tools.rs`），模型据此换来源或改口，不必反复试同一个名字。
- CLI 入口：`explain <会话> [--agent <名>] [--loop <段>] [--role <角色>]` 打印可读行文（头行 → 来源 → 逐 id 判定与理由 → 孤儿），`--json` 打原始报告。

## 四、动作与分发（一张表，两条执行路径）

- **动作表是参数契约的唯一真相**：每条动作声明参数契约，校验按声明做——这一步只有一套规则，不随调用方变；**谁能调用**不在表里（人的命令面见 §一 的 `HUMAN_ACTIONS`，会话内由来源合成的面）。**执行**有两条实现，各在自己的单元里：呈现侧（人）走 `runtime` 的 `ActionOps::act`（`src/units/business/runtime/service/action.rs`，校验 → 只服务人的命令面 → 执行 → 审计四步在一处），模型代理侧走 `ProxyTools::call`（`src/units/business/proxy/service/proxy.rs`，同样是查表 → 校验 → 交宿主，另有幂等账）。两条路径共用同一张表与同一份参数契约，但**不共用同一段执行代码**——把它们写成一处需要呈现层与模型回环互相依赖，会成环。

- **两个适配器**：模型侧（`ToolHandler`，按名字认领）与呈现侧（`POST /api/actions/{id}`、`GET /api/actions` 动作目录）都只负责造一次 `ActionCall`（动作 id + 参数 + 调用者身份）并各按自己的媒介呈现结果——模型侧拿文本回执，呈现侧拿结构化结果。**同一份声明，两种呈现。**
- **读取与视图不进动作表**：会话列表 / 配置 / 文件清单 / 历史是读接口，事实仍只有**事件台**一条来路。
- **人经呈现层调用**（`caller = user`）与**模型经工具调用**（`caller = 角色 id`）用同一份参数声明；人的入口只认 `HUMAN_ACTIONS`；审计各自入账（呈现侧进运行日志，模型侧进可回放的转录工具行）。适配器不各自校验。
- **登记处动作只在人的命令面**：供应商 / 密钥 / 模型 / agent / 设置的写面（`upsert_provider` / `remove_provider` / `discover_models` / `upsert_model` / `remove_model` / `set_core_model` / `probe_model_tools` / `probe_replay_shape` / `upsert_agent` / `remove_agent` / `set_settings`）都在 `HUMAN_ACTIONS` 里、不被任何来源交出——产品级资源默认不开放给会话；`api_key` 只进登记处，不进动作目录、不进审计。读（`GET /api/settings`、`/api/state`）仍是独立读接口。
- **工作区回滚动作也只给 user**：`restore_work_commit`（`session_id` / `commit` / `expect_head`）——非破坏性物化共享区，目标之后的提交仍保留；同一工作子树里有会话在生成时拒绝；成功后由核心落一条可回放的 `notice` 行。
- 提交历史 / 详情 / 文件预览是**只读视图**（`WorkOps::work_log` / `work_commit_view` / `work_commit_file`），不进动作表。
- **guest 只读诊断动作也只给 user**：`guest_diagnose`（`session_id` 必填非空）——诊断虚拟机档会话的 guest：进程表（pid / ppid / 状态 / CPU / 内存 / 运行时长 / 命令与参数）、CPU 汇总、内存、负载、挂载与内核 / 核数；只读、不写 guest 文件（沿用既有 fenced 执行的临时策略文件，helper 读取后即删）、不改会话状态，不是 agent 工具、不进任何工具面（`systools/tools.yaml`）。它在 guest 内走 helper 的只读诊断模式（不套 per-agent 围栏），宿主侧只用已持有的 QGA 通道、不新增外部端口（`src/units/business/vm/service.rs` 的 `guest_diagnose`）。
- **工具面解释动作也只给 user**：`explain_tools`（`session_id` 必填非空，`agent` / `loop` / `role` 可选）——按会话 meta 重算这一回合的工具面并回结构化报告（来源归属 / 授权判定与理由 / 孤儿 id）；只读、不改会话状态、不进任何工具面、不替模型决定该用什么工具。
- **常驻服务的生命周期动作也只给 user**：`control_resident`（`start` / `stop` / `enable` / `disable`，参数 `module` / `name` / `action` / `lease`）——agent 用已启动的实例，不负责拉起；服务清单是独立只读视图（`ResidentOps::services`，CLI `resident`）。会话删除时按租约回收该会话拉起的实例。
- **已启动常驻服务的操作也走模块动作**：id = `module.<模块id>.<服务名>.<操作名>`（静态模块工具优先，其次常驻服务操作）。
  - 人可直接调（`ResidentOps::call`）。
  - agent 侧：已就绪服务的操作以 `<服务名>.<操作名>` 进成员工具面（原生声明与信封清单都列）；原生声明的线上名把点换成下划线并带模块前缀。
  - 参数契约用**服务给的原样**：JSON Schema 原样进原生声明，渲染成参数说明进信封清单；与静态模块工具的参数合并进同一个「模块工具参数」段，标题只出现一次。
  - 核心**不预校验**服务参数——服务是权威，错了由它如实回绝。
  - 调用经 `ResidentOps::call` 转给常驻服务；**未启动的服务不出现**（操作是跑起来才发现的）。
  - 就绪服务的操作同样进**动作目录**（`GET /api/actions` 对 user 可见），Web / CLI 的动作面板因此不必另建一份清单。
- **隐秘字段的设置只给 user**：`set_secret` / `clear_secret`（`module` + `name`，值 `value`）——值只落 `data/.home/` 的 `secrets.yaml`，不进转录 / 日志 / 命令行，永不回显；审计只记动作 id 与成败，不记值。
- **模块工具也是动作**（动态动作，来自清单）：id = `module.<模块id>.<工具名>`。人可直接跑（无会话，围栏按模块目录 + 可选工作目录派生）；会话里由成员循环执行（同一份 `module.yaml` 声明、同一个 `ToolExec::run_module`）。两条路径的审计记录同一处格式化（`action_audit`），只是各自进自己的账本：呈现层进运行日志，模型侧进**转录工具行**（可回放）。**缺运行包 = 不执行**：两条路径读同一把尺子（`runtime_report.missing`），目录里如实标不可用，不静默降级。

## 五、执行类系统工具：shell

- `systools/tools.yaml` 的 `shell` 是唯一的执行类系统工具，参数里没有任何范围旋钮；它不新建执行机制：host 走 kernel 的 `ProcessRunner`，VM 走 guest helper。

- 参数：`command`（必填）、`local_ports`（可选，数组，最多 8 个、元素 1–65535：本机档下让容器里的 `127.0.0.2:<端口>` 经中继到达宿主的 `127.0.0.1:<端口>`；清单外连不上，产品自己监听的端口一律拒——判据是监听者 pid 在本产品进程树里、或它跑的就是本产品这个可执行文件，不靠端口表，见 `src/kernel/detail/container/windows/relay.rs`；只有 Windows 能落，其它平台与虚拟机档给了就拒绝执行，见 `src/kernel/detail/confine/job.rs`）、`timeout_secs`（缺省 120，最大 600；只约束命令子进程，从启动命令起算——Windows 首次为会话准备围栏（建容器、写 ACL）不计入，可能额外耗时，截止时间在 spawn 之后才起算，见 `src/kernel/detail/process.rs`）。cwd 固定私有沙箱；无交互 stdin / TTY；输出超限由回执预算统一收口（16k：约 4k 开头 + 约 12k 结尾，按完整行取整；单行超长时按字符切，见 `src/units/business/tools/domain/budget.rs`）；命令上限 8000 字符。
- **别用 Unix 重定向惯用法**：Windows 宿主的 shell 是 PowerShell，`1>/dev/null` 会把 `/dev/null` 当成路径（实测报 `Could not find a part of the path ...`，其中盘符是视图盘）；丢弃输出写 `| Out-Null` 或 `1>$null` / `2>$null`。POSIX 宿主与 VM guest（`/bin/sh`）照旧可用 `>/dev/null`（围栏基线对该节点例外给写权，见本条末）。
- **回执结构**（正文与结束说明由 `src/kernel/domain/receipt.rs` 装配，落点与耗时由工具层补；模型看到的文本由 `src/kernel/detail/i18n.rs` 按源语言渲染）：正文（stdout；有 stderr 时折成 `[stderr]` 段）在 `output`，结束状态与落点是 `Note`（稳定码 + 参数 + 轻重），模型通道把说明逐行接在正文后，于是看到：正文 / 空行 / 状态行 / 末行。末行是 `[shell] cwd <本环境写法> · 耗时 <N> 秒`，围栏准备 ≥1 秒时另起一行 `[shell] 围栏准备 <M> 秒（已含在耗时里）`（`N` 是整次调用墙钟，含围栏准备）：
  - 状态行四种：`[shell] 退出码 N`、`[shell] 超时（N 秒）：工具进程与它拉起的整棵进程树已被终止`、`[shell] 用户停止：工具进程与它拉起的整棵进程树已被终止`、`[shell] 退出码未知（被信号结束）`。
  - 末行恒为 cwd·耗时、上一行恒为状态行：两条都在 = 这次调用真的跑过；都没有 = 没跑（围栏拒绝 / 无租约 / helper 起不来 / 共享同步失败），此时不补落点行（判据是说明里有没有 `shell.*`，`src/units/business/tools/domain/shell.rs` 的 `did_run`；没跑成的调用仍有结构化事实，界面据此显示"最近一次尝试"）。
  - 截断由唯一的回执预算在送回模型前施加（`src/units/business/tools/domain/budget.rs`）：只切一次、只留一组说明；状态行与 cwd·耗时（说明）不随正文被切（`src/kernel/domain/receipt.rs` 只装配、不截断）。开头 / 结尾**按完整行取整**（单行超长时退化为按字符切），说明报的是实际保留字符数。
  - 退出码是**整条脚本**的收尾状态：脚本显式 `exit N` 时就是 N；Windows 宿主上否则取**最后一次原生命令**的 `$LASTEXITCODE`（`src/kernel/detail/process/windows.rs` 的 `PS_TAIL`），脚本中间失败、后面又跑成功的内置命令不会把它清零——所以 `cmd /c exit 3; echo done` 的退出码仍是 3（PowerShell 自身不传播原生进程码，这条尾行是让中间失败可见的唯一机制）；POSIX 取脚本最后一条命令的退出码。要看单条命令的退出码按平台写：POSIX（Linux / macOS 宿主与 VM guest 的 /bin/sh）用 `cmd; echo exit=$?`；Windows 宿主跑 PowerShell，`$?` 是**布尔**（上一条命令成功与否）不是退出码，退出码看 `$LASTEXITCODE`，写 `cmd /c exit 3; echo exit=$LASTEXITCODE`（由原生命令写、cmdlet 不写；首个原生命令之前它是空而不是 0）。要让「任一关键步骤失败就整体失败」，在关键命令后显式判：`if (-not $?) { exit 1 }` 或 `exit $LASTEXITCODE`（`cmd || exit $?` 这种 bash 写法在 PowerShell 里不成立——`exit` 是语句不是管道命令）。
  - **降级错误等于放弃退出码**：脚本里用 `-ErrorAction Continue` / `SilentlyContinue`、把 `$ErrorActionPreference` 改成 `Continue`、或在 try/catch 里吞掉错误后，失败只进 stderr、包装器看不到它——退出码照常是 0（`src/kernel/detail/process/windows.rs` 的 `PS_TAIL` 只看 `$LASTEXITCODE`，而 cmdlet 不写它；`src/kernel/detail/process/tests.rs` 的 `generated_shell_script_fails_on_unhandled_powershell_errors` 钉住这个口径）。要靠退出码判成败就别降级；要容错就在 try/catch 里自己判并 `exit 1`。
  - 超时与用户停止没有命令退出码（`src/units/business/tools/domain/shell.rs` 的 `exit_code` 从说明里取，取不到就是 None），只用状态行表达，不伪造 124 / 143。
  - cwd 用本环境写法：本机档 = 视图盘打开时的视图写法（**视图写法目前只用于 shell（cwd == 私有沙箱）的容器 cwd 与程序名；模块工具沿用宿主 cwd，不参与视图盘收敛**）（如 `S:\session\<工作>\<agent>`，盘符按实例分配，别假设恒为 Z:；宿主路径由路径码翻译；`SOLOMNI_VIEW_DRIVE=0` 或视图盘不可用时回退宿主路径）；虚拟机档 = guest 内路径（路径桥，宿主 `D:\a\b` → `/solomni-host/d/a/b`，见 `src/units/business/vm/domain/paths.rs`；UNC 外部根在 guest 里是 `/solomni-host/unc/<server>/<share>/…`，模型照此写法给路径即可）。
- 回放只读结构化事实：`src/units/business/agentloop/domain/member_tools.rs` 的 `last_shell` 取最后一条带命令的 shell 工具行的 `shell` 事实（cwd / 耗时毫秒 / 围栏准备 / 退出码 / 状态 / 是否截断），不解析回执正文；没有结构化事实的行跳过。派生快照标 `historical`。
- **授予固定，调用参数里没有范围旋钮**：每次调用的围栏都一样——私有沙箱 rw、模块目录 / 基础与项目工具链 / 主机只读根 ro、net 按会话档位（`offline` 恒 false）；共享区主副本永不进容器（读 / 改先 `work_pull`）。要够到沙箱外的东西，只有「先拉进沙箱」这一条路。唯一的显式例外是 `local_ports`：它不扩大文件系统范围，只把清单里的本机回环端口经中继露给容器（第三态 `NetPolicy::Local`，见 [permission/README.md](../permission/README.md)）。
- profile：只有 offline / online 两档，差别只在出站；`online` 仍受全局 `sandbox.policy.network` 上限。每次都要求围栏真强制（`allow_degraded=false`），装不上就拒绝。
- 围栏：shell 的 `for_call` 另加 `runtime_roots`（项目内基础工具链 + 本 agent 模块运行包根）；`home` 恒为私有沙箱（隐含一条写授予）、写授予另加私有区可用的模块，全部模块目录是只读子树，没有按 reach 的分支。`runtime_roots` 这类**可信执行基座**在 Windows 上一次性授给**本产品的能力 SID**（`S-1-15-3-*`，见 [kernel/README.md](../kernel/README.md) 的「授权主体分两个」）、按机器级归属记台账（`scope=machine`、无归属，启动对账不回收）、不参与按次授权。shell 的容器 cwd 走视图盘（产品根映射成实例独占的盘符；模块工具仍用宿主 cwd），PowerShell 逐级解析只经过产品根内的层级，产品根之外的祖先不授；产品根上的一条 `(OI)(CI)` 读属性是机器级授权——授给同一个能力 SID、跨会话复用（对账时不回收），不再每次会话重写。容器身份按 `会话 + agent` 派生（不含根集合），授权按会话持有（调用结束不释放，会话改配/删除时回收）；一次性调用（构建/独立模块工具）各自唯一租约、跑完即撤。首次真写 ACL 前 stderr 提示一次（解释器目录 / 祖先链 / 受保护根 / 可信基座），完成后一条汇总；回执行文不变（仍只报总秒数）。
- 平台：Linux / macOS 用 sh；Windows 用 PowerShell（pwsh 7 优先、5.1 回退；`pwsh` 是 Microsoft Store 的应用执行别名——0 字节重解析点——时不选它，那种文件在围栏里起不来，判据在 `src/kernel/detail/process/windows.rs` 的 `exe_fact`；`-File` 临时 .ps1，UTF-8 BOM；脚本先设 `$ErrorActionPreference = 'Stop'`，未处理的 cmdlet 错误中止并以非 0 结束，要容错就在脚本里显式 `-ErrorAction SilentlyContinue` 或 try/catch）；VM Linux guest 用 /bin/sh，helper 按 `timeout_secs` 杀树；host 停止经 QGA 给 helper 发 SIGTERM，立即杀整棵进程树。guest 通道产出与本机同形的正文与说明：超时 / 信号由 `src/units/business/vm/detail/guest_helper.rs` 的机器标记（`solomni-guest-fence: timeout` / `solomni-guest-fence: signal`）表达、用户停止按宿主取消旗判定，由通道翻译成 `shell.timeout` / `shell.signal` 说明；cwd·耗时由工具层在通道返回后统一补（`src/units/business/agentloop/engine/tools.rs`）。围栏拒绝（helper 的 `solomni-guest-fence:` 诊断）没有结束说明——命令没有执行。
- 围栏基线：本机 Linux / macOS 的只读运行基线**只对 `/dev/null` 例外给写权**（标准重定向要能丢弃输出，`>/dev/null` / `2>/dev/null` 不能用 EACCES 把脚本带偏）；guest helper 同一口径。其余 `/dev` 节点不放行（`src/kernel/detail/confine/linux.rs` 的 `WRITABLE_BASELINE`、`src/kernel/detail/confine/macos.rs` 的 profile、`src/units/business/vm/detail/guest_helper.rs` 的基线）。
- 状态：最近一次 shell 在会话状态栏出 chip（运行中；结束后按 `ShellLive.status` 分退出码 / 超时 / 停止 / 信号 + 耗时），点开看命令、cwd、状态、退出码、耗时与回执预览；数据来自 `SessionView.shell`，不替代转录里的工具行。重建 / 回放的会话按转录派生（`historical`）：cwd 与耗时只读工具行的结构化事实，起止时间不伪造。
- 角色：读面恒发，所有角色都有 `shell`（`discussant` / `planner` / `orchestrator` / `core_proxy` 只读核实，`solo` / `executor` 干活）。代理的 shell 只做验收性核对，不替执行席产出；共享区主副本不进它的容器（要读先 work_pull），仍没有 work_pull / work_commit / work_push / work_sync / work_diff / work_reset / 模块工具。
- 路径：shell 不接受模型给的 path 参数，**不适用** §七 的路径寻址表；边界是围栏。共享区内容先 work_pull 进沙箱；模块目录与主机只读根恒以只读进围栏，联网按会话档位，共享区主副本不进容器。
- 隐私：shell 不注入模块 secrets；模块私有区不在它的 rw 里（只有模块工具进程能写）。

## 六、核心操作必须走工具调用

- **规则**：任何**会驱动核心**的产出都必须是一次**工具调用**——建任务链、节点验收、总验收、代拟 / 推荐名单、执行席回报（`systools/tools.yaml`）。
- 两条通道等价（原生走结构化槽位、信封走手写信封），核心只从**工具参数**取载荷；正文里手写的同形 JSON **不当作载荷**。

- **为什么**：这些载荷要驱动下一步（建会话、派发、推进或定向返工）；正文 JSON 既没有 schema 校验、也不进工具台账，写坏就整轮失败。
- **普通说话仍可以是正文**：不驱动核心的发言（讨论意见、执行席的收尾话）不必包成工具——这就是"说话"与"操作"的分界。
- **工具与载荷形状**在 `systools/tools.yaml`（`plan` / `node_verdict` / `checklist` / `slate` / `submit_report` …）。
- **核心代理的工具面**（`core_proxy`，逐项见「角色与工具面」表）同样只从工具参数取载荷；外部动作经 `proxy::api` 的 `ProxyHost`（生产宿主是 `src/units/business/runtime/service/bridge.rs` 的队列桥，见「边界」）。子会话**不把整份转录推给核心**：门与意外停止由机制送达，正文经 `read_session_messages` 倒查。
- **载荷不合法 → 如实失败并中止这一步**（不猜、不回落正文 JSON）。
- **核心可以先核实**：模型先请求 `read` / `grep` 时，核心执行并把结果回灌，然后再要那一次核心操作调用；它用的是**只读**小环境（根 = 本次工作的共享区，只发 `read` / `list` / `grep` 三个只读工具）。没有这条回路，模型一想核实就会被判"没有调用 X"而整步中断。

## 七、路径模型：工具用本环境的写法

- 仓库里（代码、提示词册、`module.yaml`、文档）一律只用**占位符**，运行时由核心取真实目录替换进提示词——仓库永不出现机器路径；系统工具与模块工具**用同一套路径语言**（布局见 [session-model.md](../session/session-model.md) 一）：

- **一种写法，一处说明**：
  - 模型对本环境用绝对路径：本机档是宿主路径，虚拟机档是 guest 内路径（路径桥渲染）；**虚拟机档不认宿主写法**，写了按“不是本环境的写法”拒绝。
  - 用户 `@` 引用改写出来的也是这同一种写法（见下「用户也能引用」）。
  - 规矩只在提示词册的 `path_rules` 里说一次（见 [prompts.md](../prompt/prompts.md)）。
  - 虚拟机档的 guest 写法由核心在**寻址前**译回宿主路径（`kernel::ports::PathCodec`），再过下面的允许根校验。
  - **翻译不改变安全边界**：越界、`..`、相对路径照样拒；拒绝时按本环境写法列出允许的根。
  - 两档同一口径：共享区对 agent 只读，发布到共享区走 `work_commit`（记进工作线）+ `work_push`；虚拟机档只是把同一个只读范围翻成 guest 写法再交给 guest helper。

| 占位符 | 运行时替换为 | 可达范围 |
| --- | --- | --- |
| `{{work_root}}` | 本次工作共享区根：默认 `data/session/<工作名>/work/`，根会话绑定外部工作区时是该目录的绝对路径（**主副本**）；虚拟机档由路径桥渲染成 guest 内写法（内部布局落在会话树挂载点下，外部绑定时落在外部工作区挂载点下） | 本工作内 agent 的内置文件工具（**只读**：写会被拒绝，写入走 `work_commit` + `work_push`）；容器里的进程（shell / 模块工具）读不到它，要就地处理先 `work_pull` |
| `{{sandbox_root}}` | 该 agent 私有沙箱 `data/session/<工作名>/<agent实例名>/` 的绝对路径 | 只有它自己（**永远全权**） |
| `{{module_roots}}` | 该 agent 使用的模块目录的绝对路径（只读表头 + 一行一个；无模块时给「成员模块目录：无」） | 只有使用该模块的 agent（**对 agent 的内置文件工具永久只读**：agent 只有使用权；模块工具 / 服务进程可写自己的私有区（`data/modules/<id>/userdata`）） |

- **越界即拒绝**：
  - 路径必须是列出的真实根**之下**的绝对路径（`src/units/business/tools/domain/fence.rs`）。
  - 相对路径、`..` 跳出、不在任何根之内：一律拒绝，并把允许的根列回去（如实报错，不纠正）（`src/units/business/tools/service.rs`）。
  - **这张表只管系统工具**：模块工具的入参不查它；可达范围由围栏（本机档）/ guest 边界（虚拟机档）约束。
- **可达根与直接父目录可判断存在性**：`exists` / `stat` 在围栏里得到真实结果，「产物目录不存在就先建」这类写法可用；Windows 依赖容器父目录只读项（默认开启，关掉会让这条走偏，报错来自命令自己）。写进未授权目录仍会失败，且失败只发生在该路径所在的调用，不连累无关路径。
- **Windows 下路径仍统一写 `/`**：守门进程只把**程序名**里的 `/` 换成 `\` 再交 `cmd` 解释；参数里的路径不动。
- **会话权限叠加在落点之上**：
  - 共享区读写受**该会话**的权限档位与共享区范围约束（范围三态 all / none / 路径数组；黑名单只做减法；全局黑名单是硬底线）。
  - `work_commit` / `work_push` / `work_sync` / `work_reset` 只接受允许范围内的相对路径（越界**整条拒绝**）；`work_pull` / `work_diff` 只碰允许范围内的路径。
  - 私有沙箱不受共享区路径范围限制；模块目录对内置文件工具永久只读。
  - `work_versions` / `work_diff` / `work_resolve` / `work_lost` 走读作用域（它们不写共享主副本，发布仍由 `work_push` 收口；`work_lost` 的 restore 只写自己的沙箱）；`work_sync` 走写作用域（它推进共享区 head，但不改工作区内容）。
  - 语义见 [docs/permission/README.md](../permission/README.md)。
- **编码**：内置 `read` / `write` 读严格 UTF-8（非法字节按替换字符呈现并如实标注——**本程序不猜编码**），写一律 UTF-8（`src/units/business/tools/detail/sys_io.rs`）；
- `work_pull` / `work_commit` / `work_push` / `work_sync` / `work_versions` / `work_resolve` 按**字节**处理，图片、视频与其它二进制不走 UTF-8 假设。
- 内容按块流式读写、不整份进上下文；单文件有上限（`MAX_ASSET_BYTES`，见 [workspace](../workspace/README.md)），超限提交 / 上传整次拒绝。
- **二进制内容由模块工具或用户投喂产出**，内置 `write` 不是二进制通道；产出后用 `work_commit` 记进工作线、`work_push` 发布。
- **用户也能引用**：
  - 输入框里用 `@` 挑文件：`@work:相对路径` / `@sandbox:<agent>/相对路径`（**给人用**）。
  - 核心按**本环境的写法**渲染成绝对路径之后才进转录与上下文：本机档 = 宿主真实路径；虚拟机档 = guest 写法（同一座路径桥，模块工具在 guest 里直接能用）。
  - 所以 `@` 引用与系统工具寻址用同一种写法；系统工具先译回宿主再校验。
  - 引用别人的私有沙箱时如实说明无权读取，且不泄漏对方的真实路径。

## 八、提示词按角色分配

```text
prompts/
  shared/  协议、信封约定、工具协议、文案兜底、refs
  roles/   discussant / executor / planner / orchestrator / core_proxy（solo 复用 executor 的文件）
```

- 每份文件里**具体有哪些键、每个键干什么**：[prompts.md](../prompt/prompts.md)。

- **角色位 = 一个角色在这一段的机制动作**（`loops.yaml` 的 `cast`，类型是 `src/units/business/session_model/domain/mechanism.rs` 的 `Cast`；角色本身是 `Role` 的提示词 + core）：信封模式下这一回合能用的信封清单**要渲染进该回合的上下文**（原生通道则由声明槽给出）。文件面 / 模块面 / 系统工具不随角色位声明，它们各自有来源，分开声明不会漂——来源只答"在不在场"。
- **身份必须可派生**：提示词与工具面决定了上下文与转录，重启 / 回档后必须重建出**同样的身份与上下文**（回放与实时产出同样的消息），所以身份**不能**是运行时内存里的临时状态；每回合由 `TurnRun.loop_id` / `TurnRun.role` 现给（`src/units/business/prompt/domain/prompt.rs`）。
- **planner 与 orchestrator 分开**（上下文不同、工具面不同、指令互斥），只共享**背景知识**（链的语义、节点状态定义、信封协议）——共享的是背景，不是指令。
- **executor 是全新上下文**：讨论内容只经任务提示词带入，不继承讨论转录。
- **solo 与 executor 只差回报工具**：`submit_report` 的消费者只有协作的节点；用户建的单 agent 会话没有消费者——发了它只会多一次工具往返、把最终答复塞进工具参数。

### 角色与工具面（当前）

| 角色 | 何时 | 这一角色位交的动作（`Cast.tools`，即 `loop` 来源） |
| --- | --- | --- |
| `discussant` | 讨论阶段 | `say` `agree` `leave` `ask` |
| `executor` | 任务链节点（agent 子会话） | `submit_report` |
| `solo` | 用户建的单 agent 工作 | 无（与 `executor` 的差别就在这里） |
| `planner` | 核心整理派发 | `plan` `slate` `verdict` |
| `orchestrator` | 核心链中推进 | `node_verdict` `checklist` |
| `core_proxy` | 核心代理（用户把决定权整块交给它，代用户决定） | 无（六个会话控制动作是会话类型的系统工具） |

- 其余三面照 §二 的来源算：读面 / 写面 / `shell` 由 `agent` 来源按能力组给，11 个 `work_*` 由 `workspace` 来源给，模块工具由 `module` 来源给。
- **验收不是独立角色**：它是 `orchestrator` 的一个工具 / 一步（"根据验收情况推进任务"本就是同一个循环）。

## 九、协作动词工具化

- `say` / `agree` / `leave` / `ask` 是**声明式工具**（声明在 `systools/tools.yaml`，由 `discussant` 角色的 `tools` 发放）：原生模式走供应商的结构化槽位，信封模式走**同一份声明**渲染出的信封——两套产出同样语义，参数经同一处 schema 校验：
- 写坏的信封**不降级成普通发言**，而是如实记一条失败的工具行；没有信封的纯正文 = 这一轮**没表态**。

## 十、边界

- 边界处置按表约定：代理工具、建会话、回档与系统工具均按此收口。

| 场景 | 约定 |
| --- | --- |
| 代理工具 | 当前：六个工具 + `shell` + `core_proxy` 角色 + 队列桥宿主（真实会话）。代理档位由用户建会话时显式选择并落盘（预设全可达，不再硬编码 full），代理有真实私有沙箱 `.proxy`；权限语义见 [docs/permission/README.md](../permission/README.md)。代理的 `shell` 只做验收性核对（跑测试 / 看产物），不替执行席产出；共享区只读，仍没有 `work_pull` / `work_commit` / `work_push` / `work_diff` / `work_reset` / 模块工具；`net` 跟随全局 `sandbox.policy.network`。授不上必要落点的调用经统一裁决通道让用户选 `fence_unfenced_once` / `fence_abort`；没有可回答前端一律拒绝。孩子不把转录推给代理：门的通知、意外停止通知 + `read_session_messages` 主动倒查 |
| 建会话 = 建 + 写开头 + 开工 | `create_session` 一份声明两处调用：**人经呈现层**给 `name` / `kind`（会话类型表里的一个 id）/ `agents`（可省 `sandbox` = 执行环境策略 id；虚拟机策略时给 `guest`（虚拟根名））先建出会话，后续用 `send_message` 发言推进；**核心代理**给 `kind`（只能是 `catalog_agents` 的 `scope=types` 列出的那些，即标了 `agent_visible` 的类型）+ `agents`（只写身份：`ref` 或 `name`+`modules`+`model`+`chara`；`chara` 可选，复用项取登记处自己的）+ `task`，**建好就开工**（有流程的类型以 task 当本次需求，没有流程的以它当第一句；有流程的类型按流程的角色要求凑够人数，不够就拒）。后续补充 / 返工 / 代答门仍走 `send_session_message`（**代答要显式给 `reply` = 那张卡上的选项 id**；给不出就不作代答，不替用户猜） |
| 历史态的启用 / 继续 / 恢复归档 | 历史态（只读历史与 `stopped`）只给这一状态下有用的那一个动作：`archived` 给「恢复归档」（翻回 `active`，不唤醒）、`stopped` 与其余给「启用会话」（`activate_session` 从盘上重建、确保 guest、解冻整棵子树，**不跑模型**）。停着不给「继续」——派发会被闸拒。启用后 `SessionView.continue` 说有断点时才显示上下文动作（单 agent「继续」/ 协作「恢复协作」），走 `control_session` 的 `continue`；它**不替用户发言**（末条已是用户发言就直接问模型）；代理不显示。 |
| 代理工具的生命周期 | `interrupt` = 取消整棵子树在跑的生成、作废等裁决的队，**不动运行态**（会话照常可派发，界面上的「中断」就是这一下）；`stop` = 先整棵子树落成 `stopped`（拦住派发与唤醒）再中断，顺序不能反；`continue` = 解冻（`stopped` → `active`）并按形态重开一轮；`archive` = 整棵子树标成 `archived`、从列表收起，**不是终态**；`restore` = 翻回 `active`，不唤醒、不产生模型调用。中断 / 继续、停止 / 继续、归档 / 恢复各是一对逆操作，没有单独的暂停；都不改写历史 |
| 代理模式下的子会话审查关卡 | 不问用户：作为**门**交给核心判断，核心用 `send_session_message(kind=user_reply, reply=<选项 id>)` 回答 |
| 代理模式下子会话的转录 | **不转发给核心**：核心只拿门的通知与意外停止通知，正文经 `read_session_messages`（0 = 最新）主动倒查 |
| 改任务目标 | 等于**改任务提示词**；改完**不作废**，让它跑完再由核心验收；链有问题则与用户在**主会话**讨论改链 |
| 回档 | **整棵子树按同一回合 id 同步收窗**；留档折叠（可再恢复），删除 / 恢复真的截断（见 [session-model.md](../session/session-model.md) 的「回档：三种模式 + 工作级标记」） |
| 共享区提交历史 / 回滚 | 只读视图 + user-only 动作 `restore_work_commit`：非破坏性、`expect_head` 防陈旧、运行中拒绝；目标之后的提交仍在历史里（细则见 [workspace](../workspace/README.md)） |
| 系统工具调用 | 与模块工具同路径：工具行 + `[工具结果]`，可审计、可回放 |

## 十一、当前状态

- **共享区是版本化工作区**：

- 主副本 `work/` 对 agent 只读，agent 只写自己的沙箱（工作副本）。
- `work_pull` / `work_commit` / `work_push` / `work_sync` / `work_status` / `work_ignore` / `work_versions` / `work_diff` / `work_resolve` / `work_reset` / `work_lost` 十一个**核心自有工具**做同步、查看、过滤名单维护、差异比较、本地回退、三方比较、显式解决冲突、外部改动采纳与备份恢复。
- 冲突按文件级三方比较判定，**整体拒绝并逐条点名**，不做自动合并；文本可人工合并后用 `take=local`（带 `head`），图片 / 视频 / 二进制只能显式选边；删除型冲突（一边删、另一边改）只能选边或 `work_reset` 撤回那条提交，`work_pull` 不写这类文件（细则见 [workspace](../workspace/README.md) 与 [PRODUCT.md](../../PRODUCT.md)）。
- `take=local` 在本地与基线一致时拒绝、主副本提交变化时拒绝；它只推进基线、不写沙箱——本地缺失时主线没改过该路径可 `work_commit` 的 `deletes` 发布删除，主线改过就只能 `take=upstream` 或 `work_reset`；`take=upstream` 覆盖/删除前备份到 `lost/<agent>/@<来源>/<原路径>`（`work_resolve` / `work_reset` / 回档 / 删除 / 留档各有来源），`work_lost` 按当前 hash 恢复：清单报原路径与备份位置，`path` 两种写法都收，同一路径多份备份必须点名一份，恢复一律写回原路径。
- 文本、图片、视频与其它二进制都按字节处理；`work_versions` 给 hash、大小与扩展名媒体类型提示，文本预览最多 16 KiB；单文件超过 64 MiB 时提交 / 上传明确拒绝。
- 用户投喂是同一条提交路径里的**权威提交**（作者 = user）。
- 用户可在 Web 会话状态栏或 CLI `work` 命令看提交历史、提交详情与文本预览，并把共享区非破坏性回滚到任一提交点；agent 侧仍只拿 `work_*` 工具，不给回滚入口。

- 执行类系统工具 `shell` 属读面、对所有角色发放：授予固定（私有沙箱 rw、模块目录与基础 / 项目工具链、主机只读根 ro，网络按会话档位）；Windows 上按"会话 + agent + 根集合"派生容器身份（与模块工具同一口径）。
- 工具面**按回合按段与角色注入**（总表不进提示词）；越权调用如实拒绝并落工具行；悬空引用由装配期 `loop_check::problems()` 挡下。
- 核心操作与执行席回报都从**工具参数**取载荷。
- 代理会话复用**通用成员循环**（没有「代理专用」分支）；工具动作经队列桥回核心线程执行。
- 呈现面：Web 建工作向导含代理形态，会话状态栏显示上下文用量与共享区提交；上下文卡片的累计行读 meta 的 `usage_totals`；「过期内容换成占位符」与保底在创建向导与会话配置里设（动作 `set_context_freshness`）；CLI 有 `resident` / `secret` / `work` 入口。链的流程见 [task-chain.md](../collab/task-chain.md)。

## 十二、联动

- 模块工具的声明与执行见 [MODULE_SPEC.md](../../MODULE_SPEC.md)；
- 会话、围栏与并发模型见 [architecture/README.md](../architecture/README.md)；
- 呈现层入站契约见 [contracts.md](../presentation/contracts.md)；
- 产品行为（讨论 / 审查关卡 / 任务链）见 [PRODUCT.md](../../PRODUCT.md)；
- 测试层级与缺口账见 [TESTING.md](../../TESTING.md)。

## 十三、工具进程边界

- 模块工具与 `shell` 都从**守门进程**里跑：外层 `env_clear`，只注入 `fence_env` 与该模块的隐秘字段；到点连根杀树（模块工具 30 秒，`shell` 用本次 `timeout_secs`）；可达范围按平台围栏强制；能力等级在启动报告里如实给出（机制见 [docs/kernel/README.md](../kernel/README.md) 的「工具进程与守门协议」）。
- 模块作者仍应把工具写成**只碰自己工作区**的样子；围栏是兜底，不是许可。

## 十四、产品命令面

- **命令表是命令面的唯一声明**：`systools/commands.yaml` 写每条产品命令的名字、别名、分组、`phase`（`boot` 装配核心前跑完退出 / `core` 转录中心里输入）、执行者（`core` / `system` / `ui`）、一句话说明、确认门槛与参数声明（与动作面同一种形状：类型、位置序号、可选值、缺省值）；表经 `src/units/business/tools/domain/op.rs` 投影成 `Op` 书，手册、菜单、Tab 补全、未知命令建议与文本解析都是它的派生。
- **与工具表同族**：两张表都是"声明在文本层的契约"，读表在 `src/units/business/tools/domain/commands.rs`（`CommandBook`），文本→JSON 的解析与校验只在 `src/units/business/tools/domain/op.rs` 一处（`parse` / `usage` / `complete`）；命令面**不拥有行为**——执行者三分：`system` 交 `src/presentation/cli/boot.rs`（启动相，装配核心之前）、`core` 与 `ui` 交 `src/presentation/cli/shell.rs`（装配之后，命令行一次性调用与转录中心提示符共用同一条）。
- **编译进二进制**：表用 `include_str!` 编进来，`solomni --help` 在任何目录下都可用，不依赖产品根上的 yaml 是否就位；这与运行时读盘的 `tools.yaml` 有意不同（命令是产品语法，不是用户策略）。
- **声明自洽在装配期硬失败**：相位与执行者必须配对（启动相只能 `system`，转录相不得 `system`）、位置序号唯一、`rest` 不再写序号、可选值只给字符串——任一条不成立，`src/main.rs` 的 `command_ops()` 逐条报错并退出 1，不靠测试阶段的字面量比对兜底。
- **调用面不止命令**：模块工具与命令共用同一套词法与回执——`solomni <模块 id>` 是模块手册（版本、状态、`system` 原文、工具表），`solomni <模块 id> <工具名> --help` 是工具手册，`solomni modules` 是清单（含版本、启停与拒收原因）；`solomni <模块 id> --version` 是模块版本，工具没有自己的版本。模块工具的调用与 agent 侧同一个 `ActionCall`（id = `module.<模块 id>.<工具名>`）。
- 内部协议旗语（守门子进程、探针、路由目录）**不进**命令面，只在 `src/presentation/cli/argv.rs` 的透明透传段里。

## 检查
- 系统工具总表与角色面的一致性 → `src/tests/detail/`（`SystemTools::problems()` 管总表自身，`loop_check::problems()` 管角色位与会话类型引用的 id 都在总表里）。
- 命令表与两处分派的一致性 → `src/tests/commands.rs`（另含表自检与手册行文）。
- 工具面与越权 → `src/tests/tools/` 与 `src/tests/api/`。

## 路由
- 系统工具门户：[../../SYSTOOL.md](../../SYSTOOL.md)
- 模块契约：[../../MODULE_SPEC.md](../../MODULE_SPEC.md)
- 会话模型：[../session/session-model.md](../session/session-model.md)
- 架构约束：[architecture/README.md](../architecture/README.md)
- 呈现层契约：[../presentation/contracts.md](../presentation/contracts.md)
- 产品行为：[../../PRODUCT.md](../../PRODUCT.md)
- 测试规范：[../../TESTING.md](../../TESTING.md)