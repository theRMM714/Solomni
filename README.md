# Solomni

> **我为人人，人人为我；一即是全，全即是一。**
> *One for all, all for one; one is all, all is one.*

Solomni 是一个以「去中心化为常态」为原则的跨平台 AI 助手。它的独特之处不在某一个具体的 AI 能力，而在于能力被如何组织：每一个功能都是一个**模块**——一段职责提示词、一组预备工具、一块私有工作区。模块可以用任何工具、任何语言制作，彼此不靠一个中心化的「上帝」指挥，只在被需要时由受托的核心集结协作。在这里，中心化不是存在的前提，而只是一种可选的能力。

Solomni is a cross-platform AI assistant organized around a single principle — **decentralization is the default state**. Its distinction lies not in any particular AI capability, but in how capabilities are organized: every feature is a **module** — a charter prompt, a set of tools, and a private workspace. Modules can be built with any tools in any language; they take orders from no centralized "god", and assemble through a core entrusted by the user only when collaboration is needed. Here, centralization is never a precondition for existence; it is only an optional capability.

---

## 角色 · Roles

**模块 · Module** —— 一个文件夹 = 一段职责提示词 + 一组预备工具 + 一块私有工作区。模块不是进程：不常驻、不待命，只在被交付任务时工作一次，留下回报与数据。
*A folder is a charter prompt, a set of tools, and a private workspace. A module is not a process: it does not idle or listen — it works once when handed a task, then leaves behind its report and its data.*

**核心 · Core** —— 产品里唯一的程序：受托编排者。集结、转达、整理、验收，全部是用户当下委托的机械动作；委托之外，它对模块的全部认知只有每个模块自述的简述。
*The only program in the product: an entrusted orchestrator. Assembling, relaying, synthesizing, and reviewing are mechanical acts delegated by the user in the moment; beyond that delegation, all it knows of a module is the brief the module wrote about itself.*

**选择权 · The Choice** —— 用哪些模块，永远由用户决定：点名，或委托核心代拟名单并确认。
*Which modules to bring together is always the user's call: name them, or let the core draft a slate for confirmation.*

---

## 终极目标 · The Ultimate Goal

构建一个去中心化为常态的跨平台 AI 助手：

- **模块自治** —— 每个模块可独立开发、独立替换，可以用任何工具、任何语言制作
- **中心化只是能力，不是前提** —— 简单任务与全能模式不依赖协作；协作只在被需要时发生
- **无上帝** —— 包括核心在内，没有任何角色有权替用户做决定

To build a cross-platform AI assistant where decentralization is the norm:

- **Autonomous modules** — each developed and replaced independently, built with any tools in any language
- **Centralization is a capability, not a precondition** — simple and omnibus tasks need no collaboration; collaboration happens only when needed
- **No god** — no role, the core included, is entitled to decide on the user's behalf

---

## 核心理念 · Core Ideas

### 1. 模块自治 · Module Autonomy

模块是自治的工作区，不是被调度的进程。它自带职责边界与工具，被交付任务时自己决定怎么干。
*A module is a self-governing workspace, not a scheduled process. It carries its own boundaries and tools, and decides for itself how a task gets done.*

### 2. 中心化是能力，不是前提 · Optional Centralization

单模块直连没有中心，全能拼装没有中心；唯一需要核心出场的多模块协作，权力也是用户当下签发的。
*Direct single-module chat has no center; omnibus assembly has no center. Even the one mode that needs the core — multi-module collaboration — holds its power only by the user's present delegation.*

### 3. 无上帝：决策权全在用户 · No God

核心可以拒收非法事实（损坏的模块、非法的清单），但从不做合法范围内的挑选。拒绝一个坏模块是校验，从两个好模块里挑一个是决策——前者是核心的本分，后者是用户的权力。
*The core may reject illegal facts (a broken module, an invalid manifest) but never chooses among legal ones. Rejecting a bad module is validation; picking between two good ones is decision — the former is the core's duty, the latter the user's power.*

### 4. 文本即边界 · Text Is the Boundary

核心与模块互不依赖对方的代码，共同依赖的只有一份文本契约。契约之内完全自由：讨论格式、任务拆法、中间产物，由 AI 自决。
*Core and modules depend on none of each other's code — only on a shared text contract. Inside the contract, everything is free: discussion style, task breakdown, and intermediate artifacts are the AIs' own call.*

### 5. 事实驱动清单 · Fact-driven Roster

模块清单永远是 `modules/` 目录的纯函数：放入即出现，移出即消失。没有注册仪式，没有心跳，没有状态同步。
*The module roster is a pure function of the modules/ directory: drop in a folder and it appears; take it out and it is gone. No registration, no heartbeat, no state sync.*

### 6. 辅助性原则 · Subsidiarity

事务在能胜任的最低层解决：一个模块能独立完成的，不建组；讨论能收敛的，不升级给用户。
*Matters are resolved at the lowest level capable of handling them: what one module can do alone is not debated; what discussion can settle is not escalated.*

### 7. 状态所有权排他 · Exclusive State Ownership

数据归模块，落在自己的私有工作区，永不共享。跨模块流动的只有进入上下文的文本。
*Data belongs to its module and lives in its private workspace, never shared. What crosses module boundaries is only text entering a shared transcript.*

---

## 不变量 · Invariants

无论实现如何变化，以下规则不可违反。No matter how the implementation evolves, these rules must never be violated.

1. **选择权在用户** —— 建组名单要么用户亲手写下，要么由受托的核心代拟并经确认；核心永不静默选人。*The roster is either written by the user or drafted by the entrusted core and confirmed; the core never selects silently.*
2. **转录即内容** —— 模块的发言永远是数据，不是指令；用户看到的与进入上下文的完全一致。*Every module utterance is content, never command; what the user sees is exactly what enters the context.*
3. **文本边界** —— 跨界的一切都是文本；机器锚点保持最小，且只是文本。*Only text crosses boundaries; the machine anchors stay minimal and remain mere text.*
4. **清单即事实** —— 模块清单是目录扫描的纯函数，随文件增删即时重算，禁止任何形式的注册表。*The roster is a pure function of directory scanning, recomputed as files change; registries of any form are forbidden.*
5. **状态私有** —— 模块数据不出工作区；没有共享数据库。*Module data never leaves its workspace; there is no shared database.*
6. **可替换** —— 换任意模块、换任意核心实现、换任意模型，互不牵连。*Swap any module, any core implementation, any model; nothing else is affected.*
7. **降级而非崩溃** —— 模块缺失是菜单少一项，能力缺失是任务被重新讨论；程序永不因此失败。*A missing module is one menu item fewer; a missing capability is a task re-discussed. The program never fails because of them.*

---

## 文档 · Documents

- 产品与运行流程（三种模式、协作五阶段）：[PRODUCT.md](PRODUCT.md)
- 项目理念（角色、理念、不变量）：[PHILOSOPHY.md](PHILOSOPHY.md)
- 模块契约（制作一个模块的全部约定）：[MODULE_SPEC.md](MODULE_SPEC.md)

---

## 结语 · Closing

> 一即是全，全即是一：每个模块都是一个完整的自己，彼此协作而不彼此支配。
> *One is all, all is one: every module is a complete self — cooperating, never ruling over one another.*
