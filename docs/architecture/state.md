<!-- docs-template: spec -->
# 状态、落盘与运行时契约
> 定位：状态归谁、什么落盘、日志与提示词册的契约；改布局与改状态前先读本文。
> 效力：规范｜核对：2026-11-17｜依据：`src/capabilities/conversation/session/domain/history.rs`、`src/kernel/detail/file_log.rs`

## 适用与判据
- 动状态归属、落盘布局、日志埋点或提示词册前读本文；违反由 T0 结构审查（状态归属）与文档审查判定。

## 一、每个状态只有一个家
- 会话状态归 `session`、构建状态归 `builds`、空间台账归 `storage`；别的单元读写只能经该能力的 `api`，不直接读对方文件（R1 / R4，判据在 `tests/gates/structure/dependency.js`）。
- **跨层共享的形状由 `kernel` 托管**（R6）：`kernel` 只存放与搬运 DTO，不解释业务语义、不构造业务决策与面向用户的文案；业务自己的入参 / 出参留在本能力 `api`。
- **持状态的能力只能写自己的状态**；别人的状态只经 `api` 拿（`src/capabilities/runtime/service/core.rs` 只聚合 `turn` / `work` / `lifecycle` 与各能力的 `api`，不代写它们的状态）。
- 状态归谁的三条判据（有不变式的参与方 / 依赖图上最高的使用方 / owner 的派生值）见 [boundaries.md](boundaries.md)。

## 二、落盘契约
- **转录即状态**：会话内容 = 按 `rewind` 标记回放出的活动窗口；内存与落盘不一致时**以流水为准**。
- **能重建的不落盘**：视图、索引、账本缓存与执行计划是派生物，重启后由真相重建；写盘只写派生不出来的事实。
- **落盘路径相对根派生**：一律用 `PathBuf` 组件拼接、不写机器绝对路径（R9）；布局名字集中在 `src/kernel/domain/layout.rs`（`.home` / `.vm` / `runtimes` / `guest.yaml`），不散写字符串。
- 含机器路径的内容不进仓库（R9）；对外书写用 `/`，见 [rust-dev.md](rust-dev.md) 的跨平台机制。
- **改布局同次改三处**：`docs/<单元>/unit-map.md`、本文对应节与 [../../AGENTS.md](../../AGENTS.md) 的「文档路由」（R8）。
- **权限与围栏**：策略在 `src/capabilities/tools/domain/fence.rs`，机制与落点在 `src/kernel/detail/confine/`；规则见 [../permission/README.md](../permission/README.md)，平台差异见 [../kernel/README.md](../kernel/README.md) 的「围栏落点：必要与可选」。

## 三、运行日志（`Log` 端口）
- 业务只调用 `Log`（`info` / `warn` / `error`），机制与落盘在 `src/kernel/detail/file_log.rs`；组合根注入唯一实例。
- **关键节点必须埋点**：通道降级、HTTP 失败、会话动作失败、装配失败、工具执行异常——出问题时**看日志定因**，不靠推理猜（`src/kernel/detail/file_log.rs`）。
- 按运行分文件、目录与不入库规则见 [../kernel/README.md](../kernel/README.md) 的「运行日志」。

## 四、提示词册（`prompts/`）
- **所有发给 LLM 的指令、身份、协议、环境提示词一律写入 `prompts/`**，禁止硬编码进代码；改文案只改册子。占位符 `{{key}}` 在调用现场替换，缺文件 / 缺键 / 缺变量**报错暴露**，禁止静默兜底（`src/capabilities/prompt/domain/prompt.rs`）。
- **册子只由提示词能力持有一次**（`Arc` 共享）；别的能力**按名字取段**（`Prompt::text` / `render` + `Segment`），不点字段路径；"哪个回合发哪几段"的组装留在各业务。
- **不进册子的两类**：行身份是结构化字段（认 `LineView` 的 `speaker` / `verb` / `kind`，不认文案）；只给用户看的呈现层文案（各类 `Notice`、工具轨迹行的成败字样、面向界面 / CLI 的 `Err`）。
- 运行时回执**按产生它的模块归属**：判断标准是"谁做决定谁拥有文案"，不是"进上下文就进册子"。
- 键清单与册子规则：[../prompt/prompts.md](../prompt/prompts.md)。

## 检查
- 状态归属与跨能力只走 api → `tests/gates/structure/dependency.js`。
- 布局名字不散写 → 结构审查的 `dependency.js` / 注释契约；布局常量在 `src/kernel/domain/layout.rs`。
- 文档与布局同步 → `node run-tests.js --audit=docs`、`node tests/docs-sync.mjs`。

## 路由
- 会话目录布局、转录与行结构、回档与压缩：[../session/session-model.md](../session/session-model.md)
- 共享区与版本化工作区：[../workspace/README.md](../workspace/README.md)
- 落盘机制、进程台账与回收、围栏落点、运行日志细则：[../kernel/README.md](../kernel/README.md)
- 登记处四份 yaml：[../../REGISTRY_SPEC.md](../../REGISTRY_SPEC.md)
- 提示词键表：[../prompt/prompts.md](../prompt/prompts.md)
