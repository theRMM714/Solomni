# 测试架构（TESTING）

> 本文是测试的唯一权威：定义测试层级、测试替身、端口契约、质量门禁、缺口账、执行入口与验收判据。
> 架构约束见 [ARCHITECTURE.md](ARCHITECTURE.md)，模块交付要求见 [MODULE_SPEC.md](MODULE_SPEC.md)，仓库协作约束见 [AGENTS.md](AGENTS.md)。
>
> 本文只描述**当前状态**：已实现的写清楚，没实现的进缺口账。缺口的唯一真相是 `tests/gaps.yaml`（全局）与
> `tests/<平台>/gaps.yaml`（平台）——本文件不复述条目内容，只规定格式与判定规则。

## 一、测试的目的与硬原则

测试要回答的是"当前实现是否满足可观察的行为契约"，不是"测试数量是否很多"或"代码覆盖率是否好看"。

- 测试记录事实：通过、失败、环境跳过、缺口必须分开。
- 环境不允许执行不等于通过；必须记录 `env-skip` 与原因。
- 尚未实现测试不等于通过；必须进入缺口账。
- 失败必须有可定位证据：断言、输入、观察结果和必要的日志尾部。
- 测试必须可重复、可隔离、可清理，不依赖执行顺序。
- 默认测试不得修改真实用户数据、真实 `.home/`、真实 `session/`、真实权限或外部服务。
- 真实网络、真实密钥、开发者个人配置不能成为默认测试依赖。
- 跨平台测试必须优先使用路径组件、隔离根和平台能力探针，不把某个平台的行为猜测成所有平台的行为。
- 测试替身也属于代码：Fake、Mock、Stub、Spy 和 Fixture 必须有自己的最小验证。
- 质量门禁和业务测试是两类不同事实；质量门禁失败不能被业务测试通过抵消。
- 不为了测试制造无意义的 trait、包装层、公共状态或重复测试；可测性必须服从架构边界。

## 二、当前状态

当前仓库已经具备：

- `src/tests.rs` 中的全内存核心测试装配；
- `tests/cross-platform/` 跨平台集成与端到端测试；
- `tests/windows/`、`tests/linux/`、`tests/macos/` 平台探针（`tests/helpers/probe.rs` 提供共用探针设施）；
- `src/presentation/web/*.smoke.cjs` 前端冒烟测试；
- `tests/gaps.yaml` 与 `tests/<平台>/gaps.yaml` 缺口账；
- `node run-tests.js` 测试汇总入口（`node start.js -test` 是备好环境后的同一入口）；
- `src/adapters/fake_chat.rs` 中的 `FakeChat` 与 `DemoGateway`；
- `src/tests.rs` 中的 `InMemory*`、`FakeCatalog`、`VecSource`、`ScriptGateway`、`RecordingRunner`、`RecordingFence`、`TestPrompts`、`NoopLog` 等测试装配。

当前平台缺口账（`tests/cross-platform/gaps.yaml`、`tests/<平台>/gaps.yaml`）**为空**：三平台围栏机制与整仓测试
已由三平台 CI 真跑通过（Windows AppContainer + 目录 ACL 授权与撤权、Linux Landlock、macOS seatbelt）。

仍未完成的缺口全部记在 `tests/gaps.yaml`（T0 质量入口、FakeChat / DemoGateway 独立契约测试、端口契约矩阵、冗余门禁）。
条目存在 = 尚未完成；补齐后删除条目，不保留完成历史。

## 三、测试分类（T0-T5）

### T0：静态质量、结构和冗余检查

T0 不验证业务运行结果，而验证代码和测试系统自身是否保持健康。

检查项：

```text
cargo fmt --check
cargo check --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo tree --duplicates
```

同时需要检查：

- `Cargo.toml` 是否登记所有应运行的集成测试目标；
- 新增测试文件是否确实被入口执行；
- 测试标记是否存在、稳定且没有被重复伪造；
- 报告 JSON 是否符合约定；
- `gaps.yaml` 是否可解析、字段完整；
- 测试是否写入项目外绝对路径或真实用户目录；
- 测试结束后是否遗留子进程、端口、临时目录、权限或句柄；
- 是否存在重复测试、重复 Fixture、重复测试替身或无理由的跨层重复断言。

`cargo tree --duplicates` 只检查依赖树中的重复版本，不等于源码重复检查。`clippy` 也不能替代业务测试。三者的职责必须分开记录。

当前状态：上述命令可人工执行，但尚未全部纳入 `node run-tests.js` 的统一报告，属于全局缺口（`tests/gaps.yaml`）。

### T1：单元测试

位置：实现模块内部的 `#[cfg(test)] mod tests`，以及现有的 `src/tests.rs`。

验证：

- 纯函数和状态转换；
- 信封解析、协作状态派生、提示词渲染、路径寻址；
- 单个实现的边界与错误行为；
- 使用内存测试替身装配的 core 行为。

硬规矩：

- 不碰真实网络、真实用户文件、特权权限或外部服务；
- IO、网络、进程、时间、随机数等可替换点必须注入替身；
- 每个重要失败分支必须有明确断言；
- 不用"返回非空""没有 panic"代替对行为的断言；
- 需要观察交互时，断言记录型 Spy/Fake 的可观察记录，不暴露生产实现内部状态。

### T2：端口与适配器契约测试

T2 是当前设计中需要补强的层次。它验证同一个端口的不同实现是否遵守同一份契约。

每个端口至少需要明确：

1. 输入和输出；
2. 成功行为；
3. 错误传播；
4. 空结果和边界输入；
5. 取消与超时（如果端口支持）；
6. 重复调用和幂等性（如果端口有此语义）；
7. 资源清理；
8. Fake/Stub/Spy 的注入方式；
9. 真实适配器的验证方式。

T2 不替代：

- core 纯逻辑单元测试；
- 真实文件系统、真实进程或真实权限的跨平台集成测试；
- 平台专属围栏探针。

### T3：跨平台集成测试

位置：`tests/cross-platform/`。

验证所有目标平台都应成立的真实边界：

- 真实进程协议；
- 隔离文件系统和工作区；
- 本地环回 HTTP；
- 配置和历史落盘、恢复与损坏处理；
- 工具 stdin/stdout、退出码和错误传播；
- 进程树回收、超时和取消；
- 日志、报告和路径编码；
- 前端冒烟（由 `src/presentation/web/smoke.cjs` 自动发现同目录 `*.smoke.cjs`）。

禁止依赖：外网、真实密钥、用户目录、已有后台服务、平台特权或本机偶然配置。

### T4：平台探针

位置：`tests/windows/`、`tests/linux/`、`tests/macos/`。

只验证平台机制本身，例如：

- 文件系统围栏；
- 断网；
- 进程树和杀树；
- ACL、容器 profile、Landlock、seatbelt；
- 平台解释器或系统能力。

每条探针必须区分：

- 代码失败：`test-fail`；
- 环境不允许：`env-skip`，必须有事实原因；
- 尚未实现：`gap`；
- 机制不存在：记录能力事实，不伪装成通过。

探针不得用"本机无法运行"吞掉实现错误。探针具体缺口继续记录在 `tests/<平台>/gaps.yaml`。

### T5：端到端测试

位置：`tests/cross-platform/e2e/`。

使用本地假供应商、隔离根和可重复夹具，验证完整用户旅程：

- 单 agent 对话；
- 多 agent 协作与回报；
- 代拟、回档和会话编辑；
- 文件读写、搜索和工作区隔离；
- 工具正常、失败、超时、拒绝和异常退出；
- 模型通道失败、回落和非法响应；
- 历史恢复、损坏和重复启动；
- 删除会话后的权限回收和资源清理。

T5 验证的是跨模块行为，不把所有内部函数重复断言一遍。

## 四、测试替身规范

**本节是替身语义的唯一权威**；[ARCHITECTURE.md](ARCHITECTURE.md) 只规定"端口必须可注入"，[MODULE_SPEC.md](MODULE_SPEC.md) 只规定"模块作者要交付什么"。

### 4.1 Stub

Stub 只提供预设输入或结果，不负责验证交互。例如固定的设置、能力报告或时间来源。测试需要验证调用次数或顺序时，不能只用 Stub。

### 4.2 Fake

Fake 是可运行但简化的端口实现。它应当让 core 在没有真实网络、文件系统或外部服务时运行真实业务流程。

Fake 必须：

- 实现明确的 core 端口；
- 支持成功、失败、空结果和边界输入；
- 在端口有此语义时支持延迟、取消、超时或断开；
- 记录被测代码需要观察的调用现场；
- 通过最小端口契约测试；
- 不得只有"永远成功"的 happy path；
- 不得偷偷改变生产端口的错误、顺序或资源语义。

当前项目中的 Fake 或 Fake 候选：

| 实现 | 当前角色 | 当前状态 |
| --- | --- | --- |
| `src/adapters/fake_chat.rs:FakeChat` | 脚本模型，同时记录 `calls`，兼具 Fake + Spy | 已被业务测试使用；独立契约测试缺口 |
| `src/adapters/fake_chat.rs:DemoGateway` | 演示/回落网关 | 已被生产装配使用；独立契约测试缺口 |
| `src/tests.rs:InMemorySettings`、`InMemoryHistory`、`InMemoryWorkspace`、`InMemorySysIo` | 内存 Fake | 已被核心测试装配使用；需按端口补最小契约覆盖 |
| `src/tests.rs:InMemoryPackages` | 包库 Fake | 已被核心测试使用；契约矩阵尚未完整登记 |
| `src/tests.rs:FakeCatalog` | 模型目录 Fake + 调用记录（`seen`） | 已被核心测试使用；契约矩阵尚未完整登记 |
| `src/tests.rs:VecSource` | 模块清单 Fake | 已被核心测试使用；契约矩阵尚未完整登记 |
| `src/tests.rs:ScriptGateway`、`SharedScript` | 脚本网关 Fake | 已被核心测试使用；失败/取消场景需单独核对 |
| `src/tests.rs:TestPrompts` | 提示词册 Fake（返回内存册子） | 已被核心测试使用；契约矩阵尚未完整登记 |
| `src/tests.rs:RecordingRunner` | 工具执行 Fake + 记录 `calls` | 已被核心测试使用；失败/超时/取消场景需单独核对 |
| `src/tests.rs:SilentRunner` | 守护 Stub：任何调用即 panic | 用于"不该用工具"的路径 |
| `src/tests.rs:NoFenceHost` | 围栏释放空操作 Stub | 已被核心测试使用 |
| `src/tests.rs:RecordingFence` | 围栏释放记录型 Spy | 已钉住"删会话即请求撤销授权" |
| `src/core/ports.rs:NoopLog` | 无声日志 Stub | 已存在；不用于验证日志内容 |
| `tests/cross-platform/e2e/mock.js` | 本地假供应商服务 | 已用于 T5；应覆盖协议错误、断开、延迟等场景 |

### 4.3 Mock

Mock 表达预先声明的交互期望，适用于"必须调用一次""必须先调用 A 再调用 B""失败后禁止继续调用"等契约。

本项目不要求引入第三方 mocking 框架。优先使用手写记录型 Fake/Spy，以减少依赖和跨平台不确定性。只有当交互期望本身是被测行为时，才使用 Mock 语义。

### 4.4 Spy

Spy 记录调用现场供断言。`FakeChat.calls`、`FakeCatalog.seen`、`RecordingRunner.calls`、`RecordingFence.released` 是当前明确的 Spy 记录。Spy 不应改变被测依赖的其他行为，也不能因为记录方便而泄漏生产内部状态。

### 4.5 Fixture

Fixture 是可复用的固定输入或预期输出，例如 provider 配置、模型响应、transcript、工作区文件和工具输出。

Fixture 必须：

- 使用相对路径或测试隔离根；
- 不含真实密钥、账号、机器路径；
- 命名表达场景；
- 避免在多个测试中复制粘贴同一大段文本；
- 在测试失败时能定位到输入来源。

## 五、Fake 专项验收

以下条目当前不是"全部已完成"的声明；未完成项进入 `tests/gaps.yaml`。

### `FakeChat`

至少需要覆盖：

- 空脚本；
- 单条脚本和多条脚本；
- 多次调用时的消耗与重复语义；
- 完整消息列表记录；
- 消息顺序保持；
- streaming 回调行为；
- 回调返回 `false` 时的中止行为；
- 空响应和非法响应交给上层后的处理；
- 调用次数与业务预期一致。

### `DemoGateway`

至少需要覆盖：

- member channel 能被创建并完成调用；
- core channel 能被创建并完成调用；
- 回落通知存在且指向正确模块；
- 演示模式不发网络请求、不需要密钥；
- core 与 member 的脚本语义不会相互污染。

### 本地假供应商

至少需要覆盖：

- 正常响应；
- 非法响应；
- HTTP 错误；
- 延迟；
- 连接断开；
- 流式响应；
- 多次运行隔离；
- 端口、子进程和临时目录清理。

## 六、端口测试矩阵

端口矩阵是测试设计账，不允许只写"有 mock"而不说明 Fake 的能力。

| 端口 | 当前/计划替身 | 交互记录 | 失败注入 | 取消/超时 | 真实适配器 | 当前状态 |
| --- | --- | --- | --- | --- | --- | --- |
| `Chat` | `FakeChat` | `calls` | 待补完整场景 | 待核对 | `HttpChat` | 部分 |
| `ChatGateway` | `ScriptGateway`、`DemoGateway` | 部分 | 待补完整场景 | 不适用/待核对 | `HttpGateway` | 部分 |
| `SettingsStore` | `InMemorySettings` | 状态可观察 | 待补 | 不适用 | `YamlSettingsStore` | 部分 |
| `ModelCatalog` | `FakeCatalog` | `seen` | 待补 | 不适用 | `HttpModelCatalog` | 部分 |
| `ModuleSource` | `VecSource` | 不适用 | 待补 | 不适用 | `FsModules` | 部分 |
| `PackageSource` | `InMemoryPackages` | 不适用 | 待补 | 不适用 | `FsPackages` | 部分 |
| `Workspace` | `InMemoryWorkspace` | 状态可观察 | 待补 | 不适用 | `FsWorkspace` | 部分 |
| `SysIo` | `InMemorySysIo` | 状态可观察 | 待补 | 不适用 | `FsSysIo` | 部分 |
| `HistoryStore` | `InMemoryHistory` | 状态可观察 | 待补 | 不适用 | `FsHistory` | 部分 |
| `PromptSource` | `TestPrompts` | 不适用 | 待补 | 不适用 | `YamlPrompts` | 部分 |
| `ToolRunner` | `RecordingRunner`、`SilentRunner` | `calls` | 待补 | 待补 | `ProcTools` | 部分 |
| `FenceHost` | `RecordingFence`、`NoFenceHost` | `released` | 待补 | 不适用 | `confine::FenceHostAdapter` | 部分 |
| `Log` | `NoopLog` | 不记录（Stub） | 不适用 | 不适用 | `FileLog` | 部分 |

"部分"表示已有使用或局部覆盖，不表示该端口已经完成契约验收。

## 七、隔离、清理与副作用

每个测试必须声明其资源边界：

- 文件：使用临时根或测试专属目录；结束后清理；
- 网络：只绑定本地环回，测试后关闭监听；
- 进程：记录子进程，超时和失败路径也要杀整棵树；
- 权限：默认不写真实 ACL、profile 或系统策略；真机探针只能在显式 `--fence-live` 下执行；
- 配置：不得读取真实 `.home/`，不得覆盖用户设置；
- 日志和报告：写入 `target/` 下的测试目录，不把产物写进源码目录；
- 并发：测试不得共享可变全局状态，除非明确验证并发语义；
- 时间和随机数：需要确定性时注入 Stub 或固定种子。

测试成功、失败、panic、取消和超时都必须走清理路径。不能只在 happy path 清理资源。

## 八、质量、冗余和静态检查

代码冗余检查不是一个"再写几个测试"的业务测试，而是质量门禁。

### 必查项目

- 格式：`cargo fmt --check`；
- 编译：`cargo check --all-targets`；
- 警告：`cargo clippy --all-targets --all-features -- -D warnings`；
- 重复依赖：`cargo tree --duplicates`；
- 测试目标登记、报告结构、缺口账格式；
- 重复测试、重复 Fixture、重复 Fake 和跨层无理由重复断言；
- 未使用代码、死代码、无效分支和不必要包装层。

### 判定规则

- 质量检查失败记录为 `quality-fail`，不能折算成 `pass`；
- 工具缺失或环境不允许运行记录为 `env-skip`，不能静默跳过；
- 尚未建立检查记录为 `gap`；
- 依赖重复不一定是错误，必须有解释或后续治理记录；
- 重复代码检查不得诱导新增抽象。先判断重复是否属于同一职责，再决定合并、保留或记录原因。

当前 `node run-tests.js` 尚未统一执行上述全部 T0 检查，因此 T0 的完整门禁仍是全局缺口（`tests/gaps.yaml`）。

## 九、执行入口与报告

### 快速开发检查

```text
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

快速检查用于本地反馈，不替代完整入口。

### 完整本地入口

```text
node run-tests.js
```

当前入口的实际顺序是：

1. `cargo build`（并打印安全模式 / 真机围栏模式说明）；
2. 运行 `solomni --doctor`，记录当前平台和围栏能力；
3. 运行 L1 单元测试（`cargo test --bin solomni`）；
4. 逐个运行 `cargo test --test cross-platform/windows/linux/macos`；
5. 运行前端冒烟；
6. 如果存在编排器，运行 L4 端到端；
7. 写入 `target/test-report.json` 并打印 `TEST-REPORT-OK` 或 `TEST-REPORT-FAIL`。

当前入口还没有完整接入 T0；文档中列出的目标顺序不应被误读成当前代码已经实现。

### CI 真机入口

```text
node run-tests.js --fence-live
```

仅允许在一次性 runner、VM 或明确授权的环境使用。CI 仍应保留 Actions artifact 与 `ci-report` 报告分支中的报告和日志。

### 当前报告状态

当前实现实际会输出：

- `pass`：步骤完成；
- `fail`：断言或命令失败；
- `skip-platform`：当前平台不适用的空平台目标；
- `gap`：入口没有找到应运行的部分；
- `envSkips`：报告内记录环境性跳过及原因。

更细的状态区分（`test-fail`、`quality-fail`、`env-skip`、`blocked`）是尚未完成的缺口，记在 `tests/gaps.yaml`；
在代码入口完成升级前，不能把目标状态名称写入当前报告并宣称已支持。

## 十、成功标记

固定标记只增不删，改动含义必须同步更新测试设计：

- `FRONTEND-SMOKE-OK`：前端冒烟完成；
- `E2E-OK`：端到端场景完成；
- `TEST-REPORT-OK`：当前入口的运行步骤没有失败；
- `TEST-REPORT-FAIL`：当前入口有失败步骤（同时以非零退出码暴露）；
- `TEST-REPORT-ACCEPTED`：当前平台缺口账为空。

固定标记不能替代质量门禁，也不能覆盖 `env-skip`、`gap` 或 `quality-fail`。测试入口必须以非零退出码暴露失败。

## 十一、目录、目标与命名

当前测试目录：

```text
tests/
  helpers/
    probe.rs                    # T4 共用探针设施
  cross-platform/
    main.rs                     # T3 目标入口
    integration/                # T3（含 fence_launcher.rs）
    e2e/                        # T5：假供应商、驱动、编排与隔离根
    gaps.yaml                   # T3 缺口账
  windows/
    main.rs                     # 平台目标入口
    probes/                     # T4
    gaps.yaml
  linux/
    main.rs
    probes/
    gaps.yaml
  macos/
    main.rs
    probes/
    gaps.yaml
  gaps.yaml                     # T0/T2/T5 等全局缺口账
  ci-publish.mjs                # CI 报告发布脚本（把三平台报告写入 ci-report 分支）
```

四个平台目标在 `Cargo.toml` 中显式登记。新增测试目标、Fixture 或脚本必须能从入口追溯到执行位置，否则属于结构质量问题。

命名要求：

- 测试名称描述行为和条件，不描述实现细节；
- Fake/Stub/Mock/Spy 名称表达职责；
- Fixture 名称表达场景；
- 缺口 id 稳定、唯一、可在报告中引用；
- 平台专属行为放平台目录，跨平台行为放 `cross-platform`；
- 测试报告和日志写入 `target/`，不入库。

## 十二、缺口账

### 全局缺口

`tests/gaps.yaml` 记录 T0、T2、T5 和测试基础设施的跨平台缺口。

### 平台缺口

`tests/<platform>/gaps.yaml` 只记录平台机制或平台专属验收缺口。条目存在表示当前未完成，不得留下"已完成"的残条。

### 缺口格式

```yaml
- id: fake-chat.contract-tests
  scope: global
  level: T2
  why: 说明为什么该行为是必须验证的契约
  how: |
    写出可直接执行的命令或实现步骤
  accept: 可判定的通过条件
  blocked_by: 无 | 平台不可用 | 环境不允许 | 缺少观察面 | 缺少实现
```

每条缺口必须有：稳定 id、范围、层级、必要性、执行方法、验收条件和阻塞原因。补齐后删除条目，不保留完成历史。

## 十三、给开发者和 AI 的工作方法

1. 先阅读本文、[ARCHITECTURE.md](ARCHITECTURE.md) 和相关模块契约。
2. 先判断测试属于 T0-T5 哪一类，不要把质量检查写成业务测试。
3. 优先用工具取得事实：运行入口、查看报告、检查日志和 `gaps.yaml`。
4. 纯逻辑先写 T1；端口替身和真实适配器补 T2；真实边界补 T3；平台机制补 T4；完整旅程补 T5。
5. 新增 Fake 时，同时增加 Fake 的最小契约测试和失败注入说明。
6. 新增测试时检查是否已有等价 Fixture、Spy 或端口契约，避免复制粘贴。
7. 失败时修代码或测试；环境不允许时记录 `env-skip`；未实现时建立 `gap`；不要把任何一种写成通过。
8. 运行完整入口并阅读 `target/test-report.json`，确认报告与日志能解释结果。
9. 测试完成后检查 `git status` 和 `.gitignore`，确保没有测试产物被跟踪。
10. 只有当目标行为、质量门禁和当前平台缺口都符合要求，才可宣称本次测试验收完成。

## 十四、验收清单

一次测试相关修改至少要回答：

- 测的是什么行为，属于哪个 T 层？
- 是否有成功、失败、边界和资源清理断言？
- 是否依赖 Fake、Mock、Stub、Spy 或 Fixture？它们是否被单独验证？
- 是否需要真实文件、网络、进程、权限或平台探针？
- 测试是否会修改本机状态？如果会，如何隔离和回收？
- 测试是否被入口实际执行？
- 失败、跳过、缺口和质量失败能否在报告中区分？
- 是否引入了重复测试、重复 Fixture、重复依赖或无意义抽象？
- 相关 `gaps.yaml` 是否更新为当前状态？
- 文档、报告和日志是否只使用项目相对路径？

未能回答的问题不是"以后再说"，而是测试设计或观察面仍不完整，应进入缺口账。
