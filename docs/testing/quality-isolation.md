# 隔离、清理与质量门禁

> 本文是**测试资源边界与 T0 质量门禁的唯一权威**：每个测试必须声明资源边界，质量失败不能被业务测试通过抵消。
> 入口与 CI 见 [execution-ci.md](execution-ci.md)，层级与判定见 [levels.md](levels.md)。

## 七、隔离、清理与副作用

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

当前 `node run-tests.js` 已执行上述全部 T0 检查，但强度分两级（见 [levels.md](levels.md)）：编译与结构审查硬失败；
格式、clippy、编译告警、依赖重复按 `tests/quality-baseline.yaml` 比对存量。存量清零是全局长期目标（`tests/gaps.yaml`）。
**基线不是豁免**：超出基线一样是 `quality-fail`，只有"已是存量"才不重复记账。

**基线按平台分区**，因为这三项本来就随平台变：clippy 只编译当前平台的 `#[cfg]` 代码（Windows 的容器围栏
在 unix 上不存在，反之亦然）；依赖上 Windows 走 native-tls、unix 走 rustls（多一个 `webpki-roots`）。
`clippy` / `check_warnings` / `duplicates` 各按 `windows` / `linux` / `macos` 分段；**当前平台缺分区 = `quality-fail`**，
不许静默通过。格式偏差与平台无关：路径先做**词法归一**（收掉 `.` 与 `..` 段），
于是同一个文件被多个目标用 `#[path]` 引用时（rustfmt 在 unix 上会报成 `tests/<目标>/../helpers/probe.rs`）
不会被算成两个。`--print-quality-baseline` **只重算当前平台的分区**，其余原样保留。

