<!-- docs-template: spec -->
# 分层与依赖方向
> 定位：依赖方向的唯一权威：四环怎么分、环间与环内的规则（H1/H2）、kernel 机制与入口分段两张副表怎么算。
> 效力：规范｜核对：2026-11-28｜依据：`layers.yaml`、`docs/kernel/mechanisms.yaml`、`tests/gates/structure/layers.js`、`tests/gates/structure/kernel-mechanisms.js`

## 适用与判据
- 改代码前先读本文；归环与依赖方向以本文为准，违规由 T0 结构审查判定（`tests/gates/structure/layers.js`、`kernel-mechanisms.js`、`dependency.js`），零豁免。

## 一、四环
- 对外表述是**四个环 + 适配器**：环 0 `kernel`（地基库）、环 1 纯、环 2 状态与机制、环 3 用例；环 4 是适配器（`presentation/`、`entry/`），不在 `layers.yaml` 里，由 `dependency.js`、`entry-tiers.js` 管。依赖箭头只有一种画法（外层指向内层）：

```text
环 4 适配器（entry/、presentation/）
  └──▶ 环 3 用例 ──▶ 环 2 状态与机制 ──▶ 环 1 纯 ──▶ 环 0 kernel
```

- 同一张图的静态版：[overview.svg](overview.svg)（4 环 + 适配器、每环单元数、跨环引用数；由 archify 从 [overview.json](overview.json) 导出，拓扑钉在 `d9dad33e7` 的快照上，数据同本节与 `Structure.html` 的单元环视图）。改图只改 `overview.json`，再跑 `archify export-svg architecture docs/architecture/overview.json docs/architecture/overview.svg --theme light --quality showcase --repo-root .`；需要可交互版时跑 `archify render architecture docs/architecture/overview.json docs/architecture/overview.html --quality showcase --repo-root .`，交互产物只在本机、不入库（archify 是外部工具，不在本仓依赖内）。细到 33 个单元、136 条边的依赖矩阵与门禁问题清单在结构审查产物 `Structure.html`（`node tests/gates/structure/graph.js` 生成，不入库）。

- **环是架构地位，路径只编码「角色」**。谁在哪个环只由 `layers.yaml` 决定；路径只回答「这一类单元是什么」，三桶各收一类角色：环 1 的纯单元住 `src/units/pure/<单元>/`，盘主单元住 `src/units/resource/<单元>/`，其余业务单元（有副作用、有状态或编排用例）住 `src/units/business/<单元>/`，顶层不再有平铺单元——`business` 同时住环 2 与环 3 的单元，正是「桶不编码环」的证据。三桶各自双向判：`src/units/pure/` 下的目录必须正好是环 1，环 1 的单元也必须住在这里（判据：`tests/gates/structure/layers.js` 的 `shapeProblems`）；`src/units/resource/` 只收盘主，盘面归属由主人表判（判据：`tests/gates/structure/disk-owners.js`，口径见 [io.md](io.md)）；每个单元恰好住一个桶、`src/units/` 下只许三个桶（判据：`tests/gates/structure/unit-buckets.js`）。桶编号角色、不编号环——换环只改一行表，不会引发全量改名，也不会出现「路径与表两个权威」。门禁认单元名一律走 `tests/gates/structure/unitpath.js`（`unitOfPath` / `unitOfModule` / `unitDir` / `BUCKETS`），禁止位次假设。
- **环 1 的判据是「无 IO、无状态、无端口、可穷举测试」**，不是「没有业务语义」。权限判定、任务链推演这类有领域词但全部可穷举的规则就该在环 1；否则它们无家可归，只能被各切片复制一遍（违反 R13）；纯层判据由 `tests/gates/structure/layers.js` 的 `pureProblems` 机器判定。按**概念**切（transcript、session-model、registry-model），不按类型切。
- **环 2 与环 3 的区别是「有没有流程」**：环 2 持有状态或与外界打交道（存储、进程、通道、登记处）；环 3 是一个用例一条流程（work、lifecycle、turn-flow、collab…），流程、事务、补偿留在用例，适配器只做翻译与一行调用。
- **同环有先后，但不是层**：机制栈天然有序（agentloop → tools → workspace；work → vm）。这个先后由同环边算成 rank，只回执、不设预算；门禁只要求同环边指向 rank 更低的单元，即同环无环。
- 单元内部仍按**子业务**垂直切：`api`（入站契约：trait + DTO）/ `service`（状态与用例）/ `ports`（出站端口，只由定义它的子业务持有）/ `domain`（纯逻辑）/ `detail`（只有组合根能构造）；单元自己的 `units/<单元>.rs` 只声明子业务，不写实现。
- **业务之间与子业务之间都只经对方的 `api`**（R1）；跨单元边只许向内（H1），不反向依赖根与呈现层（T0 机器判定，见 [boundaries.md](boundaries.md) 的判据清单）。

## 二、开放架构五条
> 依赖只写一个方向：越往内越稳定、编排越少、越靠近基础设施；单元之间只经对方 `api` 通话，只传值。五条各有一处机器判据，判红即失败。

| 原则 | 依赖准则 | 机器判据 |
| --- | --- | --- |
| **SDP·单边向内** | 依赖边只有一个方向：外层编排、内层稳定；越往内编排越少、越靠近基础设施；同环只按 rank 先后 | `tests/gates/structure/layers.js` |
| **只走 api** | 单元之间的调用只经对方 `api` 的 trait 与值；不摸 `service` / `domain` / `detail`，也不反向依赖根与呈现层 | `tests/gates/structure/dependency.js` |
| **DIP·跨单元** | 需要对方替它做事的点声明成端口，由实现方实现；声明方只经自己的 `api` 交出去 | `tests/gates/structure/dependency.js` |
| **DIP·单元内部** | `service` 只认 `ports` / `api` 的抽象；具体实现只在组合根 `new` | `tests/gates/structure/capability.js` |
| **DTO·只传值** | 跨边界只传不可变值（或无状态端口）；锁、原子、可变借用、闭包这类活对象不出境 | `tests/gates/structure/values.js` |

- 出站依赖由**各单元定义端口**、**各单元的 `detail` 实现**；入站依赖由**各单元定义 `api`**；`Faces`（`src/presentation/faces.rs`）只把它们**装配**给呈现层。
- **单元不定义「前端接口让别人实现」**；呈现层拿到的是 `Faces`（各单元自己的窄 trait 对象 + 事件台 + 根填的值），不是 `Core`，也不是任何锁；`Faces` 只做装配，不得长判定（`src/presentation/faces.rs`）。
- **根不是业务层**：它只做启动、装配、探针与守门；业务用例不得写进 `main.rs` / `entry/`。
- **机制不升格为业务**：日志、路径、进程、围栏、平台差异归一化没有领域词、没有用户可见后果，留在 `kernel` 或所属单元的 `detail/`；调用方多不等于业务（判据见 [boundaries.md](boundaries.md)）。
- 「用哪个供应商 / 模型」是**策略**（`registry` 的解析链决定）；「怎么建通道」是**机制**（llm 的 `detail`）。两者不互换。

## 三、环表：声明的是架构地位，不是依赖深度的缓存
- 环表（唯一权威是 `layers.yaml`；下面这张由它生成，文档里不手写环号）：
<!-- BEGIN:layer-table -->
| 环 | 单元 | 可依赖 |
| --- | --- | --- |
| 0（地基） | `kernel` | 无 |
| 1（纯） | `exec_model`、`permission`、`registry_model`、`taskchain` | 0 |
| 2（状态与机制） | `agentloop`、`assets`、`catalog`、`content`、`home`、`llm`、`prompt`、`provision`、`residents`、`sandbox`、`session_model`、`sessions`、`tools`、`transcript`、`turn_state`、`versions`、`vm`、`vmstore`、`workbench`、`workspace` | 0–1 |
| 3（用例） | `collab`、`conversation`、`lifecycle`、`maintenance`、`proxy`、`runtime`、`system`、`turn`、`work` | 0–2 |

> 4 环、33 个单元 + `kernel`：本表由 `layers.yaml` 生成（`node tests/docs-sync.mjs --write`），不手写；环 4（`presentation/`、`entry/`）是适配器，不入表。
<!-- END:layer-table -->

- **H1（硬判）**：跨单元边只能指向严格更低的环；同环只允许指向 rank 更低的单元（rank 由同环边现算，只回执）。向上依赖与同环逆序都判红（`tests/gates/structure/layers.js`）。
- **H2（硬判）**：环 1 的单元不得出现 `std::fs/net/process/env/thread`、`tokio`、`reqwest`、`sqlx`，不得有 `ports/` 目录，不得有可变全局（`static mut`、`Mutex`、`RwLock`、`OnceLock`、`lazy_static`）。对 `kernel` 只准依赖**纯契约值**（`crate::kernel::contracts` 与 `crate::kernel::api::{types,note}`），禁 `kernel::ports`、`kernel::detail` 与函数式访问器：内核是环 0 的共享内核，`Note`/`Severity` 这类出站词汇若不许环 1 用，就会被各切片复制一份（违反 R15）；反过来 kernel 也不能依赖环 1。判据 `tests/gates/structure/layers.js` 的 `pureProblems`。
- **H4（硬判）**：一个名字只属于一个契约面。`src/kernel/contracts/*.rs` 与 `src/units/[桶/]<单元>/api*.rs` 里定义的 pub struct/enum/type，不得在两个契约面上各出现一次；呈现层与入口也不得再造同名类型（那是复制 DTO，不是防腐）。同义模型与翻译只准出现在三处：`src/units/<单元>/detail`、呈现层的屏幕装配、`src/entry`。判据 `tests/gates/structure/values.js` 的 `contractProblems`。
- **形状（硬判）**：`kernel` 固定环 0；环 1–3 都不许空；盘上每个单元都要归环，表里的单元都要有目录。
- **改环 = 改架构地位**：写进 PR 说明理由即可，没有 `--write`，也不存在「声明不等于重算就失败」——环是人拍的架构决定，不按依赖深度自动推导。
- 只回执不判红的三类：同环 rank 序、纯转发边（只 `pub use` 再导出，不算真依赖）、扇出（不设预算，数字说明这个单元在替多少个单元装配）。
- **同环互用只有三种正当处置**：合并成一个单元、把被引的形状下沉到环 1、端口反转（内层声明、外层实现）。

## 四、单元与子业务的分工
- 单元是**归环与门禁的单位**（`layers.yaml` 一行一个，rank 与扇出按它算，`src/units/<桶>/<单元>/` 与 `src/units/<桶>/<单元>.rs` 成对出现；环 1 必须住 `pure`、盘主必须住 `resource`、其余住 `business`，见上文与 [io.md](io.md)）。
- 子业务是**业务功能的垂直切分**：`<单元>/<子业务>/`、`docs/<子业务>/`、`src/tests/<子业务>.rs` 都按它走（R10）。
- 合并单元只搬目录、不改子业务内部的分区与 `super::` 语义；单元不写实现，只声明子业务。
- **状态不参与归环**：环只回答「能依赖谁」，状态归谁见 [state.md](state.md)。

## 五、kernel 内部的机制分层（第二张表）
- 地基库内部也是依赖图，不是一坨平的文件：内部按**机制**切，一个机制管一种机制——`contracts/<名>`（共享形状）、`domain/<名>`（纯逻辑）、`detail/<名>`（实现）、`ports`（全项目共享的出站面）。机制对机制**只许严格向下**，同层互依赖一律判红（`docs/kernel/mechanisms.yaml`、`tests/gates/structure/kernel-mechanisms.js`）。
- 两张表粒度不同，别混：`layers.yaml` 一行一个**单元**（判据是有自己的业务用例 + ≥2 调用方）；`mechanisms.yaml` 一行一个**机制**（没有业务词、没有用户可见后果）。机制不因调用方多就升格成单元（判据见 [boundaries.md](boundaries.md)）。
- 判据五条与单元环同口径，但环要显式报出：① 盘上每个机制都要在表里归层，表里的机制也要在盘上；② 依赖边只许从高层指向低层；③ 声明层号 == 按现有依赖重算的层号；④ 机制之间不许成环——成环时只报环路径，不拿层号糊过去；⑤ 内核内部只走机制路径，`src/kernel` 下（除 `api/` 面自身）出现 `kernel::api` 判红（面是给外面的，绕道面会让机制边隐形）。
- 机制表与单元环的一处刻意差别：机制层号是**事实缓存**，动过一条边就 `node tests/gates/structure/kernel-mechanisms.js --write` 重算全表；单元环号是**架构决定**，只许人改。
- 跨机制的常设例外只有 `ports`（R12）：全项目共享的出站端口面，由各机制实现，不参与「谁依赖谁」的业务推导；它自己只依赖共享形状。
- 机制表只说机制与层号，不替代逐文件职责：[unit-map.md](../kernel/unit-map.md) 仍由 `tests/gates/structure/map.js` 双向比对。

## 六、入口内部的段（第三张表）
- 入口也按依赖排段，不止「最外层、什么都不许依赖我」这一条：`docs/entry/layers.yaml` 一行一个**模块**（`src/entry` 下的文件名/目录名，编排根单列成 `main`；`src/entry.rs` 只放 mod 声明，不参与分层），四段——值层（`root` / `toolchains` / `probes`）→ 判定层（`startup` / `portable`）→ 装配层（`assembly` / `web` / `args`）→ 编排层（`main`）（`tests/gates/structure/entry-tiers.js`）。
- 规则与上面两张表同一条：**同段不许互相依赖，只能向下**，因此不可能成环（成环也显式报环路径）。另外声明段号必须压住它的全部依赖——两条都由 `tests/gates/structure/entry-tiers.js` 判，权威表 `docs/entry/layers.yaml`。
- 与前两张表的一个刻意差别：段号表达**启动阶段职责**，不是按依赖重算的最小层。`startup` 在入口内没有依赖，声明第 1 段是为了让它排在值层之后、装配之前；这类「比依赖深度松」的声明不算错（只有压不住依赖才失败）。

## 检查
- 依赖方向 → T0 结构审查的 apiOnly / foreignImpl / coreCycles / reverse / presentation / layers / internal 判据（`tests/gates/structure/dependency.js`、`tests/gates/structure/capability.js`、`tests/gates/structure/layers.js`；清单 `node run-tests.js --list`）。
- 呈现层「每屏许取哪几个单元的 ::api」→ `structure/presentation-inbound`（`tests/gates/structure/presentation-inbound.js`，权威清单 `docs/presentation/inbound.yaml`）：清单外的取用报外溢、清单里没用到的报过期，呈现层内部只许取根、兄弟文件不得互相引用。
- 环方向、同环 rank、纯层 H2、环表形状与目录全覆盖 → `node tests/gates/structure/layers.js`。
- 契约唯一家与适配层镜像 DTO（H4）→ `node tests/gates/structure/values.js`。
- kernel 机制：登记双向对账、同层互依赖、声明层号 == 重算层号、机制成环、内核内部绕道 `api` 面 → `node tests/gates/structure/kernel-mechanisms.js`。
- 入口分段：登记双向对账、同段互依赖、向上依赖、压不住依赖、成环、`src/entry.rs` 只放 mod 声明 → `node tests/gates/structure/entry-tiers.js`。
- 结构图与环表渲染 → `node tests/gates/structure/graph.js`：每次跑门禁把四张登记表画成仓库根 `Structure.html`（全景 + 单元环 / kernel 机制 / 入口 / 屏面四视图；点节点下钻到文件、点文件看四槽，左上「返回」或 Esc 逐级回退；永远 pass 只回执），以及 `node tests/docs-sync.mjs`。

## 路由
- 业务边界判据、硬要求 R1–R20 与 T0 判据清单：[boundaries.md](boundaries.md)
- 端口画在哪：[ports.md](ports.md)
- 状态与落盘契约、日志、提示词册：[state.md](state.md)
- 注释契约：[comments.md](comments.md)
- Rust 源码布局、格式、静态检查与跨平台机制：[rust-dev.md](rust-dev.md)
- 单元清单与职责：[docs/README.md](../README.md) 的单元表
