# 呈现层入站契约（命令/事件 + 路由目录）

> 本文是**呈现层入站契约的唯一权威**：能力接口、事件台、命令/事件规则，以及机器可读的 HTTP 路由目录。
> 分层与端口见 [ARCHITECTURE.md](../../ARCHITECTURE.md)，模块地图见 [module-map.md](module-map.md)。
> **下面的路由表由契约测试机器比对**（`src/tests/routes.rs` 直接读本文件）：表与
> `presentation/routes.rs` 的 `ROUTES` 对不上就是测试失败，不靠人记得改文档。

## 一、能力接口与事件台

**核心常驻自己的执行线程、独占全部状态**：呈现层拿不到 `Core`、也拿不到任何核心锁。两侧只通过两样东西来往：

| 东西 | 定义在 | 形态 |
| --- | --- | --- |
| 能力接口 | `core/api.rs` | `SessionOps` / `RegistryOps` / `HistoryOps` / `DiscoveryOps`（全 `&self`，可替换成假实现） |
| 事件台 | `core/api.rs` | `EventBus`：核心独占生产，任意数量的消费者按序号增量取 |

规则：

- **命令**：呈现层调能力接口 → 核心在自己的线程上执行 → 同步回包（`Advance`：本批事件 + 事件台序号）。
- **事件**：生成过程中的短暂事件与最终事件都进事件台；Web 长轮询按 `since` 取，客户端按 `seq` 去重。
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
  被中断的那条发言**不吸收**——半截 `say`/`agree` 进转录会把"轮到谁、同意了没"算歪；
  会话保持可继续（点「继续」从断点推进），停止与失败用**两句不同的话**如实告知。
- **一次命令 panic 不带垮核心**：接住并继续服务（回包通道断开，调用方得到「无回应」）。
- **传输的线格式归呈现层**：请求形状、路由、错误码在 `routes.rs`；**事实**的线格式（`SessionEvent`、`*View`）
  仍在 core。两者不混——混在一起就是「到处内联 JSON 拼装」的成因。

## 二、HTTP 路由目录（机器可读）

`presentation/routes.rs` 的 `ROUTES` 是路由的**唯一定义**：`web.rs` 的匹配与分发都由它驱动
（匹配由目录做、分支按 `id`），所以「代码里有路由但目录里没有」在结构上不可能发生。
`solomni --print-routes` 输出它的 JSON（含能力、请求/响应形状、状态码与说明）。

下面这张表由契约测试与 `ROUTES` 机器比对——对不上就是测试失败，不是靠人记得改文档：

<!-- ROUTES:BEGIN -->
| 方法 | 路径 | 能力 | 请求 | 响应 | 状态码 |
| --- | --- | --- | --- | --- | --- |
| GET | `/` | 静态资源 | — | `index.html` | 200 |
| GET | `/style.css` | 静态资源 | — | `style.css` | 200 |
| GET | `/app.js` | 静态资源 | — | `app.js` | 200 |
| GET | `/md.js` | 静态资源 | — | `md.js` | 200 |
| GET | `/api/events` | 事件台（`EventBus`） | 查询 `sid` / `since` | `{lines:[{seq,sid,events}],head}` | 200 |
| GET | `/api/state` | `DiscoveryOps` + `RegistryOps` + `HistoryOps` | — | `{modules,rejected,fence,providers,models,core,agents,settings,sessions,history}` | 200, 400 |
| POST | `/api/sessions` | `SessionOps::create_work` | `{name,mode,agents[],task?,delegate?}` | `{sid,agents,events}` | 200, 400 |
| POST | `/api/sessions/{sid}/{action}` | `SessionOps` + `intent::act` | `{text?,agent?,id?,overwrite?,data_base64?,编辑体}` | `{sid,events,seq}` 等 | 200, 400, 404, 409 |
| GET | `/api/sessions/{sid}/config` | `SessionOps::config` | — | `{config}` | 200, 400 |
| GET | `/api/sessions/{sid}/files` | `SessionOps::files` | — | `{work,agents,roots}` | 200, 404 |
| POST | `/api/providers` | `RegistryOps::upsert_provider` | `{id,base_url,api_key}` | `{ok}` | 200, 400 |
| POST | `/api/providers/{id}/{action}` | `RegistryOps::remove_provider` / `discover_models` | — | `{ok}` / `{ok,models}` | 200, 400, 404 |
| POST | `/api/models` | `RegistryOps::upsert_model` | `{id,name,api_model,provider,note?}` | `{ok}` | 200, 400 |
| POST | `/api/models/{id}/{action}` | `RegistryOps::remove_model` / `set_core_model` / `probe_model_tools` / `probe_replay_shape` | — | `{ok}` / `{ok,outcome,detail,mode}` / `{ok,shapes}` | 200, 400, 404 |
| POST | `/api/agents` | `RegistryOps::upsert_agent` | `{name,modules[],model?,note?}` | `{ok}` | 200, 400 |
| POST | `/api/agents/{name}/{action}` | `RegistryOps::remove_agent` | — | `{ok}` | 200, 400, 404 |
| GET | `/api/settings` | `RegistryOps::settings` | — | `{settings}` | 200, 400 |
| POST | `/api/settings` | `RegistryOps::set_settings` | `{streaming?,show_reasoning?}` | `{ok}` | 200, 400 |
| GET | `/api/history` | `HistoryOps::list` | — | `{sessions}` | 200, 400 |
| GET | `/api/history/{name}` | `HistoryOps::open` | — | `{meta,events}` | 200, 404 |
| POST | `/api/history/{name}/delete` | `HistoryOps::delete` | — | `{ok}` | 200, 400 |
| POST | `/api/suggest-models` | `DiscoveryOps::suggest_models` | `{task,mode}` | `{ok,agents}` | 200, 400 |
<!-- ROUTES:END -->

