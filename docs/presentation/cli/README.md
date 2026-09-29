# cli（终端转录中心）

> argv → 入站能力面 → 渲染事件流。
> 本目录是该单元的唯一细则入口：本页 → [`module-map.md`](module-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：终端转录中心：argv → 入站能力面 → 渲染事件流。`split_names` 与空登记处的引导文案是它自己的传输侧的事。

**不管**：不做业务判断、不持状态、不落盘；**永不接触端口对象，也拿不到 `Core` 本身**。

## 二、入站契约与状态归属

只依赖**入站能力面**（各能力的 `::api`）；`conductor` 的 `Ops` 是它与核心之间的唯一通道。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`conductor`、`registry`、`workspace`
- 谁在用我：（无）

## 四、改动本单元时必须同步

- 入站契约（`Ops` 的方法与事件）一改 → 本处渲染与 `src/tests/cli.rs`。
- 业务缺口账：`src/cli/testgaps.yaml`——业务 AI **只记缺口、不写测试**，由测试 AI 实现测试并销账；格式见 [docs/testing/gaps-acceptance.md](../../../docs/testing/gaps-acceptance.md) §十二。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`module-map.md`](module-map.md) | 逐文件职责（T0 与磁盘双向比对） |
