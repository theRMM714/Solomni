# 工具与角色

> 本文是**系统工具、角色（身份）、以及"谁能用哪些工具"的唯一权威**。
> 模块工具在各自 `module.yaml` 声明（见 [MODULE_SPEC.md](../../MODULE_SPEC.md)），本文只管系统工具。
> 门户：[SYSTOOL.md](../../SYSTOOL.md)（入口与真相源表）。

## 一、两张表与字段

工具**是什么**在 `systools/tools.yaml`；这个**身份有什么**在 `systools/roles.yaml`。
代码里**不得**出现"哪个角色能调哪个工具"的判断：只有两个动作——**按角色组装工具面**、**按表校验调用**。

```yaml
tools:
  read:
    capability: fs-read   # 能力：决定收口（沙箱 / 围栏）
    callers: [user, discussant, executor, solo, core_proxy]   # 谁能调它：user（经呈现层）/ 角色 id
    desc: 读取文本文件（UTF-8）。
    parallel: true        # 同一回复里的多个调用能否真的并发跑
    params:               # 既用于生成声明，也用于校验调用参数
      path:
        type: string
        required: true
        desc: 要读取的真实绝对路径
```

- **`capability` 决定收口，不按"系统 / 模块"一刀切**：`read` / `list` / `search` / `write` / `edit` / `patch` 虽都是系统工具，
  但碰文件系统，**照样受沙箱与围栏约束**；`say` / `agree` 这类不碰文件。
- **`callers` 是授权判据**：`user` = 人经呈现层调用，其余是角色 id。它与角色表的（这个身份有什么）是**同一关系的两面**，
  由 `SystemTools::problems` 双向锁死——任何一边漏写 / 多写都是装配错误。
- **列目录是"核实"的前提**：`read` 只读文件、`search` 要关键词，**没有 `list` 就确认不了"资料齐不齐"**；`read` 遇到目录会如实引导到 `list`。
- 参数不合法 → **如实拒绝并落工具行**。

```yaml
roles:
  discussant:
    prompt: roles/discussant   # 提示词与工具面**同处声明**
    tools: [say, agree, leave, ask, read, list, search]
```

- **角色 = 场景绑定的身份**：`discussant` 只存在于讨论阶段，`orchestrator` 只存在于任务链推进阶段；**不引入"阶段 → 工具"的第二张表**。
- **角色表只引用系统工具**：模块工具是"这个 agent 的能力"，不是"这个角色的属性"。
- **不做角色继承**：重复列几个 id 没有代价，继承会引入隐式耦合。

## 二、可用工具面（运行时合成）

```text
本会话可用工具 = 系统工具表 ∩ 该角色表 + 该成员所属模块的工具
```

越权防线分两段，缺一不可：系统工具查**角色表**；模块工具查**该成员有没有这个模块**（module 消歧）。

**注入口径（唯一）**：**总表不进提示词**。每回合由核心查"这一回合的身份能用的工具"，
把那一份的说明与参数作为一条系统消息随回合注入；原生通道同时用它做工具声明槽。

| 回合 | 这一份是什么 |
| --- | --- |
| 讨论回合 | `discussant`（发言动词 + 只读核实），**不含模块工具**（它不干活） |
| 执行回合·单 agent 工作 | `solo` + 该 agent 的模块工具（含共享区版本化三件套） |
| 执行回合·协作节点 | `executor` + 该 agent 的模块工具（比 `solo` 多一个回报工具） |
| 核心代理回合 | `core_proxy`（代理工具 + 只读核实），**不含模块工具**（它代用户决定，不替 agent 干活） |

发放与校验是**同一份判据**：列出来的就是这一刻真能调的；没拿到的既不在提示词里、也不在声明槽里，真去调会被如实拒绝。
发放只有一处：`SystemTools::role_face`（id 清单 + 是否给模块工具），未知名在装配期就如实报错。
驱动把它装进**这一回合**的工具环境（`session::TurnRun.face`），跑完还原——同一个会话因此能用两种身份干活（讨论席说话 / 执行席干活）。

**两条硬口径**（`systools/tools.yaml` 是唯一真相）：

- **AI 对系统的每个操作都要有对应工具 + 身份限制**：没有工具的操作 = 契约缺口（例如"要返工哪个节点"
  必须是 `checklist.items[].rework` 这类**字段**，不能塞进自由文本）。
- **工具的"后果"写进声明**：会驱动核心动作的工具要写清"这个结论会让系统做什么"——`node_verdict` 的 `ok=false` =
  该节点退回待办、点「继续」只重派它；`checklist` 的 `fail` 必须填 `rework` = 要返工的节点 id；`verdict.clear=true` 才照用户说的开工 / 放行。
  写了表里没有的 id 或漏填 = 这次判定用不了，核心**要求重填**。

身份与环境同理：它们是**会话参数**（`session::SessionParams`），每次调用现渲染成系统消息，不占对话的位置
（见 [session-model.md](../session/session-model.md) 的「会话参数与对话分开」）。

核心自己的操作（`planner` / `orchestrator`）不走 agent 会话：原生通道由声明槽给出它们的面，
信封通道由各自的角色提示词写清载荷形状——总表同样不进提示词。

## 三、越权校验

角色表不只是渲染进提示词的清单——**工具面与越权校验都出自它**：讨论回合按 `discussant`；执行回合**按身份**
（用户建的单 agent 工作用 `solo`，协作的节点子会话用 `executor`，两者只差回报工具）；核心代理回合按 `core_proxy`（`module_tools: false`）。
代码里**没有任何**"哪个角色能调哪个工具"的名单，加一个工具只改两张表。

- **原生模式与信封模式走同一个校验点**；两者**互斥但等价**——等价性一破，回放就与实时不一致。
- 越权调用 → **如实拒绝 + 落工具行**（用户要能看到"它越权了"）。
- **悬空引用**（角色引用总表里不存在的 id）→ **结构审查硬失败**。这是两张表不漂的机制保证。

## 三之二、动作与分发（唯一一处）

**动作表是授权与执行的唯一真相**：每条动作声明参数契约与 `callers`；分发只有一处（`conductor` 的 `ActionOps`）——
**参数按声明校验 → 按 `callers` 授权 → 执行 → 审计**。

- **两个适配器**：模型侧（`ToolHandler`，按名字认领）与呈现侧（`POST /api/actions/{id}`、`GET /api/actions` 动作目录）
  都只负责造一次 `ActionCall`（动作 id + 参数 + 调用者身份）并各按自己的媒介呈现结果——
  模型侧拿文本回执，呈现侧拿结构化结果。**同一份声明，两种呈现。**
- **读取与视图不进动作表**：会话列表 / 配置 / 文件清单 / 历史是读接口，事实仍只有**事件台**一条来路。
- **人经呈现层调用**（`caller = user`）与**模型经工具调用**（`caller = 角色 id`）走同一次校验、同一处授权、同一条审计；
  适配器不各自校验、也不各自判权。
- **登记处动作只给 user**：供应商 / 密钥 / 模型 / agent / 设置的写面（`upsert_provider` / `remove_provider` / `discover_models` / `upsert_model` / `remove_model` /
  `set_core_model` / `probe_model_tools` / `probe_replay_shape` / `upsert_agent` / `remove_agent` / `set_settings`）的 `callers` 只有 `user`——
  产品级资源默认不开放给任何角色；`api_key` 只进登记处，不进动作目录、不进审计。读（`GET /api/settings`、`/api/state`）仍是独立读接口。
- **常驻服务的生命周期动作也只给 user**：`control_resident`（`start` / `stop` / `enable` / `disable`，参数 `module` / `name` / `action` / `lease`）——
  agent 用已启动的实例，不负责拉起；服务清单是独立只读视图（`ResidentOps::services`，CLI `resident`）。会话删除时按租约回收该会话拉起的实例。
- **已启动常驻服务的操作也走模块动作**：id = `module.<模块id>.<服务名>.<操作名>`（静态模块工具优先，其次常驻服务操作）——
  人可直接调（`ResidentOps::call`）；agent 侧：已启动服务的操作以 `<服务名>.<操作名>` 进成员工具面（原生声明与信封清单都列，线上名用下划线），
  调用经 `ResidentOps::call` 转给常驻服务；**未启动的服务不出现**（操作是跑起来才发现的）。
- **隐秘字段的设置只给 user**：`set_secret` / `clear_secret`（`module` + `name`，值 `value`）——
  值只落 `.home/secrets.yaml`，不进转录 / 日志 / 命令行，永不回显；审计只记动作 id 与成败，不记值。
- **模块工具也是动作**（动态动作，来自清单）：id = `module.<模块id>.<工具名>`。人可直接跑（无会话，围栏按模块目录 + 可选工作目录派生）；
  会话里由成员循环执行（同一份 `module.yaml` 声明、同一个 `ToolExec::run_module`）。两条路径的审计记录同一处格式化（`action_audit`），
  只是各自进自己的账本：呈现层进运行日志，模型侧进**转录工具行**（可回放）。**缺运行包 = 不执行**：两条路径读同一把尺子（`runtime_report.missing`），
  目录里如实标不可用，不静默降级。

## 四、核心操作必须走工具调用

**规则**：任何**会驱动核心**的产出都必须是一次**工具调用**——建任务链、节点验收、总验收、代拟 / 推荐名单、执行席回报。
两条通道等价（原生走结构化槽位、信封走手写信封），核心只从**工具参数**取载荷；正文里手写的同形 JSON **不当作载荷**。

- **为什么**：这些载荷要驱动下一步（建会话、派发、推进或定向返工）；正文 JSON 既没有 schema 校验、也不进工具台账，写坏就整轮失败。
- **普通说话仍可以是正文**：不驱动核心的发言（讨论意见、执行席的收尾话）不必包成工具——这就是"说话"与"操作"的分界。
- **工具与载荷形状**在 `systools/tools.yaml`（`plan` / `node_verdict` / `checklist` / `slate` / `submit_report` …）。
- **核心代理的工具面**（`core_proxy`，逐项见「角色与工具面」表）同样只从工具参数取载荷；外部动作经 `ports::ProxyHost`
  （生产宿主是 `service/proxy.rs` 的队列桥，见「边界」）。子会话**不把整份转录推给核心**：门与意外停止由机制送达，正文经 `read_session_messages` 倒查。
- **载荷不合法 → 如实失败并中止这一步**（不猜、不回落正文 JSON）。
- **核心可以先核实**：模型先请求 `read` / `search` 时，核心执行并把结果回灌，然后再要那一次核心操作调用；
  它用的是**只读**小环境（根 = 本次工作的共享区，只发声明里 `capability: fs-read` 的工具）。没有这条回路，模型一想核实就会被判"没有调用 X"而整步中断。

## 五、路径模型：工具只看到真实绝对路径

仓库里（代码、提示词册、`module.yaml`、文档）一律只用**占位符**，运行时由核心取真实目录替换进提示词——
仓库永不出现机器路径；系统工具与模块自带的外部工具**用同一套路径语言**（布局见 [session-model.md](../session/session-model.md) 一）：

| 占位符 | 运行时替换为 | 可达范围 |
| --- | --- | --- |
| `{{work_root}}` | 本次工作共享区 `session/<工作名>/work/` 的绝对路径（**主副本**） | 本工作内的 agent（**只读**：agent 会话里写它会被工具层拒绝，写入走 `work_commit`） |
| `{{sandbox_root}}` | 该 agent 私有沙箱 `session/<工作名>/<agent实例名>/` 的绝对路径 | 只有它自己（**永远全权**） |
| `{{module_roots}}` | 该 agent 各模块目录的绝对路径（一行一个） | 只有该模块所属的 agent（**默认只读**；`module_write` 授权才可写；`<module>/userdata/` 恒可写） |

- **越界即拒绝**：路径必须是列出的真实根**之下**的绝对路径；相对路径、`..` 跳出、不在任何根之内一律拒绝，并把允许的根列回去（如实报错，不纠正）。
- **会话权限叠加在落点之上**：共享区读写受该 agent 的白名单 / 黑名单约束（默认整棵放行；白名单一出现就取代默认，黑名单只做减法）；
  `work_commit` 只接受白名单内的相对路径（越界**整条拒绝**），`work_pull` 只拉白名单内的路径。语义见 [docs/permission/README.md](../permission/README.md)。
- **编码**：读严格 UTF-8（非法字节按替换字符呈现并如实标注——**本程序不猜编码**）；写一律 UTF-8。
- **用户也能引用**：输入框里用 `@` 挑文件（`@work:相对路径` / `@sandbox:<agent>/相对路径`，**给人用**），
  核心替换成真实绝对路径之后才进转录与上下文；引用别人的私有沙箱时如实说明无权读取，且不泄漏对方的真实路径。

## 六、提示词按角色分配

```text
prompts/
  shared/  协议、信封约定、工具协议、文案兜底、refs
  roles/   discussant / executor / solo / planner / orchestrator / core_proxy
```

每份文件里**具体有哪些键、每个键干什么**：[prompts.md](../prompt/prompts.md)。

- **角色声明 = 提示词 + 工具面 + 渲染规则，三者同处声明**：信封模式下该角色能用的信封清单**要渲染进该回合的上下文**（原生通道则由声明槽给出），分开声明必然漂。
- **角色必须可派生**：提示词与工具面决定了上下文与转录，重启 / 回档后必须重建出**同样的角色与上下文**（回放与实时产出同样的消息），所以角色**不能**是运行时内存里的临时状态。
- **planner 与 orchestrator 分开**（上下文不同、工具面不同、指令互斥），只共享**背景知识**（链的语义、节点状态定义、信封协议）——共享的是背景，不是指令。
- **executor 是全新上下文**：讨论内容只经任务提示词带入，不继承讨论转录。
- **solo 与 executor 只差回报工具**：`submit_report` 的消费者只有协作的节点；用户建的单 agent 会话没有消费者——发了它只会多一次工具往返、把最终答复塞进工具参数。

### 角色与工具面（当前）

| 角色 | 何时 | 工具面（`systools/roles.yaml`） |
| --- | --- | --- |
| `discussant` | 讨论阶段 | `say` `agree` `leave` `ask` `read` `list` `search` |
| `executor` | 任务链节点（agent 子会话） | `read` `list` `write` `edit` `patch` `search` `work_pull` `work_commit` `work_status` `submit_report` + 该 agent 的模块工具 |
| `solo` | 用户建的单 agent 工作 | `read` `list` `write` `edit` `patch` `search` `work_pull` `work_commit` `work_status` + 该 agent 的模块工具（**不发** `submit_report`） |
| `planner` | 核心整理派发 | `read` `search` `plan` `slate` `verdict` |
| `orchestrator` | 核心链中推进 | `read` `search` `node_verdict` `checklist` |
| `core_proxy` | 核心代理（用户把决定权整块交给它，代用户决定） | `catalog_agents` `create_session` `send_session_message` `observe_session` `read_session_messages` `control_session` `read` `list` `search`（**不发**模块工具） |

**验收不是独立角色**：它是 `orchestrator` 的一个工具 / 一步（"根据验收情况推进任务"本就是同一个循环）。

## 七、协作动词工具化

`say` / `agree` / `leave` / `ask` 是**声明式工具**（声明在 `systools/tools.yaml`，由角色面发放）：原生模式走供应商的结构化槽位，
信封模式走**同一份声明**渲染出的信封——两套产出同样语义，参数经同一处 schema 校验：
写坏的信封**不降级成普通发言**，而是如实记一条失败的工具行；没有信封的纯正文 = 这一轮**没表态**。

## 八、边界

| 场景 | 约定 |
| --- | --- |
| 代理工具 | 六个工具 + `core_proxy` 角色 + 队列桥宿主都已接上真实会话；委托是**全权**（`granularity=full`）；路径的白/黑名单与模块写授权已落在会话权限（[docs/permission/README.md](../permission/README.md)），工具级"每次调用是否放行"的引擎路径尚未接入。孩子不把转录推给代理：门的通知、意外停止通知 + `read_session_messages` 主动倒查 |
| 建会话 = 建 + 写开头 + 开工 | `create_session` 一份声明两处调用：**人经呈现层**给 `name` / `mode` / `agents`（可省 `tier`）先建出会话，后续用 `send_message` 发言推进；**核心代理**给 `mode`（single / collab）+ `agents`（只写身份：`ref` 或 `name`+`modules`+`model`）+ `task`，**建好就开工**（single 以 task 为第一句，collab 以它当本次需求）。后续补充 / 返工 / 代答门仍走 `send_session_message`（**代答要显式给 `reply` = 那张卡上的选项 id**；给不出就不作代答，不替用户猜） |
| 代理工具的生命周期 | `stop` = 把整棵子树落成 `stopped`（拦住派发与唤醒）再**级联中断**在跑的生成，会话上的停止按钮就是这一下；`continue` = 解冻并按形态唤醒接着走；`close` 是终态。停止 / 继续是一对逆操作，没有单独的暂停；都不改写历史 |
| 代理模式下的子会话审查关卡 | 不再问用户：作为**门**交给核心判断，核心用 `send_session_message(kind=user_reply, reply=<选项 id>)` 回答 |
| 代理模式下子会话的转录 | **不转发给核心**：核心只拿门的通知与意外停止通知，正文经 `read_session_messages`（0 = 最新）主动倒查 |
| 改任务目标 | 等于**改任务提示词**；改完**不作废**，让它跑完再由核心验收；链有问题则与用户在**主会话**讨论改链 |
| 回档 | **整棵子树按同一回合 id 同步收窗**；留档折叠（可再恢复），删除 / 恢复真的截断（见 [session-model.md](../session/session-model.md) 的五） |
| 系统工具调用 | 与模块工具同路径：工具行 + `[工具结果]`，可审计、可回放 |

## 九、当前状态

**共享区是版本化工作区**：主副本 `work/` 对 agent 只读，agent 只写自己的沙箱（工作副本），
`work_pull` / `work_commit` / `work_status` 三个**核心自有工具**做同步与查看；冲突按文件级三方比较判定，
**整体拒绝并逐条点名**，不做自动合并（细则见 [workspace](../workspace/README.md) 与 [PRODUCT.md](../../PRODUCT.md)）；
用户投喂是同一条提交路径里的**权威提交**（作者 = user）。

工具面**按回合按身份注入**（总表不进提示词），越权调用如实拒绝并落工具行，悬空引用由结构审查硬失败挡下；
核心操作与执行席回报都从**工具参数**取载荷。代理会话复用**通用成员循环**（没有「代理专用」分支），工具动作经队列桥回核心线程执行。
剩下的呈现面（前端向导与代理视图、CLI 入口）尚未接入；链的流程见 [task-chain.md](../collab/task-chain.md)。

## 十、联动

- 模块工具的声明与执行见 [MODULE_SPEC.md](../../MODULE_SPEC.md)；
- 会话、围栏与并发模型见 [ARCHITECTURE.md](../../ARCHITECTURE.md)；
- 呈现层入站契约见 [contracts.md](../presentation/contracts.md)；
- 产品行为（讨论 / 审查关卡 / 任务链）见 [PRODUCT.md](../../PRODUCT.md)；
- 测试层级与缺口账见 [TESTING.md](../../TESTING.md)。
