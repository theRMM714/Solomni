# registry（登记处）

> 供应商 / 模型 / agent / 设置四份 yaml 与「模型 → 通道」解析链。
> 本目录是该单元的唯一细则入口：本页 → [`module-map.md`](module-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：四份 yaml 的内存形态与用例、解析链、agent 登记处与名字校验、代拟名单落地、密钥回显边界。

**不管**：不建通道（`llm` 的 `detail`）、不替用户选模型（用户选，登记处只解析）、不碰会话。

## 二、入站契约与状态归属

`api::RegistryOps`（呈现层队列面）+ `api::Registry`（能力面：读 `&self`、写 `&mut self`——状态住在协调业务的执行线程上，靠单线程命令队列互斥，**不额外上锁**）+ 登记处词汇与视图。出站端口 `SettingsStore` **只由 `service.rs` 持有**。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`llm`、`prompt`、`session`、`workspace`
- 谁在用我：`cli`、`collab`、`conductor`、`slate`、`web`

## 四、改动本单元时必须同步

- 字段一改 → `REGISTRY_SPEC.md`（唯一权威）、`src/tests/registry.rs`、`web` 的设置页；密钥永不进入提示词 / 转录 / 日志 / 模块工作区。
- 业务缺口账：`src/capabilities/registry/testgaps.yaml`——业务 AI **只记缺口、不写测试**，由测试 AI 实现测试并销账；格式见 [docs/testing/gaps-acceptance.md](../../docs/testing/gaps-acceptance.md) §十二。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`module-map.md`](module-map.md) | 逐文件职责（T0 与磁盘双向比对） |
