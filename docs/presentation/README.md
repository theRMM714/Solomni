# presentation（前端 / 交付机制）

> 这一格是**归类**：前端按渠道一个目录——`cli/`（终端）与 `web/`（本地网页）。
> 两者**完全分开、互不依赖、没有共享层**（唯一共享的是各能力的 `api`）。

## 它不是业务能力

没有自己的状态、没有独立的不变式；只做三件事：**传输、路由、纯渲染**。
只依赖**入站能力面**（各能力的 `::api`）；**永不接触端口对象，也拿不到 `Core` 本身**。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`contracts.md`](contracts.md) | 呈现层入站契约、事件台、命令/事件规则与机器可读的 HTTP 路由目录 |
| [`cli/README.md`](cli/README.md) · [`cli/module-map.md`](cli/module-map.md) | 终端转录中心的入口与逐文件职责 |
| [`web/README.md`](web/README.md) · [`web/module-map.md`](web/module-map.md) | 本地网页的入口与逐文件职责 |

## 改动时必须同步

- 增删路由 → 五处同改：`src/presentation/web/routes.rs` 的 `ROUTES`、前端调用点、[`contracts.md`](contracts.md) 的 ROUTES 段、`src/tests/routes.rs`（机器比对） 与 `demo/*.mjs` 的调用点。
- 入站契约（`Ops` 的方法与事件）一改 → 两个渠道的渲染、`src/tests/api.rs`、[`contracts.md`](contracts.md)。

详见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一（`cli/` + `web/` 一行）。
