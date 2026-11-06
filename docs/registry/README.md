# registry（登记处）
> 定位：供应商 / 模型 / agent / 设置四份 yaml 的内存形态、解析链、写面与探测行为。
> 效力：规范｜核对：2026-11-04｜依据：`src/capabilities/registry/api.rs`

## 管什么 / 不管什么
- 管：四份 yaml 的内存形态与用例、解析链（含思考强度目录与模型映射）、agent 登记处与名字校验、代拟名单落地、密钥回显边界。
- 不管：不建通道（`llm` 的 `detail`）、不替用户选模型（用户选，登记处只解析）、不碰会话。

## 入站契约与状态归属
- 呈现层队列面：`api::RegistryOps`（登记 / 移除 / 设核心默认 / 探测 / 改设置）。
- 能力面：`api::Registry`（读 `&self`、写 `&mut self`；`probe_replay_shape` 只读）。
- 状态住在协调业务的执行线程上，靠单线程命令队列互斥，不额外上锁。
- 出站端口 `SettingsStore` 只由 `service.rs` 持有。
- 落盘形态归本单元：`.home/` 下四份 yaml，读写与解析在 `src/capabilities/registry/detail/yaml_settings.rs`。

## 依赖图位置（由源码的 `::api` 引用推导）
- 经 `::api` 用到：`kernel`、`llm`、`modules`、`permission`、`prompt`、`session`
- 谁在用我：`cli`、`collab`、`conductor`、`slate`；`web` 经 `Ops.registry`（`RegistryOps`）调用

## 一、工具调用探测（`--probe-tools`）
- 把"支不支持 function calling"变成事实，而不是靠猜：命令行跑 `solomni --probe-tools <模型 id>`。
- 适配层发两条最小请求对比：不带工具声明的、带工具声明的。
- 不带就失败 → 如实报"通道本身没打通"（不当成"不支持工具调用"）。
- 带工具声明被拒（附供应商原话）→ 判不支持，并把 `tools` 写回 `envelope`。
- 返回了工具调用 → 判支持，并把 `tools` 写回 `native`。
- 带了声明但这次没发起调用 → 判无法判定，只报事实、不改登记处（由用户决定填哪个）。
- 只有确凿的结论才会写回 `models.yaml`；无法判定时绝不替用户拍板。

## 二、回放形状探测（`--probe-replay`）
- 探"工具调用历史怎么发回供应商才收"：命令行跑 `solomni --probe-replay <模型 id>`。
- 适配层把同一件事的几种写法各发一次：基线是现在线上真在用的文本回放，其余是 `assistant` 带 `tool_calls` + `role=tool` 结果的几种可判定变体（正文空串 / null / 有正文 / 结果消息带 name）。
- 逐项报两件事：供应商收了没有（HTTP 层），以及模型真的读到了没有（回答里带回工具结果里那个编号才算读懂；收了不等于看懂了）。
- 它不改登记处：只报事实，采不采用由人定。
- 界面上：模型登记里那两行按钮（测工具调用 / 测回放形状）走的就是这两个探测（`model.act` 的 `probe` / `probe-replay`），结论逐项显示。

## 三、保存闸门（原生形态）

- 原生形态的模型在**保存前**实测三件事：通道基线（不带 tools）、带 tools 的最小调用、实际要用的回放形状。
- 只有「明确支持 + 回放形状被接受」才落盘；明确不支持 / 无法判定 = **不保存**，只如实报原因，不静默降级成手写信封，也不引导用户改信封。
- 手写信封是用户显式选择：**不测试，直接保存**（通道是否可用由使用者自己负责）。
- 所有写入口（Web 模型表单、CLI、`upsert_model` 动作）都走同一道闸（`RegistryOps::upsert_model`），不各写一份。

## 四、写回与写面
- 探测写回只发生在 `--probe-tools`：`envelope` / `native` 都写回 `models.yaml` 的 `tools`；`--probe-replay` 永不写回。
- 写面（登记 / 移除 / 设核心默认 / 探测 / 改设置）以用户动作暴露：声明在 `systools/tools.yaml`（`callers: [user]`），执行体在 `src/capabilities/conductor/api/action.rs` 的 `execute_registry`。
- 这些动作默认不开放给任何角色（供应商与密钥是产品级资源）。
- 改一个写方法要同时改动作表、执行体与前端调用点。
- 密钥回显边界：界面只呈现「供应商 id + 端点」与「模型 id + 展示名」，永不携带密钥；更新供应商时 `api_key` 留空 = 保留原值。

## 四、四份 yaml 的读写
- `.home/` 下四份文件分开读写：`load` 逐份读，缺哪份就用哪份的默认；`save` 会同时写四份。
- 结构定义在 `src/capabilities/registry/domain/providers.rs` 与 `src/capabilities/registry/domain/agents.rs`；解析与写盘在 `src/capabilities/registry/detail/yaml_settings.rs`。
- yaml 非法（解析失败）= 装配失败并指出是哪一份。
- 字段、默认值与拒收规则见 `REGISTRY_SPEC.md`。

## 改动本单元时必须同步
- `REGISTRY_SPEC.md` → 字段与拒收规则的唯一权威；`src/tests/registry.rs` → 用例。
- `systools/tools.yaml` + `src/capabilities/conductor/api/action.rs` + `web` 的设置页 → 写方法三处同改。
- `docs/permission/README.md` → 权限与围栏语义；密钥永不进入提示词 / 转录 / 日志 / 模块工作区。

## 本目录
| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
