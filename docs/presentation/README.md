# presentation（前端 / 交付机制）

> 定位：前端交付机制的归类——`cli/`（终端）与 `web/`（本地网页）按渠道各一个目录。
> 效力：规范｜核对：2026-11-04｜依据：`src/presentation.rs`
> 本目录是单元唯一细则入口：本页 → 两个渠道各自的 [`README.md`](cli/README.md) 与 unit-map（逐文件职责，机器比对）。

## 管什么 / 不管什么

- 管：这一格是**归类**——前端按渠道一个目录——`cli/`（终端）与 `web/`（本地网页）。
- 不管：两者**完全分开、互不依赖、没有共享层**（唯一共享的是各能力的 `api`）。
- 不管：业务判断、状态、落盘都不在这里。

## 入站契约与状态归属

- 入站面：只依赖**入站能力面**（各能力的 `::api`）；`conductor` 的 `Ops` 是两个渠道与核心之间的唯一通道。
- 状态归属：没有自己的状态、没有独立的不变式；只做三件事——**传输、路由、纯渲染**。
- 边界：**永不接触端口对象，也拿不到 `Core` 本身**（见 `src/presentation.rs`）。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`conductor`（`Ops`）、`modules` / `registry` / `vm`（后三者经 `Ops` 字段）。
- 谁在用我：（无——流程终点，没有下游业务依赖本层）
- 分层依据：见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一（`cli/` + `web/` 一行）。

## 一、它不是业务能力

- 没有自己的状态、没有独立的不变式；只做三件事：**传输、路由、纯渲染**。
- 只依赖**入站能力面**（各能力的 `::api`）；**永不接触端口对象，也拿不到 `Core` 本身**。

## 改动本单元时必须同步

- 增删路由 → 五处同改：`src/presentation/web/routes.rs` 的 `ROUTES`、前端调用点、[contracts.md](contracts.md) 的 ROUTES 段、`src/tests/routes.rs`（机器比对）与 `demo/*.mjs` 的调用点。
- 入站契约（`Ops` 的方法与事件）一改 → 两个渠道的渲染、`src/tests/api/`、[contracts.md](contracts.md)。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [contracts.md](contracts.md) | 呈现层入站契约、事件台、命令 / 事件规则与机器可读的 HTTP 路由目录 |
| [cli/README.md](cli/README.md) · [cli/unit-map.md](cli/unit-map.md) | 终端转录中心的入口与逐文件职责 |
| [web/README.md](web/README.md) · [web/unit-map.md](web/unit-map.md) | 本地网页的入口与逐文件职责 |
