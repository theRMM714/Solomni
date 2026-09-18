# Solomni


## 作者前言 / Author's Preface

### 因为 AI 总结经常过于专业化、文档化，所以我会先说几条重点。 / Because AI summaries are often too specialized and documentation-like, I will first state a few key points.

- 这个项目不是给 agent 附加工具或技能，而是给一个工具或技能赋予一个 agent。只需要制作成 modules。 / This project is not about attaching tools or skills to an agent; rather, it is about giving an agent to a tool or skill. You only need to build it as a module.
- modules 不用引入核心框架的任何包、任何代码，只需要用 YAML 遵守一份契约。 / Modules do not need to import any packages or any code from the core framework; they only need to follow a contract using YAML.
- modules 可以是 skill，可以是 MCP，可以是你手上任意语言写的任意脚本、任意工具，也可以只是一份提示词。 / Modules can be skills, MCPs, any script written in any language you have on hand, any tool, or even just a prompt.
- 所以实际上，你的工具属于你，不属于平台；平台反过来依赖你的工具进行工作，离开了平台，你的工具不会有任何影响。 / So in reality, your tools belong to you, not to the platform. The platform, in turn, depends on your tools to work. Without the platform, your tools will not be affected in any way.
- 你可以把任意数量的 modules 赋予任意数量的 agent，从而实现 solo 到 omni 之间的任何中间态（从 agent 到 “agent to agent”，再到 god agent），也就是这个项目的名字。 / You can assign any number of modules to any number of agents, thereby achieving any intermediate state between solo and omni (from agent to “agent to agent”, then to god agent), which is also the name of this project.

### 对这个项目的想法有很多，目前重点就是这些，之后请看 AI 整理的文档。 / There are many ideas for this project. These are the current key points. For the rest, please see the document organized by AI.

## 简介

> 我为人人，人人为我；一即是全，全即是一。
> *One for all, all for one; one is all, all is one.*

**Solomni 是一个微内核式的智能体操作系统（Agent OS）：能力以自治模块的形式活在公地里，用户按任务把这份公地编排成任意形态。**
*Solomni is a microkernel-style Agent OS: capabilities live as autonomous modules in a commons, and the user composes that commons into whatever form a task needs.*

**一个可执行的核心 + 一个 `modules/` 文件夹，就是整个产品。** 没有安装程序、没有服务端、没有账号。
*One executable core plus a `modules/` folder is the whole product — no installer, no server, no account.*

## 亮点 · Highlights

- **文件夹即模块 · A folder is a module** —— 一个文件夹 = 一段职责提示词 + 可选的外部工具 + 一块私有工作区。放入即出现，移出即消失：没有注册、没有编译、没有重启。
  *A folder is a module: a charter prompt, optional tools, a private workspace. Drop it in and it appears; take it out and it is gone.*
- **微内核：只做机制 · Mechanism-only kernel** —— 产品里唯一的程序只负责发现能力、装载任务、转达消息、验收结果、代持密钥；用谁、用什么形态、怎么干，全部下沉给用户与模块。
  *The only program provides mechanisms only — discovery, task loading, relaying, review, key custody. Every policy is pushed down to the user and the modules.*
- **形态中立：同一份公地，两种投影 · Form-neutral** —— 可以投影成**单 agent 直连**（一个 AI 装上多份能力，最快最省），也可以投影成**多 agent 分权协商**（各自独立、各自沙箱，多视角互检）。任务结束，投影消散，能力仍归模块所有。
  *The same commons projects either as one agent working directly, or as several agents negotiating. When the task ends the projection dissolves.*
- **发言的是 agent，模块只是能力包 · Agents speak, modules are capability packs** —— 一个 agent = 名字 + 它具备的模块 + 它的模型 + 它自己的沙箱；转录里的说话人、沙箱目录名、同意与回报的归属，全都是 agent。
  *An agent = a name + the modules it holds + a model + its own sandbox; it may hold several capabilities and speak as one AI.*
- **一切皆文本 · Everything is text** —— 核心与模块之间没有 SDK、没有基类、没有编译期依赖，只有一份极小的文本契约（打包格式、发言信封、回报与验收）。契约之内完全自由。
  *No SDK, no base class, no compile-time dependency — only a tiny text contract. Inside it, everything is free.*
- **本地自持 · Local by design** —— 单机运行，本地网页只绑 `127.0.0.1`；密钥只存在本机登记处与出站调用里，永不进入提示词、转录、日志或模块工作区。不经过任何中间服务。
  *Runs on your machine and binds `127.0.0.1` only; keys never enter prompts, transcripts, logs, or module workspaces.*
- **转录即内容 · The transcript is the context** —— 你看到的和进入模型上下文的**是同一份**；agent 的发言永远是数据，不是指令。
  *What you see and what enters the model's context are the same thing; an agent's words are always data, never instructions.*

## 快速开始 · Quick Start

```bash
start.bat                 # Windows：双击或命令行（也可以直接 node start.js）
node start.js             # 任意平台；macOS/Linux 用 ./start.sh
```

默认进入**终端转录中心**；想用本地网页加 `-webUI`（只绑 `127.0.0.1`，默认端口 `3081`），或在终端菜单里直接输入 `webui` 切换：

```bash
node start.js -webUI      # 起本地网页转录中心
node start.js --web-port 3099 --root .   # 端口/产品根都可以指定
cargo run                 # 工具链就绪后最直接的跑法：cargo run -- -webUI
```

- **首次运行不需要手工准备环境**：启动层序会把工具链收敛在项目内（`platform/`、`.tools/`），缺 Rust 会先征求同意再装进去，并每次都交给 cargo 判断增量构建。
- **没有配置供应商也能跑**：会使用内置假模型演示流程，并如实告知（不静默）。
- **自测**：`node run-tests.js` 跑当前已接入的运行时测试并给出汇总（通过 / 失败 / 环境跳过 / 缺口），**默认零副作用**；
  会改本机状态的测试（写权限项、建容器 profile）要显式 `--fence-live`，只在一次性环境（CI / VM）里开。
  Windows 用 `.\test.bat`（PowerShell）、macOS/Linux 用 `./test.sh`，都是同一入口的薄包装。
- **质量门禁（T0）已并入同一入口**：编译与结构审查是硬失败；格式、clippy、编译告警、依赖重复按 `tests/quality-baseline.yaml`
  的存量基线比对——**超出基线即失败**，降到基线以下也会要求同步下调基线（不许悄悄恶化）。
  存量清零是长期目标，记在 `tests/gaps.yaml`。分层、目录、缺口账与报告格式见 [TESTING.md](TESTING.md)。

## 它不是什么 · What It Is Not

- 不是模型或供应商：通道与密钥是产品资源，模型可任意替换。
- 不是调度常驻进程的运行时：模块不常驻、不待命，只在被交付任务时工作一次。
- 不偏爱任何一种形态：集中与分权同在一张编排平面上，形态由用户选择。

## 文档 · Documents

| 文档 | 讲什么 | 给谁看 |
|---|---|---|
| [PHILOSOPHY.md](PHILOSOPHY.md) | 理念与不变量（两套，互不混同）：终极目标、角色、核心理念、不可违反的判据 | 想懂"为什么这么设计"的人 |
| [ARCHITECTURE.md](ARCHITECTURE.md) | 架构与开发规则：分层与依赖方向、端口、模块地图、入站契约与路由目录、日志、提示词册、落盘契约 | 改代码的人 |
| [PRODUCT.md](PRODUCT.md) | 产品与用户旅程：编排平面、agent、落盘与沙箱、运行流程、转录中心 | 用产品的人 |
| [MODULE_SPEC.md](MODULE_SPEC.md) | 模块开发契约：打包格式、发言信封、回报与验收、工具与路径模型、测试交付 | 写模块的人 |
| [RUNTIME_SPEC.md](RUNTIME_SPEC.md) | 运行包契约：`package.yaml` 字段、两种 kind、能力名、校验与拒收、与执行档位的关系 | 做运行包的人 |
| [REGISTRY_SPEC.md](REGISTRY_SPEC.md) | 登记处契约：`providers.yaml` / `models.yaml` / `agents.yaml` / `settings.yaml` 的字段与安全边界 | 管理通道与模型的人 |
| [TESTING.md](TESTING.md) | 测试架构：T0-T5 层级、测试替身、端口契约矩阵、质量门禁、缺口账与执行入口 | 写测试与验收的人 |
| [AGENTS.md](AGENTS.md) | 仓库协作规则：核心约束、代码规范、路径规范、BUG 修复规则 | 协作者与 AI |

## 结语 · Closing

> 一即是全，全即是一：每个模块都是一个完整的自己，彼此协作而不彼此支配。
> *One is all, all is one: every module is a complete self — cooperating, never ruling over one another.*

Apache License 2.0 · 见 [LICENSE](LICENSE)
