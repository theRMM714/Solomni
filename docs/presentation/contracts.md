# 呈现层入站契约（命令/事件 + 路由目录）

> 本文是**呈现层入站契约的唯一权威**：能力接口、事件台、命令/事件规则，以及机器可读的 HTTP 路由目录。
> 分层与端口见 [architecture/README.md](../architecture/README.md)，逐文件职责见 [cli/unit-map.md](cli/unit-map.md) 与 [web/unit-map.md](web/unit-map.md)。
> **下面的路由表由契约测试机器比对**（`src/tests/routes.rs` 直接读本文件）：表与
> `web/routes.rs` 的 `ROUTES` 对不上就是测试失败，不靠人记得改文档。

## 一、能力接口与事件台

**核心常驻自己的执行线程、独占全部状态**：呈现层拿不到 `Core`、也拿不到任何核心锁。两侧只通过两样东西来往：

| 东西 | 定义在 | 形态 |
| --- | --- | --- |
| 能力接口 | **各能力自己的 `api.rs`** | `Faces` 的字段：`WorkOps` 在 `src/capabilities/work/api.rs`（工作用例与核心推荐）、`TurnOps` 在 `src/capabilities/turn/turn/api.rs`（回合推进）、`RuntimeOps` / `ActionOps` / `LogOps` 在 `src/capabilities/runtime/api.rs`（启用 / 改需求 / 回档 / 停止、动作目录与分发、埋点门面），`RegistryOps` 在 `src/capabilities/catalog/registry/api.rs`，`HistoryOps` 在 `src/capabilities/conversation/session/api.rs`，`ModulesOps` 在 `src/capabilities/catalog/modules/api.rs`，`ResidentOps` / `SecretOps` / `Provision` 各在 `src/capabilities/<能力>/api.rs`。**全 `&self`、可替换成假实现**；实现都是 `src/capabilities/runtime/service/contracts.rs` 里 `Handle` 的队列代理（`impl` 在核心线程上执行）|
| 事件台 | `src/capabilities/turn/turn/api.rs` | `EventBus`：核心独占生产，任意数量的消费者按序号增量取 |

规则：

- **命令**：呈现层调能力接口 → 核心在自己的线程上执行 → 同步回包（`Advance`：**事件台头部序号**）。
  命令**不携带事实**——事实只有一条来路（事件台）；回包里的 `head` 只是"我现在说到哪了"。
- **按屏幕取面**：`Faces`（`src/presentation/faces.rs`）是根装配出的面袋，每个字段是一个入站契约或一个根填的值（`provision` / `storage` / `toolchains`）；呈现层按屏幕取自己需要的面，不再有一个字段全塞的桶（现 14 个字段），也没有「会话中心」这种横跨三个单元的接口。**每屏到底许取哪几个能力单元**见 [inbound.yaml](inbound.yaml)（T0 双向比对）。
- **动作只有一条路**：`GET /api/actions` 是**动作目录**（这个调用者此刻能做什么，含可用性），`POST /api/actions/{id}` 是**唯一分发**；
  参数按动作表（`systools/tools.yaml`）校验 → 按 `callers` 授权 → 执行 → 审计，各只有一处。
  模型侧经 `ToolHandler` 走**同一份声明与同一处授权**——两个适配器，两种呈现，事实同源。
- **事件**：生成过程中的短暂事件与最终事件都进事件台；Web 长轮询按 `since` 取，客户端按 `seq` 增量取。
- **回档是事件台的一等事实**：留档 / 删除 / 恢复成功后，核心为受影响的
  每个会话各推一条瞬时 `rewound`（携带回档后的**活动视图**），并在事件台记录该会话的**重置水印**；
  `GET /api/history/{name}` 的 `live` 只取水印之后的批次。前端收到 `rewound` 时清空并按 `events` 重建——
  刷新/多端因此不会把回档前的旧事实当成实时尾巴再拼回来。
- **事件按会话分开**：一条会话在自己那一回合里产生的东西进**它自己的事件流**（`sid` 就是那条流）——
  流式增量、核实行、**定稿的转录行**、运行态都是。主会话只拿"谁说了什么"的投影；
  子会话拿完整的一份（含定稿行）。客户端打开哪个标签页读哪条流，不靠"从别人的流里猜自己"。
- **连接状态只看事件流的取用**：客户端把"取事件失败"（服务没起来 / 连接断了）与"事件应用/渲染出错"分开——
  后者不是断线，如实报错并留在日志里；两类都要落到日志，不能只亮状态点
  （传输层 `src/presentation/web/assets/app/core/events.js` 把两类分别交给 `onConn` / `onError`）。
- **客户端请求有界**：`api()` 的一次调用可以带 `timeoutMs`（`src/presentation/web/assets/app/core/http.js`）——
  到点用 `AbortSignal` 中止并抛带 `timeout: true` / `status: 0` 的 `Error`；不带就不设上限
  （建会话、补编、上传都是长动作，按固定秒数掐会误杀）。**装饰性的状态读取必须给上限**
  （`STATE_TIMEOUT_MS = 10000`，`src/presentation/web/assets/app/features/shell/shell.js`），
  且**不许拿它当某个动作的完成门槛**：关弹层、收卡片只看它自己那条命令的回包，
  状态刷新慢或失败只让侧栏晚一点刷出来（`src/presentation/web/assets/app/features/wizard/wizard.js`）。
  理由：请求排在浏览器队列里时既不回也不报错，界面就永远停在「正在创建」，用户既等不到结果也看不到失败。
- **并发归核心**：`stop` 直接置位核心内部的取消标志，**不进命令队列**，所以生成期间照样立刻生效；
  呈现层不需要知道「生成时核心状态被占用」这类内部事实。
- **生成不占命令队列**（单 agent 与协作的长步骤都是）：命令队列只做**状态变更**与**派发**——生成时先把会话对象
  **取出来**交给工作线程独占（核心表里留"生成中"这一态，读接口照常列出它），跑完再**交回**核心
  重新插入并落盘。于是生成期间读接口（历史 / 状态）与其它会话的命令都照常返回，
  而**所有状态变更仍只发生在核心线程上**（工作线程只跑生成，不碰核心状态）。
  取出的窗口内，改配置 / 回档 / 删除一律如实拒绝（文案见 `Core::running_refusal`）。
  线程崩溃时只解除"生成中"：转录在盘上，下次访问按它重建，绝不把会话卡死。
  协作的长步骤（开始讨论 / 回答 / 继续）走同一条路，且事件**边产边送**事件台——用户能看着讨论
  一轮轮推进，而不是等整段结束才一次性出现。
- **「停止」在协作里同样立刻生效**：取消标志由派发时注入协作的泵，泵在**每次模型调用前**与
  **调用中途**（分片回调返回 false）都看它，所以一个成员说到一半就能停下。
  被中断的那条发言**不吸收**进协作席位——半截 `say`/`agree` 进转录会把"轮到谁、同意了没"算歪；
  单 agent 已经写出来的那半句留在转录里、标「（已中断）」，但不进模型上下文（不算这一轮的回答）。
  中断后会话保持 `active`，输入区照常可写，想让被打断的那轮接着走再点「继续」（重开一轮、不续写半句，也**不替用户发一句"继续"**）；
  「停止会话」是另一件事：会话落成历史态、输入区收起（见下）。停止与失败用**两句不同的话**如实告知。
- **历史态不渲染输入区**：只读历史与已停止都收起输入区（`syncComposer` 看 `readonly || run === 'stopped'`），按状态给唯一入口——`archived` 给「恢复归档」，`stopped` 与其余给「启用会话」；两者都不跑模型。
  `SessionView.continue` 由后端给（单 agent 上一轮被打断 / 协作泵），前端只按它渲染上下文「继续」，不自己判断；`stopped` / `archived` / 正在跑 / 代理一律不给（`Work::continue_action`）。
- **恢复入口 = f(这条视图自己的 run, 在跑)**：正在跑时**一个按钮都不给**（那一轮跑完条子自己换回「启用会话」），
  判据取视图自己的 run 与运行态，**不取历史列表里那份打开时的快照**——快照会一直停在「已停止」。
  理由：会话被解冻的那一刻 `run` 就变了，任何按快照渲染的入口都会在生成期间变成假入口（`app/features/transcript/page.js`）。
- **「继续」不等回包**：`control_session{action:continue}` 在服务端是**同步跑完整轮**才返回（`TurnOps::continue_flow`，
  `src/capabilities/turn/turn/api.rs`），所以界面发出去就立刻对账并启用视图，进度与失败都从事件与转录看；
  等回包再启用会让整轮回答都停在只读条后面，用户既看不到回答也点不到「中断」（`app/features/actions/actions.js`，
  同一个理由也适用于输入区的「启用会话」：它只解冻、不跑模型，回包即真相）。
- **一次命令 panic 不带垮核心**：接住并继续服务（回包通道断开，调用方得到「无回应」）。
- **传输的线格式归呈现层**：请求形状、路由、错误码在 `src/presentation/web/routes.rs`；**事实**的线格式（`SessionEvent`、`*View`）
  仍在协调业务：通用事件在 `session`，机制自己的事件走 `SessionEvent::Push`、词汇归各自能力（协作在 `collab`）。两者不混——混在一起就是「到处内联 JSON 拼装」的成因。
- **工具卡按结构化轻重上色，说明书由词表渲染**：工具行带 `severity`（`info` / `warning` / `error`）与 `notes`（稳定码 + 类型化参数），
  卡片据此三色（绿勾 / 黄叹号 / 红叉）并把 `notes` 逐行接在正文后（`src/presentation/web/assets/app/features/transcript/parts.js`）；
  **呈现层不解析回执正文**——结束状态、退出码、落点、耗时都是工具行里的结构化字段。
  词表来自 `/api/state` 的 `i18n.messages`（后端给模板源文本，前端只做 `{ $arg }` 替换；
  目录缺失的码按 `[code]` 原样显示，不编造文案）（`src/presentation/web/assets/app/core/i18n.js`）。

### 传输选择：长轮询

事件台是**服务端单向推送**（客户端不在实时通道上发东西——命令走普通 HTTP POST）。传输用**长轮询**：
`GET /api/events?since=N` 最多挂 20 秒，有新事件立即回；客户端拿 `head` 推进游标、按 `seq` 增量取。
`oldest` 是事件台里**还留着的最老序号**：事件台会裁剪（`BUS_MAX`/`BUS_KEEP`），`since` 之后那段
可能已永久丢失——客户端据此**重新对齐**（拉一次历史重放），而不是按 `seq` 干等
（干等会让后续批次全部滞留，只有刷新页面才恢复）。

选它而不是 SSE 的理由（针对本项目的形态：**本机、单客户端、直连无代理**）：

- 挂起的请求在页面关闭/空闲时**释放**；SSE 会让**每个标签页永久占一条连接**（HTTP/1.1 每域 6 条）；
- SSE 的主要优势（内建重连、省掉每请求开销）在这里没有痛点：本机直连、客户端已有失败重试、开销可忽略；
- **传输可替换**：契约在**事件台**（一个可重读的日志）——换 SSE / WebSocket 只换"怎么把批次送到前端"，
  不改事实词汇与命令形状。

**顺序与去重（客户端硬约束）**：事实只有**一条来路**（事件台）。命令回包只给 `head`，
客户端因此只需维护一个单调游标 `appliedSeq`：取到的批次按 `seq` 排序后应用；
`oldest` 已经越过游标说明那段被裁剪了，就拉一次历史重放**重新对齐**。
**会话表也只由服务端给**：客户端的会话来自 `/api/state` 的 `sessions`，只把事件应用到它已有的会话；
收到**不认识的 sid**（典型是系统会话，见 [session-model.md](../session/session-model.md) 的推/落表）一律不动。

**历史与实时只有一条流（客户端不合并两个来源）**：`GET /api/history/{name}` 一次给全——
盘上转录 `events` + 事件台上**它之外**的尾巴 `live` + 合流时的头部序号 `head`。
回档过的会话从**最近一次 `rewound` 之后**取尾巴（重置水印）；水印之前的批次已被盘上转录取代。
判据是**结构化相等**（两边的 JSON 出自同一套序列化器，就是同一个值），**不是按行 id 猜**：
`node_started`/`report` 与瞬时提示本来就没有 id；内容类系统提示已经是 `transcript` 里的 `notice` 行。
按 id 去重正是"刷新后整段重复"的来源。
客户端把 `head` 记成这条会话的**水位**，此后 `seq ≤ 水位` 的批次一律丢掉（合流里已经给过），
只按序 append 水位之上的实时批次。
`live` 只取「转录在事件台里的最后一个位置之后」——转录之前的短暂事件（流式增量、开跑那条运行态）
已被盘上定稿行取代，跟下去只会画出一个填不上的空块。
## 二、HTTP 路由目录（机器可读）

`src/presentation/web/routes.rs` 的 `ROUTES` 是路由的**唯一定义**：`src/presentation/web.rs` 的匹配与分发都由它驱动
（匹配由目录做、分支按 `id`），所以「代码里有路由但目录里没有」在结构上不可能发生。
`solomni --print-routes` 输出它的 JSON（含能力、请求/响应形状、状态码与说明）。

下面这张表由契约测试与 `ROUTES` 机器比对——对不上就是测试失败，不是靠人记得改文档：

静态资源只有两条路由：`/` 给 `index.html`，`/assets/{*path}` 给其余文件。文件来自编译期嵌入表（`build.rs` 扫描 `src/presentation/web/assets/`，`.cjs` / `.mjs` 开发脚本不进表；未知扩展名让构建失败），
请求只查这张表、不读文件系统，表外路径一律 404（`src/presentation/web/assets.rs`）。增删前端文件**不改路由**。
静态资源一律 `Cache-Control: no-store`：改了前端**刷新即生效**，不需要硬刷新（`src/presentation/web.rs` 的 `static_head`）。

<!-- ROUTES:BEGIN -->
| 方法 | 路径 | 能力 | 请求 | 响应 | 状态码 |
| --- | --- | --- | --- | --- | --- |
| GET | `/` | 静态资源 | — | `index.html` | 200 |
| GET | `/assets/{*path}` | 静态资源 | — | 编译期嵌入表里的文件 | 200, 404 |
| GET | `/api/events` | 事件台（`EventBus`） | 查询 `sid` / `since` | `{lines:[{seq,sid,events}],head,oldest}` | 200 |
| GET | `/api/state` | `ModulesOps::roster / module_enabled / service_enabled` + `WorkOps::session_views` + `RegistryOps` + `HistoryOps::list` | — | `{modules,services,rejected,fence,providers,models,core,agents,settings,sessions,history}`（`modules[].runtimes` 是模块声明的运行能力，创建向导据此预判虚拟根够不够；`modules[].version` 是模块自己的版本、`modules[].system` 是给 agent 的说明（模块页直接显示），`modules[].enabled` 与 `modules[].services[].enabled` 是**有效启停**；`services` 是常驻服务的**运行态** `{module,name,adapter,state,reason,operations}`；`sessions[].pending` 是**当前那张裁决卡**的快照，与推的 `decision_card` 同一份；`sessions[].context` 是最近一次模型调用的上下文用量，只对 `Session::Single`（single / proxy / agent 子会话）存在；`sessions[].usage_totals` 是 meta 里累计的模型花费（calls / prompt / completion / cache_read / cache_creation，全零 = 还没落过精确调用）；`sessions[].context_freshness` 与 `freshness_keep_percent` 取工作根 meta（过期占位符开关与保底百分比），`freshness_stats` 是最近一次发送视图的替换统计（只对在内存里的会话有）；`sessions[].shell` 是最近一次 shell 的运行状态（读工具行的结构化事实 `tool.shell`，不解析回执正文），没跑过就是 null；`i18n` 是回执说明的界面词表 `{lang,available,messages}`——语言按设置协商，`messages` 是稳定码到模板源文本的映射，前端只做 `{ $arg }` 替换） | 200, 400 |
| GET | `/api/actions` | `ActionOps::catalog` | 查询 `sid` | `{actions:[{id,desc,params,available,reason}]}` | 200, 400 |
| POST | `/api/actions/{id}` | `ActionOps::act` | `{…按动作声明}` | `{head}` / `{events}`（重放快照） / `{ok,…}` | 200, 400 |
| GET | `/api/sessions/{sid}/config` | `WorkOps::config` | — | `{config}` | 200, 400 |
| GET | `/api/sessions/{sid}/files` | `WorkOps::files` | — | `{work,agents,roots,usage}` | 200, 404 |
| GET | `/api/sessions/{sid}/work` | `WorkOps::work_log` | 查询 `limit` / `before` | `{head,commits:[{id,parent,author,session,time,message,changes,on_head}],total,truncated}` | 200, 404 |
| GET | `/api/sessions/{sid}/work/{commit}` | `WorkOps::work_commit_view / work_commit_file` | 查询 `path`（可选） | `{id,parent,author,session,time,message,changes,changes_truncated}` / `{path,hash,size,media_type,text,truncated,too_large}` | 200, 404 |
| GET | `/api/dirs` | 宿主目录浏览 | 查询 `path`（可选；省略 = 用户主目录） | `{path,parent,roots:[盘符],dirs:[{name,path}]}` | 200, 400 |
| GET | `/api/settings` | `RegistryOps::settings` | — | `{settings}` | 200, 400 |
| GET | `/api/history` | `HistoryOps::list` | — | `{sessions}` | 200, 400 |
| GET | `/api/history/{name}` | `HistoryOps::open` | — | `{meta,events,live,head,shell}`（`shell` 从转录派生：回放/重建后的状态栏快照；读工具行的结构化事实 `tool.shell`，不解析回执正文） | 200, 404 |
| POST | `/api/history/{name}/delete` | `HistoryOps::delete` | — | `{ok}` | 200, 400 |
| GET | `/api/tiers` | `WorkOps::tier_choices` | — | `{tiers:{default,vm_available,vm_unavailable_reason,vm_requirements,guests,available}}` | 200, 400 |
| GET | `/api/processes` | `Provision::processes` | — | `{procs:[{pid,alive,purpose,owner,since,orphanRisk,guardNote}]}` | 200, 400 |
| GET | `/api/setup/status` | `Provision::status + job` | 查询 `since` / `wait` | `{deps:[{id,guest,present,install,probe,source,url,size_mb,note}],job:{running,seq,lines,error},missing,pending:[id]}` | 200, 400 |
| POST | `/api/setup/ensure` | `Provision::status + start` | `{deps?:[id],confirm?:bool}`（不点名 = 只补现在缺的） | `{preview:[{id,url,size_mb,present}]}` / `{ok,deps}`（都装好了 → `{ok,deps:[]}`） | 200, 400 |
| DELETE | `/api/setup/job` | `Provision::cancel` | — | `{ok}` | 200 |
| GET | `/api/storage` | `Storage::report` | — | `{report:{generatedAt,managedBytes,capBytes,categories,policy,job}}` | 200, 400 |
| POST | `/api/storage/gc` | `Storage::gc` | `{apply?:bool}` | `{ok,report:{actions,skipped,freedBytes,overCap}}` | 200, 400 |
| POST | `/api/storage/clean` | `Storage::clean` | `{kind,id?,module?,confirm:bool}` | `{ok,result}`（report 或 started 作业） | 200, 400 |
| POST | `/api/suggest-models` | `WorkOps::suggest_models` | `{task,mode}` | `{ok,agents}` | 200, 400 |
<!-- ROUTES:END -->

