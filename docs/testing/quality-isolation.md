# 隔离、清理与质量门禁

> 本文是**测试资源边界与 T0 质量门禁的唯一权威**：每个测试必须声明资源边界，质量失败不能被业务测试通过抵消。
> 入口与 CI 见 [execution-ci.md](execution-ci.md)，层级与判定见 [levels.md](levels.md)。

## 一、隔离、清理与副作用

每个测试必须声明其资源边界：

- 文件：使用临时根或测试专属目录；结束后清理；
- 网络：只绑定本地环回，测试后关闭监听；
- 进程：记录子进程，超时和失败路径也要杀整棵树；
- 权限：默认不写真实 ACL、profile 或系统策略；真机探针只能在显式 `--fence-live` 下执行；
  写了就必须撤回：真机测试收尾要按台账撤掉自己写下的 ACE、删掉自己建过的容器 profile
  （端到端收尾调 `--fence-clean`，`confine` 的 ACL 契约测试调 `confine::clean`），并把回收结果如实打印——不许留下没人管的痕迹；
- 配置：不得读取真实 `.home/`，不得覆盖用户设置；
- 日志和报告：写入 `target/` 下的测试目录，不把产物写进源码目录；
- 并发：测试不得共享可变全局状态，除非明确验证并发语义；
- 时间和随机数：需要确定性时注入 Stub 或固定种子。

测试成功、失败、panic、取消和超时都必须走清理路径。不能只在 happy path 清理资源。

## 二、质量、冗余和静态检查

代码冗余检查不是一个"再写几个测试"的业务测试，而是质量门禁。

### 2.1 必查项目

- 格式：`cargo fmt --check`；
- 编译：`cargo check --all-targets`；
- 警告：`cargo clippy --all-targets --all-features -- -D warnings`；
- 重复依赖：`cargo tree --duplicates`；
- 测试目标登记、报告结构、缺口账格式；
- 重复测试、重复 Fixture、重复 Fake 和跨层无理由重复断言；
- 未使用代码、死代码、无效分支和不必要包装层。

### 2.2 判定规则

- 质量检查失败记录为 `quality-fail`，不能折算成 `pass`；
- 工具缺失或环境不允许运行记录为 `env-skip`，不能静默跳过；
- 尚未建立检查记录为 `gap`；
- 依赖重复不一定是错误，必须有解释或后续治理记录；
- 重复代码检查不得诱导新增抽象。先判断重复是否属于同一职责，再决定合并、保留或记录原因。

当前 `node run-tests.js` 已执行上述全部 T0 检查，且**全是零容忍硬失败**：
编译、结构审查、格式、clippy、编译告警、依赖重复——任何一项不过即 `quality-fail`，**没有存量基线**。
清单与判定见 [levels.md](levels.md) 的 T0 一节。

三平台各自跑同一套检查：clippy 只编译当前平台的 `#[cfg]` 代码（Windows 的容器围栏在 unix 上不存在，
反之亦然），依赖上 Windows 走 native-tls、unix 走 rustls——所以**任一项都必须在三平台 CI 上分别成立**。
格式偏差与平台无关，但路径先做**词法归一**（收掉 `.` 与 `..` 段）：同一个文件被多个测试目标用 `#[path]`
引用时（rustfmt 在 unix 上会报成 `tests/<目标>/../helpers/probe.rs`）不会被算成两个。

## 三、设计取舍的窄 allow 清单

有些 lint 指出的是**有意的设计取舍**，不是没修。这些一律**就近窄 allow + 写清理由**，不允许宽范围放行：

| lint | 位置 | 为什么是取舍而不是缺陷 |
| --- | --- | --- |
| `too_many_arguments` | `core/collab.rs` 的 `start` / `restore`、`core/session.rs` 的 `new` / `restore`、`core/mod.rs` 的 `Core::new` | 全是**组合根注入的构造函数**：参数天然多，收口成参数对象只是把参数挪个地方、并让装配更难读 |
| `too_many_arguments` | `core/engine.rs` 的 `core_operation` / `converse_with` / `turn_with` / `Execution::review`、`core/session.rs` 的 `discussion_turn`、`core/collab.rs` 的 `judge_clear` / `review_nodes` | 同一组参数（身份 / 工具面 / 通道 / 消息 / 出口 / 对照表 / 重填说明）：它们必须一路透传，收口成参数对象只是把参数挪个地方（`turn_with` 只服务内存测试通道） |
| `dead_code` | `core/events.rs` 的 `enum SessionEvent` | 事件词汇里的字段**不全在生产路径被读**（例如 `DiscussionDone` 的 `round` / `over_cap` 供呈现层做裁决确认页）；词汇就是线格式，字段随契约保留，删掉会让呈现侧拿不到事实 |
| `large_enum_variant` | `core/mod.rs` 的 `enum Session` | 两变体大小差得远，但装箱只换来一次间接寻址，却把"会话本体可直接移动"这个形状改掉 |

新增 allow 必须同时更新本表；理由说不清的就不该 allow。

