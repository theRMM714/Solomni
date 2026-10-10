# prompt（提示词册）

> 册子的唯一持有者：按名字给段，不替业务拼回合。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：提示词册（`prompts/`）的内存形态与 `{{key}}` 渲染、`@` 引用改写成真实路径、按名字取段、两块共享记录（工具文本 / 引用）。

**不管**：**不替业务拼「哪个回合发哪几段」**（组装留在各业务）；不落盘；界面通知文案不进册子。

## 二、入站契约与状态归属

`api::Prompt`（`text` / `render` + `tools()` / `refs()`）+ 词汇（`Segment` / `ToolTexts` / `RefsPrompts` / `RefRoots` / `rewrite` / `Vars`）。端口 `PromptSource` 由本单元的装载入口 `load()` 使用，组合根注入。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`
- 谁在用我：`collab`、`conductor`、`llm`、`registry`、`session`、`slate`、`tools`、`workspace`

## 四、改动本单元时必须同步

- 册子只被本单元持有一次（组合根装载后把 `Arc<dyn Prompt>` 注入协调业务）；**缺文件 / 缺键 / 缺变量 = 报错暴露**，禁止静默兜底文案；键清单一改 → `prompts/`、本目录 `prompts.md`、`src/tests/prompt.rs`。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
| [`prompts.md`](prompts.md) | `prompts/` 的结构与键清单 |