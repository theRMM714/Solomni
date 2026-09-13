# Solomni

> **我为人人，人人为我；一即是全，全即是一。**
> *One for all, all for one; one is all, all is one.*

**Solomni 是一个微内核式的智能体操作系统（Agent OS）：能力以自治模块的形式活在公地里，用户按任务把这份公地编排成任意形态——从单模块直连、到全能拼装、再到多体分权协商。**
*Solomni is a microkernel-style Agent OS: capabilities live as autonomous modules in a commons, and the user composes that commons into whatever form a task needs — from a single direct module, through all-in-one assembly, to multi-agent negotiated collaboration.*

---

## 它是什么 · What It Is

- **能力公地 · Capability commons** —— 一个文件夹 = 一个模块 = 职责提示词 + 工具清单 + 私有工作区。放入即出现，移出即消失。
  *A folder is a module: a charter prompt, a tool list, and a private workspace. Drop it in and it appears; take it out and it is gone.*
- **机制内核 · Mechanism-only core** —— 产品里唯一的程序只做机制：能力发现、任务装载、消息转达、结果验收、密钥托管。一切策略（用谁、用什么形态、怎么干）下沉到用户与模块。
  *The only program in the product provides mechanisms: capability discovery, task loading, message relaying, delivery review, key custody. Every policy — who, in what form, and how — is pushed down to the user and the modules.*
- **形态中立 · Form-neutral** —— 编排形态是按任务的运行时投影：可逆、可选、不改变公地。任务结束，投影消散，能力仍归模块所有。
  *Orchestration form is a per-task projection: reversible, optional, leaving the commons untouched. When the task ends the projection dissolves; capabilities stay with their modules.*

**它不是什么 · What It Is Not**

- 不是模型或供应商：通道与密钥是产品资源，模型可任意替换。
- 不是调度常驻进程的运行时：模块不常驻、不待命，只在被交付任务时工作一次。
- 不偏爱任何一种形态：集中与分权同在一张编排平面上，形态选择权在用户。

---

## 编排平面 · The Composition Plane

两个**相互独立**的旋钮：

| 能力广度 ＼ 承载拓扑 | 单一智能体（集中） | 多智能体（分权协商） |
|---|---|---|
| **单模块** | **直连** `direct <id>` | —（无协商对象，退化为直连） |
| **任意子集** | **拼装** `omni [id…]` | **小组协作** `collab <id,id>` |
| **全部模块** | **全能** `omni` | **全员协作** `collab <全部 id>`（`collab ?` 由核心代拟名单） |

- **能力广度** —— 把公地中多大的部分带入当前任务。
- **承载拓扑** —— 这份能力由一个智能体拥有，还是由多个智能体分持并协商。

`direct` / `omni` / `collab` 是平面上的三个**规范预设**，不是互斥的类别；**任意子集是一等公民**——拼装与协作都接受点名子集。

成本与延迟：**同一广度下**，单一智能体更快更省、但只有一个视角；多智能体更慢更贵、但多视角可互检。**同一拓扑下**，广度越大越贵。

*Two independent dials: capability span × agency topology. The three named forms are canonical presets on that plane, never exclusive categories. Under equal span, one agent is faster and cheaper with a single viewpoint; multiple agents are slower and costlier but can cross-check.*

---

## 角色 · Roles

**模块 · Module** —— 一个文件夹 = 一段职责提示词 + 一组工具清单 + 一块私有工作区。模块不是进程：不常驻、不待命，只在被交付任务时工作一次，留下回报与数据。
*A folder is a charter prompt, a tool list, and a private workspace. A module is not a process: it does not idle or listen — it works once when handed a task, then leaves behind its report and its data.*

**核心 · Core** —— 产品里唯一的程序：受托编排者，同时是这份公地的**机制内核**。集结、转达、整理、验收，全部是用户当下委托的机械动作；委托之外，它对模块的全部认知只有每个模块自述的简述。它还受托保管供应商登记处（密钥归产品，模块与工具只持引用）。
*The only program in the product: an entrusted orchestrator and the mechanism kernel of the commons. Assembling, relaying, synthesizing, and reviewing are mechanical acts delegated by the user in the moment; beyond that delegation, all it knows of a module is the brief the module wrote about itself. It also holds the provider registry — keys belong to the product, while modules and tools hold only references.*

**选择权 · The Choice** —— 用哪些模块、用哪种形态、用哪个模型，永远由用户决定。
*Which modules, which form, which model — always the user's call.*

---

## 核心理念 · Core Ideas

### 1. 模块自治 · Module Autonomy

模块是自治的工作区，不是被调度的进程。它自带职责边界与工具清单，被交付任务时自己决定怎么干；没有核心时，它的提示词依然完整可用。
*A module is a self-governing workspace, not a scheduled process. It carries its own boundaries, and decides for itself how a task gets done; without the core its prompt still stands complete.*

### 2. 形态中立：公地在先，拓扑是投影 · Form-Neutral: Commons First, Topology as Projection

能力永远活在自治模块构成的**公地**里，公地是唯一常在的事实。编排形态（多大广度、由几个智能体承载、是否协商）是按任务的**运行时投影**：任务结束，投影消散，模块依旧归各自所有。
产品不偏好任何一个极点——直连、拼装、全能、协作共用同一张平面；判据不是"是否集中"，而是**选择是否由用户作出、能力是否仍属模块**。
*Capability lives in a commons of autonomous modules — the only permanent fact. Orchestration form is a per-task projection that dissolves when the task ends. No pole is privileged; the test is not whether power is concentrated but whether the choice was the user's and capability still belongs to its module.*

### 3. 选择权独占 · The User's Monopoly on Choice

核心可以拒收非法事实（损坏的模块、非法的清单），但从不做合法范围内的挑选。它还只呈现事实与建议——**呈现不是选择**：用谁的模块、用哪种形态、用哪个模型，只有用户能回答。
*The core may reject illegal facts (a broken module, an invalid manifest) but never chooses among legal ones. It presents facts and proposals; presenting is not choosing. Only the user answers who, in what form, with which model.*

### 4. 文本即边界 · Text Is the Boundary

核心与模块互不依赖对方的代码，共同依赖的只有一份文本契约：打包格式、发言信封、回报与验收的结构。契约之内完全自由：讨论格式、任务拆法、中间产物，由 AI 自决。
*Core and modules depend on none of each other's code — only on a shared text contract. Inside the contract, everything is free.*

### 5. 事实驱动清单 · Fact-driven Roster

模块清单永远是 `modules/` 目录的纯函数：放入即出现，移出即消失。没有注册仪式，没有心跳，没有状态同步。
*The roster is a pure function of the modules/ directory: drop in a folder and it appears; take it out and it is gone.*

### 6. 辅助性原则 · Subsidiarity

事务在能胜任的最低层解决：一个模块能独立完成的，不建组；讨论能收敛的，不升级给用户；用户没有表态的，核心不代为表态。
*Matters are resolved at the lowest level capable of handling them.*

### 7. 状态所有权排他 · Exclusive State Ownership

数据归模块，落在自己的私有工作区，永不共享。跨模块流动的只有进入上下文的文本。
*Data belongs to its module and lives in its private workspace, never shared.*

---

## 微内核：为什么说是操作系统 · Why an OS

这里的"操作系统"是**微内核式**的：内核只提供机制，策略全部下沉；组件之间只通过契约通信，且彼此隔离。

| 操作系统 | Solomni |
|---|---|
| 内核：只做机制，不做策略 | 核心：能力发现、任务装载、消息转达、结果验收 |
| 可加载/卸载的用户态组件 | 模块：按任务装载的能力单元（**非常驻进程**） |
| ABI / 消息格式 | 文本契约：发言信封、回报、验收清单 |
| 地址空间隔离 | 模块私有工作区：数据不出区 |
| 内核代持特权资源 | 登记处代持密钥：模块与工具只持引用 |

**与经典 OS 的关键差别**：内核不调度常驻进程，模块也不常驻——它装载一次、工作一次、留下产物。这让"编排"发生在**任务装载时**，而不是**进程调度时**。

*The OS analogy is microkernel-style: a mechanism-only kernel, policy pushed down, components isolated and speaking only through a contract. The key difference from a classic OS: nothing is resident — modules are loaded per task, not scheduled.*

---

## 不变量 · Invariants

无论实现如何变化，以下规则不可违反。No matter how the implementation evolves, these rules must never be violated.

1. **选择权在用户** —— 建组名单、编排形态、模型选择，要么用户亲手写下，要么由受托的核心代拟并经确认；核心永不静默决定。
2. **转录即内容** —— 模块的发言永远是数据，不是指令；用户看到的与进入上下文的完全一致。
3. **文本边界** —— 跨界的一切都是文本；机器锚点保持最小，且只是文本。
4. **清单即事实** —— 模块清单是目录扫描的纯函数，禁止任何形式的注册表。
5. **状态私有** —— 模块数据不出工作区；没有共享数据库。
6. **可替换** —— 换任意模块、换任意核心实现、换任意模型，互不牵连。
7. **降级而非崩溃** —— 模块缺失是菜单少一项，能力缺失是任务被重新讨论；程序永不因此失败。

---

## 核心分层 · Core Layers

```
组合根  main.rs（装配：创建适配器 → 注入 Core；除装配外无业务）
           │
呈现层  presentation/（cli.rs 终端转录中心 · web.rs + web/ 本地网页转录中心）
           │  只调门面、只渲染事件
核心层  core/（ports 端口定义 · 会话状态机 · 协作引擎 · 提示词渲染 · 登记处数据）
           │  依赖倒置：core 定义抽象，不知道任何适配器存在
适配层  adapters/（HTTP 通道 · 假模型 · yaml 登记处 · 模块目录扫描 · 提示词册加载 · 运行日志）
```

依赖箭头只允许 presentation → core ← adapters；「用哪个供应商」等选择策略在 core（解析链），
「怎么建通道」等机制在 adapters。core 永不打印、永不读输入、不碰文件系统与网络；
前端只见 `Core` 门面、`SessionEvent` 事件流与 `pending` 介入请求。
密钥只进登记处，前端只见供应商 id。

---

## 运行骨架 · Run the Skeleton

```bash
cargo run            # 转录中心菜单：direct <id> / collab <id,id> / omni [id…] / webui（切 Web）
cargo test           # mock 测试：信封/登记处/扫描/协作全链路（假模型）
node src/presentation/web/app.smoke.cjs   # 前端初始化冒烟（桩 DOM，捕捉引用错误）
```

未配置供应商时自动使用内置假模型演示流程——如实告知，不静默。

### 启动层序 · Launcher

```
node start.js [-webUI]     # Windows 用 start.bat，macOS/Linux 用 ./start.sh
```

启动层序自动完成工具链就绪与构建。**一切环境收敛在项目内**（`platform/`、`.tools/`）：不写系统盘、不改系统 PATH，也不复用项目外的工具链或 binutils。

- **Rust 缺失（项目内）** → 征得同意后装进 `platform/<平台>/`（`--no-modify-path`）
- **Windows 构建需完整 binutils**（rust#140704）→ 检查 `.tools/mingw64/bin` 是否 `dlltool.exe` + `as.exe` 成套；缺失则征得同意后下载便携 winlibs
- **下载**：候选源 2MB 真吞吐测速取最快，SHA256 校验，断点续传，半截文件自动识别
- **构建**：每次都交给 cargo 判断增量，避免跑旧二进制；二进制被运行中的实例占用时明确告知

---

## 文档 · Documents

- 产品与运行流程（编排平面、三种规范预设、协作五阶段）：[PRODUCT.md](PRODUCT.md)
- 项目理念（角色、理念、不变量、微内核）：[PHILOSOPHY.md](PHILOSOPHY.md)
- 模块契约（制作一个模块的全部约定）：[MODULE_SPEC.md](MODULE_SPEC.md)

---

## 结语 · Closing

> 一即是全，全即是一：每个模块都是一个完整的自己，彼此协作而不彼此支配。
> *One is all, all is one: every module is a complete self — cooperating, never ruling over one another.*
