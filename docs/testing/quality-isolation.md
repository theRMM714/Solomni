# 隔离、清理与质量门禁

> 本文是**测试资源边界与 T0 质量门禁的唯一权威**：每个测试必须声明资源边界，质量失败不能被业务测试通过抵消。
> 入口与 CI 见 [execution-ci.md](execution-ci.md)，层级与判定见 [levels.md](levels.md)。

## 一、隔离、清理与副作用

每个测试必须声明其资源边界：

- 文件：使用临时根或测试专属目录；结束后清理；
- 网络：只绑定本地环回，测试后关闭监听；
- 进程：记录子进程，超时和失败路径也要杀整棵树；
- 权限：默认不写真实 ACL、profile 或系统策略；真机探针只能在显式 `--fence-live` 下执行；
 写了就必须撤回：真机测试收尾要按台账撤掉自己写下的 ACE、还原根内路径的原始安全描述符、删掉自己建过的容器 profile
 （端到端收尾调 `--fence-clean`，`confine` 的 ACL 契约测试调 `confine::clean`），并把回收结果如实打印——不许留下没人管的痕迹；
 写 ACL 前先把记录落盘，写后核对（我们的 ACE 在不在、原有 ACE 集合有没有变少），失败就回滚；**不许用 `let _ =` 吞掉清理失败**，清不掉就返回错误；
- 临时目录：ACL 测试与探针不在共享临时根上授父目录 ACE——在自有 base 下再套一层 `target/`，父目录就是自有 base，home 也落在 base 内，收尾删整个 base；
- 配置：不得读取真实 `.home/`，不得覆盖用户设置；
- 日志和报告：写入 `target/` 下的测试目录，不把产物写进源码目录；
- 项目外写：借用系统工具链可以，但**缓存与临时一律在项目内**——`CARGO_HOME` 钉 `platform/<os>/`、`TMPDIR/TMP/TEMP` 钉 `target/tmp`；
  启动时清理系统临时目录里上次崩溃残留的 `solomni-*`，结束时快照 `~/.cargo` / `~/.rustup` 等缓存根，**新增即硬失败（零豁免）**；
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
- 供应链：`cargo audit`（已知 CVE）与 `cargo deny check`（许可证 / 禁用 / 来源，配置 `deny.toml`）；CI 用 `taiki-e/install-action` 预编译装，本地开发不引第三方、走 `cargo install` 到项目内；
- 测试目标登记、报告结构、缺口账格式；
- 重复测试、重复 Fixture、重复 Fake 和跨层无理由重复断言；
- 项目外写（env / 工具链）：借用系统工具链也不许写它的 home，缓存与临时必须留在项目内；
- 未使用代码、死代码、无效分支和不必要包装层。

### 2.2 判定规则

- 质量检查失败记录为 `quality-fail`，不能折算成 `pass`；
- 工具缺失或环境不允许运行记录为 `env-skip`，不能静默跳过；
- 尚未建立检查记录为 `gap`；
- 依赖重复不一定是错误，必须有解释或后续治理记录；
- 重复代码检查不得诱导新增抽象。先判断重复是否属于同一职责，再决定合并、保留或记录原因。

当前 `node run-tests.js` 已执行上述全部 T0 检查：编译、结构审查（含**门禁解析器自测**与 **`#[ignore]` 禁令**）、
格式、clippy、编译告警、依赖重复**零容忍硬失败**——任何一项不过即 `quality-fail`，**没有存量基线**。
供应链（`cargo audit` / `cargo deny check`）同为硬失败，但**工具缺失或取不到 advisory 数据**记 `env-skip`。
清单与判定见 [levels.md](levels.md) 的 T0 一节。

三平台各自跑同一套检查：clippy 只编译当前平台的 `#[cfg]` 代码（Windows 的容器围栏在 unix 上不存在，
反之亦然），依赖上 Windows 走 native-tls、unix 走 rustls——所以**任一项都必须在三平台 CI 上分别成立**。
格式偏差与平台无关，但路径先做**词法归一**（收掉 `.` 与 `..` 段）：同一个文件被多个测试目标用 `#[path]`
引用时（rustfmt 在 unix 上会报成 `tests/<目标>/../helpers/probe.rs`）不会被算成两个。

## 三、设计取舍的窄 allow 清单

有些 lint 指出的是**有意的设计取舍**，不是没修。这些一律**就近窄 allow + 写清理由**，不允许宽范围放行：

| lint | 位置 | 为什么是取舍而不是缺陷 |
| --- | --- | --- |
| `too_many_arguments` | `capabilities/collab/service/collab.rs` 的 `start` / `restore`、`capabilities/session/domain/session.rs` 的 `new` / `restore`、`capabilities/conductor/service/mod.rs` 的 `Conductor::new` | 全是**组合根注入的构造函数**：参数天然多，收口成参数对象只是把参数挪个地方、并让装配更难读 |
| `too_many_arguments` | `capabilities/session/service.rs` 的 `core_operation`、`capabilities/collab/service/round.rs` 的 `converse_with`、`capabilities/collab/service/discussion.rs` 的 `turn_with`、`capabilities/collab/service/synthesis.rs` 的 `Execution::review`、`capabilities/collab/service/driver.rs` 的 `discussion_turn`、`capabilities/collab/service/collab.rs` 的 `judge_clear` / `review_nodes` | 同一组参数（身份 / 工具面 / 通道 / 消息 / 出口 / 对照表 / 重填说明）：它们必须一路透传，收口成参数对象只是把参数挪个地方（`turn_with` 只服务内存测试通道） |
| `dead_code` | `capabilities/conductor/api/mod.rs` 的 `SessionOps::exists` / `is_running` | **入站契约是发布给前端的接口面**：二进制 crate 里暂时没有生产调用点的接口方法会被 `dead_code` 误报（`is_running` 是运行态的**权威查询**——`SessionView.running` 只是事件台对账副本，最终一致） |
| `dead_code` | `capabilities/session/domain/events.rs` 的 `enum SessionEvent` | 事件词汇里的字段**不全在生产路径被读**（例如 `DiscussionDone` 的 `round` / `over_cap` 供呈现层做裁决确认页）；词汇就是线格式，字段随契约保留，删掉会让呈现侧拿不到事实 |
| `dead_code` | `capabilities/workspace/api.rs` 的 `Workspace::work_restore` | 共享区"指定提交点还原"的**机制入口**（由测试钉住）：生产回档走 `work_rewind_to` / `work_discard_after` / `work_restore_point` 按行锚算目标，`work_restore` 作为对照留在接口面 |
| `large_enum_variant` | `capabilities/conductor/service/mod.rs` 的 `enum Session` | 两变体大小差得远，但装箱只换来一次间接寻址，却把"会话本体可直接移动"这个形状改掉 |

新增 allow 必须同时更新本表；理由说不清的就不该 allow。

## 四、不变量断言账（生产代码里的 panic 面）

生产代码（不含 `src/tests/**` 与 `#[cfg(test)]` 之后）共 **24 处** `expect` / `panic!`。它们只允许出现在
**同一函数内可静态看出必然成立**的位置——判据就在上文，`expect` 只是把「已判」写进代码：

| 位置 | 处数 | 为什么必然成立 |
| --- | --- | --- |
| `capabilities/collab/service/{collab,pump,turn_io,round}.rs`、`capabilities/conductor/service/rewind.rs` | 14 | 协作状态机的 `disc` / 任务链 / 工具上下文：进入这段之前刚判过存在，`expect("disc 已确认存在")` 与其后的 `expect("上臂已判存在")` 是同一判断的延续；`rewind.rs` 的两处（`l.get("tool")` 与其后跳过被总结行的同一判断）同理 |
| `capabilities/prompt/{domain/prompt.rs,domain/refs.rs,service.rs}` | 3 | 模板变量缺失 = **装配错误**（`prompts/` 或调用方写错），不在用户输入路径上；启动即炸好过渲染出半截文案 |
| `capabilities/tools/detail/proc_tools.rs` | 3 | `Command` 已声明 `Stdio::piped()`，`child.stdin` / `stdout` / `stderr` 的 `take()` 必为 `Some` |
| `capabilities/tools/detail/confine/macos.rs` | 2 | `CString::new` 的两个入参是不含 NUL 的字面量与临时路径 |
| `capabilities/tools/service/systool.rs` | 1 | `pending.get(path)` 的键由上一行同一函数算出 |
| `capabilities/collab/service/tool_loop.rs` | 1 | 每个工具调用在上一行都被配对写入了执行结果 |

新增 panic 面必须同时更新本表；说不清「判据在哪一行」的，改成 `Result`。

## 五、门禁之外：仓库卫生审查

门禁回答「这次改动有没有把代码弄坏」——零容忍、每次必过；**仓库卫生**回答「仓库里还欠多少、挂在谁身上」——
存量在迁移期必然存在（改到一半就是这样），所以它只报，不拦：

```text
node run-hygiene.js            # 报现状：注释契约存量（按规则汇总）+ 与快照的差 + 内容卫生
node run-hygiene.js --tighten  # 按现状下调 tests/comment-baseline.json（唯一会写文件的入口；出现新增就拒绝）
node run-hygiene.js --strict   # 有发现即非零退出（给想拿它当自查的人）
```

- **格式存量**（注释契约，见 `ARCHITECTURE.md` 的「注释契约」）：判定与比对只有一处（在这个脚本里）；
  门禁拿它做**棘轮**——按 `文件 × 规则` 计数，**只减不增**（新文件零容忍、老文件同类变多也算新增），变少则要求销账；
  收紧是显式动作且只能往下（有新增就拒绝写盘），不让门禁偷偷改数，也不让「先违规再收紧」洗成存量。
- **内容卫生**（只报）：悬挂的缺口 id（已删 id 从 git 历史取）、docs 与根文档里指向不存在的 `src/**.rs`；
  取不到 git 历史时如实写「没跑」，不当作通过。
- 它**不进 T0、不进 `TEST-REPORT-*`**：混进门禁只有两种结局——门禁天天红，或者开始放行，两种都不诚实。

## 产物落点（临时诊断与运行日志）

非源码产物只许落在下面两处，**不许平铺在 `target/` 根**——它是最容易攒出来的一类垃圾（真机上 `target/` 根曾被 90 多个
`gate*.log` / `e2e-*.log` / `probe-*.txt` 铺满，编辑器与搜索都被拖慢）：

- `target/logs/` —— 运行日志、门禁与端到端的原始输出、一次性诊断的 `.txt`；
- `target/scratch/` —— 一次性脚本与探针（`.mjs` / `.js` / `.ps1` / `.sh`）。

门禁自己写的每步日志仍在 `target/test-logs/`（按步骤名覆盖、不累积），测试隔离根仍在 `target/test-scratch/`；
`target/` 根只留工具自己读的那几份（`test-report.json`、`e2e-report.json`、`.rustc_info.json`、`CACHEDIR.TAG`、`.dep-graph.json`）。
平铺即视为没收尾。