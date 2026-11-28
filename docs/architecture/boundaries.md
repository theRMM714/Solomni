<!-- docs-template: spec -->
# 业务边界与硬要求
> 定位：什么该独立成一个业务、状态归谁、文件多大算过大，以及所有业务必须守的硬要求 R1–R20。
> 效力：规范｜核对：2026-11-28｜依据：`tests/gates/structure/`、`src/units/`

## 适用与判据
- 新单元要不要独立、状态放哪、文件拆不拆、硬要求守没守之前读本文；本文件回答「什么该独立成一个业务」与「所有业务必须守的硬要求」，全部由 T0 结构审查机器判定（`tests/gates/registry.js`），零豁免。

## 一、业务边界判据（先归层，再判用例）

> 新单元先回答「它属于哪一环」，再回答「它有没有自己的用例」：分层是结构前提，用例是业务前提，两者都成立才独立成一个业务。

1. **有自己的用例**：一组有领域词、有**自有协议**、有**用户可见后果**的行为。**它说的概念属于某个参与方 → 就归那个参与方**；不属于任何参与方的概念（名单、任务链）才自成业务。
2. **能指名 ≥2 个调用方**（单元、呈现层、入口的模型工具面都算）：不抽出来，这两个（或更多）调用方就会各写一遍。**已被证实的重复是最强证据**；**只有一个调用方的编排不抽**，留在它自己的 `service.rs`。

- **归环是必填身份**：单元必须在 `layers.yaml` 里有一行，跨单元边只能指向**严格更低的环**，同环只按 rank 先后（`tests/gates/structure/layers.js`）。环号不回答「是不是业务」，它只回答「能依赖谁」。
- **机制不因调用方多而升格**：日志、路径、进程、围栏、平台差异归一化这类没有领域词、没有用户可见后果的机制，留在 `kernel` 或所属单元的 `detail/`，不按第 2 条算作业务；第 2 条只用来判断「已有自己用例」的单元是否值得独立。
- **已判例（代理工具族 → `proxy`）**：判据 1 成立——六工具有自有协议（`src/units/business/proxy/domain/proxy.rs` 的载荷、授权判据与回执形状）与用户可见后果（代用户建会话、转达、控会话）。判据 2 目前只有 `runtime` 一个调用方，它成立的方式是**端口反转**：`proxy` 声明宿主契约 `ProxyHost`（`src/units/business/proxy/api.rs`），由 `runtime` 的 `ProxyBridge` 实现（`src/units/business/runtime/service/bridge.rs`）、只在 `src/units/business/runtime/service/handle.rs` 的 `inject_proxy_handler` 装配——`runtime` 对它的依赖因此止于 `::api`，真实会话状态仍在 `runtime` / `turn` 一侧。
- **已判例（顶层编排 → `runtime`）**：判据 1 成立——编排有自己的领域词（启用工作、改需求、回档、停止、动作分发）、自有协议（`src/units/business/runtime/api.rs` 的 `RuntimeOps` / `ActionOps` / `ActionCall` / `Acted`）与用户可见后果（一条命令把几个下层用例按序跑完）；判据 2 成立——入口（`src/main.rs` 的 `Handle::spawn`）与呈现层（`src/presentation/faces.rs` 的 `Faces`）是两个调用方，两个渠道共用同一份编排。它**不搬**下层用例的身体：方法体只是「按序调几个下层用例 + 状态聚合」。
- **状态不参与这条判据**——它只在本文件第二节回答「这块状态归谁」。

## 二、状态归谁

- 有参与方拥有它的不变式 → 归**那个参与方**（它的 `service.rs` 写，别人只经 `api` 读改）；
- 没有参与方拥有、且 ≥2 个参与方共同依赖 → 归**依赖图上最高的那个使用方**（`turn_state` 的会话在世表、运行态、创建占位与裁决队，`runtime` 的作业登记）：由它持有、只经自己的 `api` 暴露，不许为这块状态另造一个横跨所有层的单元；命令队列本身是机制，在 `src/kernel/domain/actor.rs`；
- 只被一个参与方用、自己又没有不变式 → 它是**派生值**，留在 owner 的 `domain/`，现算。

- **持状态的单元只能写自己的状态**；别人的状态只经 `api` 拿（R4）（`src/units/business/runtime/service/core.rs` 只持有 `turn_state` / `session_model` / `work` / `lifecycle` 等单元的入站对象并薄转发，不代写它们的状态）。

## 三、三分法（正交：只决定它持有什么、依赖谁）

| 类型 | 判据 | 归属与形态 |
| --- | --- | --- |
| **应用型业务** | 处在依赖图顶层、按自己的用例把内环单元编排起来：有自有协议 + 用户可见后果 + 领域词 API | 独立业务；**可持状态**；**只向下依赖、不横向**，参与方不得反向调它（`runtime` 在最顶层） |
| **领域型业务** | 说的是自己的数据 / 概念，规则与状态自成一体 | 独立业务；**只持自己的状态**；有 IO 或可替换点才有 `ports.rs` / `service.rs`（`taskchain` 就是没有端口、没有 `service` 的领域型） |
| **地基库（`kernel`，环 0）** | 没有领域语义 | `kernel/`：托管跨层共享的形状与机制；**领域词测试**——它不需要知道什么是回合 / 回复 / 工具执行；**不解释形状的业务语义、不构造业务决策与文案**；有机制状态与机制端口，只被依赖 |

- 前两类是业务；第三类不是业务，是地基库，不参与「独立业务」计数。

## 四、反例：什么不配切成业务

- **纯机制**（线程、锁、定时、文件句柄、HTTP 客户端）→ `kernel` 或所属单元的 `detail/`；
- **纯逻辑 / 派生**（解析器、状态派生、提示词渲染）→ 所属业务的 `domain/`，**不加 trait**；
- **纯交付**（HTTP 路由、终端渲染）→ `presentation/{cli,web}/`，**不是单元**；
- **只服务单一调用方的脚本 / 编排** → 它自己的 `service.rs`；
- **DTO 中转站**（把两个单元的类型拼一起的适配业务）→ 新的水平层，禁止（`node run-tests.js` 的 T0 结构审查）。
- **动作表不等于分发器**：`runtime` 的 `ActionOps::act` 是**入口分发**（查表 → 校验 → 授权 → 审计，`src/units/business/runtime/service/action.rs`），执行体仍在各单元的用例里；把它写成什么都干的「万能分发器」等于把下层用例搬进顶层。
- **聚合面不是业务**：`Faces`（`src/presentation/faces.rs`）只做装配——回答「这块屏幕取哪几个面」，不得长判定；判定与编排留在 `runtime`，机制留在各单元的 `detail/`。
- **把可变借用当参数传**：账本这类纯值按值进出——交一份、拿回更新后的那份，不借别人的 `&mut`（`src/units/business/tools/api.rs` 的 `Recorded`；R16）。
- **把环境包当参数传**：把端口、状态与值拼成一个结构递给别的单元，等于传活对象；改成只传值 + 少量窄端口（`src/units/business/agentloop/domain/member_tools.rs`、R16）：该文件里的 `ToolFace` 是抽出来的值面，端口与活状态仍留在 `MemberTools`；核实用例只借一个窄端口 `CoreVerify`，不把 `MemberTools` 写进 api 签名。`agentloop` 的 `api` 不转出 `MemberTools`：装配入口（`src/units/business/agentloop/domain/assembly.rs` 的 `single_session` / `rebuild_single` / `proxy_session`）只收端口与值，核实环境经 `verify_tools(&VerifySpec)`（`src/units/business/agentloop/domain/verify.rs`）取。

## 五、粒度上限

按文件**角色**分档，覆盖全 `src/`（含 `kernel/`、`presentation/`、`entry/`、`main.rs` 与各单元的 `api.rs` / `api/` 子目录），判据只有一处：`tests/gates/structure/size.js`。

| 角色 | 认定 | 软线（只进 `[big]` 回执） | 硬线（门禁失败） |
| --- | --- | --- | --- |
| 实现文件 | 其余 `src/**/*.rs`；**生产行**不含内联测试块 | 800 行 | 生产 1200 行，总行 1500 行 |
| 测试用例 | `src/tests/**` 与 `#[cfg(test)] mod x;` 指向的兄弟文件 | 1500 行 | 2000 行 |
| 测试支撑 | `src/tests/doubles/**`、`src/tests/builders/**` | 800 行 | 1200 行 |
| 内联测试块 | 实现文件里的 `#[cfg(test)] mod tests { … }` | 200 行 | 400 行 |

- 没有棘轮、没有历史行数快照，也没有 `EXEMPT`：硬线写死在 `tests/gates/structure/size.js`，超线即失败。
- 软线是气味：超线回执行数、IO 次数与文件头目的槽写不写得下，**不改变成败**；硬线是职责没拆完。
- 内联测试只许**纯函数与全内存断言**；碰真实进程 / IO / 网络的用例放 `src/tests/`，不在实现文件里长成第二个测试文件。
- 一个单元的 `api.rs` ≤ **20 个方法**——按「一个调用角色的面」计数；`registry` 是已知的宽面，**它仍是一个单元**（判据是 §九.1，不是方法数）。
- 平台后端按**平台目录**切，再按关注点分（如 Windows 的 `acl` / `record` / `container`）。

## 六、硬要求清单

- **R1**：单元之间**只经对方的 `api` 交流**（trait + DTO）。不准引 `ports` / `domain` / `detail`；不准给别的单元的 `types` 写 `impl`；`::detail` 只有根能碰
- **R2**：依赖图**无环**，由 T0 结构审查机器判定
- **R3**：DIP **只画在 IO 或可替换点上**；纯逻辑刻意不抽象
- **R4**：**状态所有权排他**：一块状态只有一个单元写，别人只经 `api` 读改
- **R5**：**并发模型不变式**：单线程命令队列（`src/kernel/domain/actor.rs` 的 `Actor`）+ 单元间同步调用，**核心状态不加锁**
- **R6**：**跨层共享的形状有唯一家**：`kernel` 只托管端口载荷（`SessionId` / `ToolOutcome` / `Ask` 这类跨层搬运的事实）；不含端口的产品词汇归环 1（如 `exec_model` 的 `Side` / `TargetSpec` / `JobState`）。禁止各业务复制 DTO（R15）；`kernel` 不解释这些形状的业务语义，也不构造业务决策与面向用户的文案（`src/kernel/`、`src/units/pure/exec_model/`）
- **R7**：**不留兼容层**：项目是 GREEN FIELD，**只维护当前唯一实现**——旧实现直接删，不留转发壳或历史说明 <!-- hygiene-ignore -->
- **R8**：**文档同步**：改结构必须同一次改 `docs/architecture/` 对应细则 / `docs/<单元>/unit-map.md` / 相关细则 / `AGENTS.md` 路由表
- **R9**：**跨平台与路径**：一律 `PathBuf` 组件拼接；对外用 `/`；不假设平台
- **R10**：**测试跟着业务分区走**：`units/<单元>/<子业务>/` ↔ `src/tests/<子业务>.rs`（用例多的按域分成子目录，一域一个文件），单文件 ≤ 2000 行；用例只经 `::api` / `::ports` / 组合根 `crate::entry::assembly`，出现 `::service` / `::domain` / `::detail` 即失败（`tests/gates/structure/tests-api.js`）。（存量 8 个适配器边界用例按名豁免，记账在 `tests/gaps.yaml` 的 `tests.impl-partition-imports`）
- **R11**：**测试入口 = 生产入口**：禁止 `#[cfg(test)]` 专用语义入口（门禁 `structure/cfg-boundaries`）
- **R12**：**端口只有一个持有者**：定义它的那个单元的 `service.rs`；别人只拿 `api` 面。**例外**：`kernel` 的机制端口是全项目共享的机制接口
- **R13**：**跨单元的同形重复不许存在**：要么收进唯一所有者，要么独立成一个业务（见「判据」第 2 条）
- **R14**：**单元必须归环**：`layers.yaml` 是分环的唯一权威（§一）；跨单元边只许指向更低环，同环只许指向 rank 更低的单元，环 1 还必须满足纯层判据（无 IO / 无端口 / 无可变全局）；新增单元先归环，改环在 PR 说明理由（`node tests/gates/structure/layers.js`）
- **R15**：**值有唯一家**：跨边界传的值类型只在产生方单元定义一次（经 `api.rs` 导出），消费方直接 `use`，不许本地再造同形再转换（`tests/gates/structure/values.js`）
- **R16**：**边界只传值与端口**：跨单元 `api` 的参数与返回只能是值、端口（`dyn Trait`）或基本类型；活对象（带锁、带内部可变、带身份）与别人的 `&mut` 状态都不出境（`tests/gates/structure/values.js`）
- **R17**：**权限判定只有一套**：`PermissionSet` 只承载网络档位，合并只在 `apply_root` / `apply_narrowing`、判定只问 `allows_network()`；文件面是固定授予（围栏按沙箱推导），共享区归属在 workspace 的 `SharedScope`（`src/units/pure/permission/domain/permission/base.rs`）；外部拆字段即失败（`tests/gates/structure/permission-values.js`）
- **R18**：**同名类型不许同居一文件**：同一个文件里 `use` 进两个同名类型即失败，改名让名字自证身份，不用 `as` 遮歧义（`tests/gates/structure/import-names.js`）
- **R19**：**死参只回执**：`impl` 里有函数体的 fn，形参带下划线前缀且体内没用到，就是"传进来被丢掉"，T0 结构审查回执（`tests/gates/structure/dead-params.js`）
- **R20**：**相似函数只回执**：token 3-gram 包含度跨文件 ≥ 0.72 / 同文件 ≥ 0.88 的函数对进回执，判定权在评审——形状像不等于语义同（`tests/gates/structure/similar-functions.js`）

## 七、T0 判据与零豁免

- `node run-tests.js` 的 T0 结构审查按下列判据机器判定（规则在 `tests/gates/structure/dependency.js` 与 `tests/gates/structure/capability.js`，逐条清单用 `node run-tests.js --list`；零豁免，实际违规集合必须为空）：

- **apiOnly**：跨单元引用必须以 `::api` 结尾；任何非根引用别人的 `ports` 都算违规；
- **foreignImpl**：不得给别的单元的类型写 `impl`（`node run-tests.js`）；
- **coreCycles**：单元节点图无环；
- **values**：api 面的公共签名（按完整签名判，跨行也算）不得有 `&mut` 参数，豁免 `&mut self`、端口借用 `&mut dyn Trait` 与泛型 / 回调边界里的 `&mut`；api 面声明或转出的值带内部可变、值里塞具名状态的可变借用或闭包（借端口不算）、跨能力同形结构只回执（`tests/gates/structure/values.js`）；
- **layers**：单元必须在 `layers.yaml` 归环，跨单元边只能指向**严格更低的环**，同环只许指向 rank 更低的单元，环 1 不得有 IO / `ports/` / 可变全局（`tests/gates/structure/layers.js`）；环表形状、rank 序与扇出只回执，用 `node tests/gates/structure/layers.js` 看当前表；
- **internal**（单元内部）：`service` 不引本单元 `detail` 或 `kernel::detail`；`service` 不直接做 IO（`std::fs` / `Command` / 网络 / `canonicalize`）、不读环境变量（`std::env::var` 族）、不自己探测路径（`Path::is_dir` / `is_file` / `exists` / `read_dir`，经端口调用的不算）；同能力的方法族之间不成环；`domain` 不自己取时（`SystemTime::now` / `Instant::now` / `UNIX_EPOCH`）——记时刻经 `kernel::api::clock`，耗时要调用方传进来；覆盖 `api`（含 `api.rs` 与 `api/` 子目录）/ `service` / `domain` / `detail` 四个分区；
- **resourceNames**（介质名）：`.home` / `.vm` / `data` / `logs` / `session` / `runtimes` / `prompts` / `systools` 只在 `src/kernel/domain/layout.rs` 拼；业务代码写 `.join("<介质名>")` 即报错，改走 `kernel::api::layout` 的对应函数（`tests/gates/structure/resource-names.js`）；
- **rootEntries**（根目录条目）：仓库根只许受版本管理的骨架条目；产品介质一律写 `data/`，旧位置（`.home` / `.vm` / `logs` / `session` / `runtimes`）出现即报错；`modules/<id>/userdata` 也算越界（源码树不长数据）（`tests/gates/structure/root-entries.js`）；
- **kernelProducts**（内核无产品名）：`src/kernel` 下任何 `.rs`（含注释与测试）不得出现 `qemu` / `smbd` / `virtiofsd`——内核只提供通用机制，产品名与它的 argv、配置文件名留在单元里；命中即失败、零豁免（`tests/gates/structure/kernel-products.js`）；
- **envSurface**（环境面）：`std::env::` 只许在 `src/kernel/detail/io/env.rs`、`src/main.rs`、`src/entry/`、`src/presentation/` 出现（呈现面是终端能力探测）；其余一律经 `kernel::api::env`，`std::env::consts` 与 `split_paths` 不在判据内（`tests/gates/structure/env-surface.js`）；
- **fsSurface**（文件面）：写盘只经 `kernel::api::fs`——除 `src/kernel/` 与测试文件外，`std::fs` 的变更原语（写 / 删除 / 改名 / 建目录 / 复制 / 链接 / 改权限）与 `File::create` / `OpenOptions::new` 出现即报错（锁、旧版本比对、暂存与一次发布都在内核事务里）（`tests/gates/structure/fs-surface.js`）；
- **processSurface**（进程面）：跑外部程序只经 `kernel::api::process`——除 `src/kernel/` 与测试文件外，`std::process::Command::new` 出现即报错（等待、超时与杀树只有一处实现）（`tests/gates/structure/process-surface.js`）；
- **diskOwners**（盘主）：一块盘一个主——`layout::<介质>()` 只许主人单元、组合根与内核调用，别的单元走主人单元 api / 端口；主人表与 `layout.rs` 双向比对，改了名不登记即报错（`tests/gates/structure/disk-owners.js`）；
- **unitBuckets**（单元桶）：`src/units/` 下只许 `pure` / `resource` / `business` 三个桶，每个单元恰好住一个桶，桶的模块根声明与桶内目录双向一致，并与 `layers.yaml` 的单元清单双向比对；在顶层新加一个平铺单元即失败（`tests/gates/structure/unit-buckets.js`）；
- **ioOwners**（IO 面）：入站监听、回环客户端、出站装配各只有一份实现（`src/kernel/detail/io/net/`）；命中点只能落在实现文件或表里逐条登记过的例外（容器内回环中继绑 `127.0.0.2`、流式栈自建 `reqwest` client），其余一律经 `kernel::api::net`（`tests/gates/structure/io-owners.js`，口径见 [io.md](io.md)）；
- **toolPaths**（外部工具位置）：设置里的 `qemu_path` / `virtiofsd_path` / `smbd_path` 只在定义（catalog）、解析（provision）、写入（runtime）、装配（work、entry）读；别处要工具位置问 `provision::api::vm_tools` / `vm_tools_report`（`tests/gates/structure/tool-paths.js`）；
- **fileSize**（文件粒度）：按角色分档判行数（实现 / 测试用例 / 测试支撑）与内联测试块，软线只回执、硬线失败，无 `EXEMPT`（`tests/gates/structure/size.js`）；
- **reverse** / **presentation**：单元不反向依赖根或呈现层；呈现层只认入站面——各单元的 `::api`，加上内核的 `kernel::api`（回环端口、环境这类原语只有内核一份实现，屏里不许再写一套），具体哪屏能取哪几个由 [presentation/inbound.yaml](../presentation/inbound.yaml) 定；
- **channelCross**：呈现层的两个渠道（`cli` 与 `web`）互不依赖；
- **unitMap**：各单元的 `unit-map.md` 合起来与磁盘**双向一致**（结构审查）；**文档链接**：代码注释里的 `docs/**.md` 引用由结构审查判，文档之间的 md 链接与根 `*.md` 路由表由**文档审查**判（本地全量入口的「T0 结构审查」步骤跑全部门禁、含文档审查；CI 拆成 `docs` / `structure` 两个 job，`node run-tests.js --audit=docs` / `--audit=structure`）；
- **permissionValues** / **importNames**：权限判定不出 `permission` 单元、同名类型不并存在同一个文件里（`tests/gates/structure/permission-values.js`、`import-names.js`），两者都判红；
- **deadParams** / **similarFunctions**：死参与相似函数只回执、不进失败集（`tests/gates/structure/dead-params.js`、`similar-functions.js`）——它们是「该不该合」的候选清单，判定要人读；
- **commentContract**：全仓 `.rs` 的文件头四槽（目的 / 管 / 不管 / 联动）、`pub` 项的条目槽（目的 / 参数 / 返回 / 错误 / 约束）与行内注释限量；零存量：任何一处违规都直接失败（见 [comments.md](comments.md)）。

- **零豁免**：任一判据不成立即报错；不存在基线文件、豁免条目或过期销账流程。

## 检查
- 依赖方向、单元内部与硬要求 → T0 结构审查（`node run-tests.js`，清单 `node run-tests.js --list`）。
- 粒度上限 → `tests/gates/structure/size.js`。
- 文档与结构同步 → `node run-tests.js --audit=structure`、`node run-tests.js --docs`。

## 路由
- 分层与依赖方向：[layers.md](layers.md)
- 端口画在哪：[ports.md](ports.md)
- 状态与落盘契约：[state.md](state.md)
- 注释契约：[comments.md](comments.md)
- 测试层级与替身语义：[../../TESTING.md](../../TESTING.md)