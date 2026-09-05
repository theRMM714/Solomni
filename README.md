# Solomni

> **我为人人，人人为我；一即是全，全即是一。**
> *One for all, all for one; one is all, all is one.*

Solomni 是一个以「去中心化为常态」为原则的跨平台 AI 助手。它的独特之处不在某一个具体的 AI 能力，而在于能力被如何组织：每一个功能都是一个**自治模块**，模块之间不靠一个中心化的「上帝」来指挥，而是通过一个内容无关的**交易所**彼此协作。在这里，中心化不是存在的前提，而只是一种可选的能力。

Solomni is a cross-platform AI assistant organized around a single principle — **decentralization is the default state**. Its distinction lies not in any particular AI capability, but in how capabilities are organized: every feature is an **autonomous module**, and modules do not take orders from a centralized "god" — they cooperate through a content-agnostic **exchange**. Here, centralization is never a precondition for existence; it is only an optional capability.

---

## 终极目标 · The Ultimate Goal

构建一个去中心化为常态的跨平台 AI 助手：

- **模块自治** —— 每个模块可独立运行、独立替换
- **中心化只是能力，不是前提** —— 有中心时值得共享的才被共享，没有中心时各自满足自身需求
- **无上帝** —— 没有任何模块（包括核心）有资格决定什么该被共享

To build a cross-platform AI assistant where decentralization is the norm:

- **Autonomous modules** — each can run and be replaced independently
- **Centralization is a capability, not a precondition** — with a center, only what is worth sharing gets shared; without one, each satisfies its own needs
- **No god** — no module (not even the core) is entitled to decide what should be shared

---

## 核心理念 · Core Ideas

### 1. 模块自治 · Module Autonomy

每个模块是一个自治程序：拥有独立的入口、独立的状态、独立的生命周期。它对世界的全部认知，只是一份声明——「我提供什么、我需要什么」。上线、下线、重连是常态事实，而不是异常事件。没有核心时，模块以自己的方式满足自己的需求。

Every module is an autonomous program with its own entry point, its own state, and its own lifecycle. Its entire understanding of the world is a single declaration — "what I provide, what I need". Coming online, going offline, and reconnecting are ordinary facts, not exceptional events. Without a core, a module satisfies its own needs in its own way.

### 2. 中心化是能力，不是前提 · Optional Centralization

去中心化是默认状态；依赖中心化协调是一种可选能力。类比：同一块网卡，有基站走基站，没基站自组网。没有核心，模块各自实现需求；有核心，值得共享的才被共享。

Decentralization is the default; relying on centralized coordination is an optional capability. By analogy: a single network card uses a base station when one exists, and forms an ad-hoc network when it doesn't. Without a core, modules fulfill needs on their own; with a core, only what is worth sharing gets shared.

### 3. 无上帝：机制与策略分离 · No God — Mechanism, Not Policy

核心只有三个动词，全部是服务，没有一个是权力：**校验、配对、投递**。它拒收非法事实（身份冲突、拓扑成环、接线指向未知），却从不替任何人做选择。决策权永远在边缘：消费方声明首选，部署者显式接线，调用方定向调用。

The core has exactly three verbs, all of them services and none of them power: **validate, pair, deliver**. It rejects illegal facts (identity conflicts, cyclic topology, wiring to the unknown), yet never chooses on anyone's behalf. Decision-making always lives at the edge: consumers declare preferences, deployers wire explicitly, callers direct their own calls.

### 4. 涌现式中心化 · Emergent Centralization

共享 = 提供方的主权声明 + 消费方的策略选择，双边同意才成立。中心化是涌现的结果，不是设计的前提。一个能力同时有多个提供方在线，不是错误状态，而是**消费方的选择空间**。核心如实呈现候选，绝不代为选择；「曾经唯一」只是巧合，不构成任何特权。

Sharing = a provider's sovereign declaration + a consumer's policy choice; only mutual consent makes it real. Centralization is an emergent result, not a designed premise. Multiple providers for one capability is not an error — it is the consumer's space of choice. The core honestly presents candidates and never chooses; "used to be the only one" is mere coincidence, never a privilege.

### 5. 事实驱动配对 · Fact-driven Pairing

配对永远是当前声明的纯函数：声明到达或离开，配对即重算；同 id 重连，即一次离开加一次到达。提供方离线，消费方自动降级；提供方回归，配对自动恢复。事实应随事实的变化而到达——轮询是用重复劳动换取本可直接送达的事实，是一种浪费。

Pairing is always a pure function of current declarations: when a declaration arrives or departs, pairing recomputes; reconnecting under the same id is one departure plus one arrival. A provider going offline degrades consumers automatically; a provider returning restores pairing automatically. Facts should arrive as facts change — polling is wasted effort spent extracting what could simply be delivered.

### 6. 辅助性原则 · Subsidiarity

事务在能胜任的最低层解决；中心只做局部做不了的事。

Matters are resolved at the lowest level capable of handling them; the center does only what a part cannot do alone.

### 7. 状态所有权排他 · Exclusive State Ownership

数据归模块所有，没有共享数据库——这是模块间「无冲突」的根源。跨模块流动的是消息，不是状态。

Data belongs to its module; there is no shared database — this is the root of conflict-freedom between modules. What flows across module boundaries is messages, never state.

---

## 不变量 · Invariants

无论实现如何变化，以下规则不可违反。No matter how the implementation evolves, these rules must never be violated.

1. **协作单路径 · Single collaboration path** —— 模块间协作只经核心，无点对点直连。Modules cooperate only through the core; no peer-to-peer direct links.

2. **消息边界 · Message boundary** —— 边界上只有消息，永不共享对象引用。Only messages cross the boundary; object references are never shared.

3. **薄内建 · Thin built-ins** —— 模块的内建实现有意保持最小，富实现只放共享侧一处。A module's built-in implementation is deliberately minimal; rich implementations live in exactly one place on the shared side.

4. **配对即事实 · Pairing is fact** —— 接线永远是当前声明的纯函数；成员变化即重算并推送给受影响者，禁止轮询。Wiring is always a pure function of current declarations; membership changes recompute and push immediately; polling is forbidden.

5. **无静默选择 · No silent selection** —— 核心只在合法事实间配对、只在非法事实上拒收；多提供方是消费方的选择空间而非错误，缺提供方是降级而非失败。The core pairs only among legal facts and rejects only illegal ones; multiple providers is a space of choice, not an error; a missing provider is degradation, not failure.

6. **可替换 · Replaceable** —— 换任意模块、换任意核心，互不牵连。Swap any module or swap the core itself; nothing else is affected.

7. **降级而非崩溃 · Degrade, don't crash** —— 能力缺失导致功能降级，而不是程序失败。A missing capability causes graceful degradation, never program failure.

---

## 结语 · Closing

> 一即是全，全即是一：每个模块都是一个完整的自己，彼此协作而不彼此支配。
> *One is all, all is one: every module is a complete self — cooperating, never ruling over one another.*

理念的完整抽象定义见 [assistant/PHILOSOPHY.md](assistant/PHILOSOPHY.md)；产品形态与用户旅程见 [assistant/PRODUCT.md](assistant/PRODUCT.md)。
