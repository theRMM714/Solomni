# sandbox（执行环境策略）

> 定位：会话只写「用哪个执行环境策略 + 不透明参数」，解释权在实现方——本单元是这条契约的家。
> 效力：规范｜核对：2026-11-27｜依据：`src/units/business/sandbox/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：**执行环境声明** `SandboxDecl`（策略 id + 定版 `pins` + 不透明 `params`）与解析出来的**事实** `SandboxFacts`（`policy` / `side` / `guest` / `guest_os` / `guest_arch`）。
- 管：**策略契约** `SandboxPolicy`（自我介绍 `decl`、声明校验 `check`、事实求值 `facts`）与**注册表** `SandboxPolicies`（按 id 取策略、缺省回落本机、未登记如实报错）。
- 不管：**具体环境怎么起**——本机侧隔离在内核围栏（`kernel` / `tools`），虚拟机侧在 `vm`；本单元只认「哪一侧、客机是谁」这些事实（`src/units/business/sandbox/domain/policy.rs`）。
- 不管：**会话怎么选它**（`session_model` 只存声明与事实快照）、**装配怎么用事实派生计划**（`work` / `vm`）、**呈现层怎么列可选策略**（`work` 的 `SandboxChoices`）。

## 入站契约与状态归属

- 入站面：`api`——`SandboxDecl` / `SandboxFacts` / `PolicyDecl` / `SandboxPolicy` / `SandboxPolicies` / `HOST_POLICY` / `VM_POLICY` / `host_policy()`。
- 登记两个策略：`host` 在本单元（`src/units/business/sandbox/domain/host.rs`），`vm` 在 `vm`（`src/units/business/vm/domain/policy.rs`，经 `vm::api::vm_policy` 取实例）；组合根按顺序登记进 `SandboxPolicies`，第一项是缺省。
- 状态归属：会话的**声明与事实**落在 `meta.yaml`（`SessionMeta.sandbox` / `SessionMeta.facts`，由 `work` 在新建与改配时同一次重算）；本单元自身无状态（注册表是装配期构造的只读表）。
- **`side` 复用环 1 的那一份**：`Side`（`host` / `guest`）与派生选型 `TargetSpec` 在 `exec_model`，本单元不另立副本（R15）；`SandboxDecl::target(&facts)` 是声明到选型的唯一换算。
- **参数不透明**：`params` 是策略自己的对象，本单元只校验「是不是给这个策略的」，字段含义在各策略实现里（虚拟机策略只认 `guest`，本机策略没有参数）。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`exec_model`（`Side` / `TargetSpec`）、`registry_model`（`Param` / `ParamType`）。
- 谁在用我：`session_model`（存声明与事实）、`work`（建会话与改配时求事实、派生计划）、`vm`（登记虚拟机策略）、`agentloop` / `turn`（把 `side` 交给一席的沙箱与工具环境）、`presentation`（配置界面列可选策略）。

## 改动本单元时必须同步

- `docs/session/session-model.md` → `meta.yaml` 的 `sandbox` / `facts` 两段口径（声明是唯一真相，事实是快照）。
- `docs/vm/README.md` → 虚拟机策略只是「一种执行环境」：档位词汇改成策略词汇时两边不许各说一半。
- `REGISTRY_SPEC.md` → `settings.yaml` 的 `default_sandbox`（默认执行环境）声明变了要同步。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
