# assets（随包只读内容）

> 定位：`prompts/`、`systools/`、`locales/` 这三块**随包只读内容**的唯一主人——介质名、固定文档清单、按文档读文本、提示词册的目录枚举与排序。
> 效力：规范｜核对：2026-11-25｜依据：`src/units/resource/assets/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：**落点**——`prompts/`、`systools/`、`locales/` 三处的拼法（只经 `layout` 的介质名，别处不再 `join`）。
- 管：**固定文档清单**——`systools/` 下可读的 `tools.yaml` / `names.yaml` / `profiles.yaml` / `loops.yaml`（`Doc` 一个变体一份），文件名只在这里出现。
- 管：**读法**——按文档读全文（`read`）；提示词册递归收 `.yaml`、按路径排序、逐份读出（`prompt_docs`）。装配必须确定：同一份代码在任何机器上装配出同一册子。
- 不管：**文档形态与解码**——`tools.yaml` 的 `ToolBook` 归 `tools`，`profiles.yaml` 的 `Profiles`、`loops.yaml` 的 `Loops` 与校验规则归 `session_model`，提示词册的合并与渲染归 `prompt`。资源单元不依赖任何业务 DTO。
- 不管：**`commands.yaml`**——它由 `tools` 的 `include_str!` 编译期编入，不是运行期盘 IO，因此不在 `Doc` 里。
- 不管：**`locales/` 的运行期读取**——文案在编译期由 `kernel` 的 `include_str!` 编进来；本单元只给落点，免得别处另拼。
- 不管：**`bin/` 与 `modules/`**——前者由 `layout` 给、后者是模块源码（归 `content` 的私有区与之分离）。

## 入站契约与状态归属

- 入站面：`api`——`Assets`（`read(doc)` / `prompt_docs()`）、`Doc`（四份固定文档）、`PromptDoc`（一份提示词文档）、`paths(install) -> AssetPaths`（落点派生，纯函数，今天没有生产读者）。
- 出站端口：`AssetFiles` **只由 `service.rs` 持有**（R12）；真实实现 `detail::FsAssetFs` 只由组合根构造。
- 状态归属：**无内存状态**——`AssetsService` 只持落点派生器与端口，每次读都重新落盘取；不缓存文件内容。
- 契约方向：`prompt` / `tools` / `conversation` 的装配输入适配器持有 `Arc<dyn Assets>`，本单元**不依赖任何业务单元**。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`（`layout` 的介质名）
- 谁在用我：`prompt`（提示词册）、`tools`（工具总表与保留名）、`session_model`（会话形状的两份声明）、`entry`（组合根装配）

## 一、盘上有什么

- 落点拼法只有一处：`src/units/resource/assets/domain/paths.rs`；新增一类随包内容落点 = 在那里加一个方法，别在消费方 `join`。
- 文档名也只有一处：`src/units/resource/assets/domain/doc.rs` 的 `Doc::file_name()` 与 `Doc::label()`（后者是给错误文案用的相对路径）。

| 落点 | 路径 | 内容归谁解释 | 可写 |
| --- | --- | --- | --- |
| 提示词册 | `prompts/`（递归 `.yaml`） | `prompt`（合并成 `Prompts`） | 否 |
| 工具总表 | `systools/tools.yaml` | `tools`（`ToolBook`） | 否 |
| 名字表 | `systools/names.yaml` | `tools`（保留名） | 否 |
| 会话类型表 | `systools/profiles.yaml` | `session_model`（`Profiles` + 校验） | 否 |
| 流程段表 | `systools/loops.yaml` | `session_model`（`Loops` + 校验） | 否 |
| 命令表 | `systools/commands.yaml` | `tools`（编译期 `include_str!`） | 否 |
| 界面语言 | `locales/<tag>.ftl` | `kernel`（编译期 `include_str!`） | 否 |

- 这些内容随包分发、可能装在只读位置，**运行期只读**。但它们仍是资源单元：只读不代表以后只读——一旦要支持热更新或语言包安装，落点与清单已经在这里，改的是本单元而不是二十处调用点。

## 二、为什么 `systools/` 要收口到一个单元

- 以前这块目录**一目录两主**：`tools` 读 `tools.yaml` / `names.yaml`，`conversation` 读 `profiles.yaml`，两边各自 `join` 目录名、各自 `read_to_string`、各自写错误文案（今天两份声明都由 `session_model` 读，见 `src/units/business/session_model/detail/yaml_decls.rs`）。
- 目录名一旦要挪（比如随包内容换到别处），得同时改两处业务代码；而"这份目录里到底有哪些文档"没有人回答得出来。
- 现在：目录与文档清单归本单元，业务只问"给我 `Profiles` 的文本"；错误文案由消费方补语义（"会话类型表读不了"），文件名从 `Doc::label()` 取，不再在文案里手写路径。

## 三、待办（本阶段没做，别当成已完成）

- **`locales/` 没有运行期读者**：文案编译期编入，因此 `AssetPaths::locales()` 与 `layout::locales()` 今天只有测试在读（已注明 `#[allow(dead_code)]`）。真要支持多语言热加载，从这里起步。
- **没有热更新 / 安装**：随包内容只随发行版走；要支持"安装语言包 / 提示词包"，得先给本单元加写入面与清单（不是拼路径的事）。
- **`systools/` 的其余文档**：今天只有三份在 `Doc` 里；新增一份 = 加一个变体 + 一行 `file_name`/`label`，并同步本页 §一 的表格。

## 改动本单元时必须同步

- 新增 / 改动落点或文档 → 本页 §一 的表格与 [state.md](../architecture/state.md) 的盘面契约。
- 改动端口形状（`AssetFiles`）→ [docs/testing/doubles.md](../testing/doubles.md) 的端口矩阵。
- 改动装配输入 → [docs/tools/README.md](../tools/README.md)（工具总表与保留名）、[docs/session/README.md](../session/README.md)（会话类型表）、[docs/prompt/README.md](../prompt/README.md)（提示词册）。
- 盘面布局改动的唯一权威是 `src/kernel/domain/layout.rs`；本页不重复常量。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
