# presentation（前端 / 交付机制）

> 定位：前端交付机制的归类——`cli/`（终端）与 `web/`（本地网页）按渠道各一个目录。
> 效力：规范｜核对：2026-11-21｜依据：`src/presentation.rs`、`docs/presentation/inbound.yaml`
> 本目录是单元唯一细则入口：本页 → 两个渠道各自的 [`README.md`](cli/README.md) 与 unit-map（逐文件职责，机器比对）。

## 管什么 / 不管什么

- 管：这一格是**归类**——前端按渠道一个目录——`cli/`（终端）与 `web/`（本地网页）。
- 不管：两个渠道**彼此不依赖**；唯一共享的是各能力的 `api` 与面袋 `faces`（只装面，不含判定）。
- 不管：业务判断、状态、落盘都不在这里。

## 入站契约与状态归属

- 入站面：只依赖**入站能力面**（各能力的 `::api`）；本层的 `Faces`（`src/presentation/faces.rs`）由组合根用核心手柄装出，是两个渠道与核心之间的唯一通道。
- 状态归属：没有自己的状态、没有独立的不变式；只做三件事——**传输、路由、纯渲染**。
- 边界：**永不接触端口对象，也拿不到 `Core` 本身**（见 `src/presentation.rs`）。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`runtime`（`RuntimeOps` / `ActionOps` / `LogOps`）、`work`（`WorkOps`）、`turn`（`TurnOps`）、`modules` / `registry` / `vm`（经 `Faces` 字段）。
- 谁在用我：（无——流程终点，没有下游业务依赖本层）
- 分层依据：见 [layers.md](../architecture/layers.md)（`cli/` + `web/` 一行）。

## 一、它不是业务能力

- 没有自己的状态、没有独立的不变式；只做三件事：**传输、路由、纯渲染**。
- 只依赖**入站能力面**（各能力的 `::api`）；**永不接触端口对象，也拿不到 `Core` 本身**。

## 二、每屏入站面清单（inbound.yaml）

- **唯一权威**：[inbound.yaml](inbound.yaml)。每一屏（一个 `.rs`）登记它能取哪几个能力单元（`work`、`catalog/registry` 这种粒度，与 [layers.yaml](../architecture/layers.yaml) 的顶层能力对齐）；**没有登记的屏 = 这一屏不直接取能力**，只吃 `Faces` 的字段。
- **双向比对**：`tests/gates/structure/presentation-inbound.js`（T0 结构审查）把清单与源码对两遍——源码取了清单外的单元报「入站面外溢」，清单登记了源码没取报「清单过期」。要放行新依赖，改清单，不改门禁。
- **同层不许互相依赖**：呈现层内部只能向上取根（`src/presentation.rs`、`faces.rs`、`cli.rs`、`web.rs`）；兄弟文件之间互相 `use` 一律失败（同层有边就是将来成环的入口）。现状是干净星形：两个渠道入口 + 面袋，其它屏都只取根。
- **不替代**依赖方向门禁：[dependency.js](../../tests/gates/structure/dependency.js) 管「只能经 `::api`」，这张表管「能经**哪几个** `::api`」——两层判据各管一件事。
- 回执只报不拦：门禁会把「最宽的三屏」写进回执（`faces.rs` 是面袋，本来就宽；某一屏悄悄变宽就是该拆面的信号）。

## 改动本单元时必须同步

- 增删路由 → 五处同改：`src/presentation/web/routes.rs` 的 `ROUTES`、前端调用点、[contracts.md](contracts.md) 的 ROUTES 段、`src/tests/routes.rs`（机器比对）与 `demo/*.mjs` 的调用点。
- 入站契约（`Faces` 各面的方法与事件）一改 → 两个渠道的渲染、`src/tests/api/`、[contracts.md](contracts.md)。
- 某屏新取一个能力单元（或不再取）→ 同次改 [inbound.yaml](inbound.yaml)，否则 T0 结构审查报外溢 / 过期。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [contracts.md](contracts.md) | 呈现层入站契约、事件台、命令 / 事件规则与机器可读的 HTTP 路由目录 |
| [cli/README.md](cli/README.md) · [cli/unit-map.md](cli/unit-map.md) | 终端转录中心的入口与逐文件职责 |
| [web/README.md](web/README.md) · [web/unit-map.md](web/unit-map.md) | 本地网页的入口与逐文件职责 |
| [unit-map.md](unit-map.md) | 本层根级文件（入口与面袋 `faces`）的逐文件职责 |
| [inbound.yaml](inbound.yaml) | 每屏入站面清单：每个 `.rs` 许取哪几个能力单元（T0 双向比对） |
