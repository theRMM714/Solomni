# 测试架构（TESTING）

> 本文是**测试的唯一权威**：分层与禁令、目录与命名、缺口账的纪律、入口与报告格式、覆盖矩阵、给 AI 的工作方法。
> 开发规则见 [ARCHITECTURE.md](ARCHITECTURE.md)，模块契约见 [MODULE_SPEC.md](MODULE_SPEC.md)，仓库协作规则见 [AGENTS.md](AGENTS.md)。

## 立场

- **测试是事实账，不是通过率**：跑不了就说跑不了，没测就记成缺口。
- **环境不允许 ≠ 通过**：跳过必须打印原因并计入报告（`env-skip`），绝不静默算过。
- **缺口必须成账**：每个平台一份 `gaps.yaml`；一个平台"验收通过" = 该平台的测试目标全绿 **且** 该平台的 `gaps.yaml` 清空。
- **判据**：任意环节、任意实现都必须能单独 mock 测试（见 [AGENTS.md](AGENTS.md) 代码规范）。

## 〇、安全边界（硬规矩）

- **测试不得改本机状态**：会写权限项、建容器 profile 的测试（Windows 容器探针、端到端里的真实围栏写入）**默认跳过**并打印原因；只有显式 `--fence-live` 才真跑，且建议只在**一次性环境**（CI runner / VM / WSL）里开。
- **`node run-tests.js`（默认）零副作用**：L1、跨平台集成、前端冒烟照常跑；端到端在「不写本机权限」的模式下跑（工具走无围栏路径，断言不变）。
- **产品侧也要同意**：本程序只在用户显式授权（`.home/settings.yaml` 的 `fence_write: true`，或环境变量 `SOLOMNI_FENCE_WRITE=1`）后才写权限项；未授权时如实告知「容器围栏未启用」，按无围栏执行。写了什么会在 stderr 逐条列出（`[围栏] 已写权限 N 处：…`）。
- **可精确回收**：`solomni --fence-clean` 按授权台账（`.home/fence-grants.json`）逐条撤销并删掉我们建过的容器 profile；删会话时也会按同一台账撤掉该会话的授权。

## 一、四层

| 层 | 测什么 | 放哪 | 硬规矩 |
| --- | --- | --- | --- |
| **L1 单元** | 纯逻辑、单个实现的行为（用内存适配器 mock 掉 IO） | 内联在模块里：`#[cfg(test)] mod tests` | 不碰盘、不碰网、不起进程；平台中立 |
| **L2 跨平台集成** | 真进程、真文件系统、本地环回 HTTP；与平台无关的行为 | `tests/cross-platform/` | 可起真进程，但**不依赖任何特权或特定平台的机制** |
| **L3 平台探针** | 该平台的真机机制（围栏、进程树、目录授权、断网、解释器可达性） | `tests/<平台>/probes/` | 机制可用就必须**真断言**；环境不允许才允许跳过，且必须打印原因 |
| **L4 端到端** | 假模型 + 本地假供应商，跑完整流程（单 agent 对话 / 协作五阶段 / 会话编辑 / 工作区与工具） | `tests/cross-platform/e2e/` | 不出网、不要真密钥、可反复跑；用隔离根，绝不碰真实 `.home/` 与 `session/` |

## 二、位置由可见性决定（不是风格）

本项目是 bin crate：`tests/` 里的集成测试**看不到 crate 内部项**（私有函数与内部状态）。于是：

- **要看 crate 内部 → 内联在模块里**（`#[cfg(test)] mod tests`）：这些测试天然是 L1。
- **只走公开面（二进制命令行 / HTTP / JSON 协议）→ 放 `tests/<平台>/`**：新写的测试默认走这条路，能黑盒就黑盒。
- 一个测试既想黑盒又需要 crate 内部的东西时，说明该机制缺少对外的观察面——那是设计问题：先补观察面（例如 `solomni --doctor` 这类机器可读输出），而不是把测试塞回 `src/`。

## 三、目录与 cargo 目标

```text
tests/
  cross-platform/
    main.rs                     # 目标入口
    integration/                # L2
    e2e/                        # L4（假供应商 + 驱动 + 夹具根）
    gaps.yaml                   # 跨平台层的缺口账
  windows/
    main.rs                     # 首行 #![cfg(windows)]：别的平台上整目标为空
    probes/                     # L3
    gaps.yaml
  linux/
    main.rs  probes/  gaps.yaml
  macos/
    main.rs  probes/  gaps.yaml
```

四个目标在 `Cargo.toml` 里显式登记（新增测试文件必须挂到某个目标，否则不会被跑）：

```toml
[[test]]
name = "cross-platform"
path = "tests/cross-platform/main.rs"
name = "windows"
path = "tests/windows/main.rs"
name = "linux"
path = "tests/linux/main.rs"
name = "macos"
path = "tests/macos/main.rs"
```

平台目标的 `main.rs` 第一行是 `#![cfg(target_os = "…")]`：在别的平台上该目标编译为空，`cargo test --test linux` 在 Windows 上就是"0 个测试"——既不报错，也绝不假装跑过。

前端冒烟（`src/presentation/web/*.smoke.cjs`）由 `src/presentation/web/smoke.cjs` 自动发现，属 L2 的一部分。

## 四、缺口账（`tests/<平台>/gaps.yaml`）

```yaml
- id: linux.landlock.not-run        # 稳定 id：报告与文档都引用它
  level: L3                         # L1..L4
  why: 一句话说清"为什么这条必须有"与"现在为什么没有"
  how: |                           # 照着能做：命令 + 预期
    cargo test --test linux -- --nocapture
  accept: 可判定的验收条件（哪条测试通过 / 哪个文件存在）
  blocked_by: 无 | 平台不可用 | 环境不允许 | 缺少观察面（附说明）
```

纪律：

- **缺口清单是当前状态**：补上测试就把条目删掉，不留"已完成"的残条。
- `how` 必须能照着做，`accept` 必须可判定；写不出 `how` 的说明还缺观察面，`blocked_by` 写清。
- 平台验收 = 该平台目标全绿 + 该平台 `gaps.yaml` 为空；runner 会把两侧都报出来。

## 五、一条入口与报告

```text
node run-tests.js        # 主入口：任何环境都能跑（自己指向项目内工具链，逐层点名）；与 shell 无关，推荐
node start.js -test      # 便利入口：先做工具链前置检查（缺 Rust 会先征求同意），再把环境交给 run-tests.js
.\test.bat               # 便利入口的薄包装（PowerShell 要带 .\ ；cmd 里可直接 test.bat）
./test.sh                # 同上（macOS / Linux）
```

受限环境（不允许以管道抓子进程输出的沙箱）里，便利入口会在启动层自己的 `cargo --version` 探针上被拒（EPERM）——那种环境请用主入口 `node run-tests.js`：它把每步输出写进日志文件再解析，不抓管道。

执行顺序：

0. 说明：入口自身不抓管道，每步输出写进 `target/test-logs/*.log` 再解析（受限环境里用管道抓子进程输出会被拒），失败时打印该步日志尾部。
1. **`solomni --doctor`**：机器可读的本机事实（平台、围栏能力 `fs/net/tree` 与原因、能否写目录 ACL、能否建容器 profile、外部解释器是否可用）。**"这台机器能承载哪些测试"只以它为准**，不靠猜。
2. **L1/L2/L3**：`cargo test` 逐目标点名（缺目标即失败）。
3. **L4**：假供应商 + 隔离根跑端到端。
4. **前端冒烟**。
5. **汇总**：人读表格 + `target/test-report.json` + 固定标记 `TEST-REPORT-OK` / `TEST-REPORT-FAIL` + 退出码。

报告把每个测试目标归入四态之一：`pass` / `fail` / `env-skip`（环境不允许，附原因）/ `gap`（该平台缺这条，附 `gaps.yaml` 的 id）。

## 六、成功标记规范

- 每层一个固定标记，**只增不删**（改了报告立刻失准）：`FRONTEND-SMOKE-OK`、`E2E-OK`、`TEST-REPORT-OK`。
- 标记由测试自己打印，runner 只做汇总，不替测试下结论。
- 失败一律非零退出码。

## 七、当前覆盖矩阵

| 层 | cross-platform | windows | linux | macos |
| --- | --- | --- | --- | --- |
| L1 单元 | `src/tests.rs` 与各模块内联（核心、适配层、纯逻辑；含「删会话即请求撤销授权」的接线断言） | `confine/windows` 内联：授权探针（要看 crate 内部，只能内联）、能力自检 | —（探针已黑盒化到 L3） | —（同上） |
| L2 集成 | 守门进程协议、环境白名单、杀整棵树 | — | — | — |
| L3 平台探针 | — | `probes/container.rs` 四条：启动 / 越界读 / 越界写 / 断网（本机 env-skip） | `probes/fence.rs`：Landlock（尚未在 Linux 跑过，见 gaps） | `probes/fence.rs`：seatbelt（尚未在 macOS 跑过，见 gaps） |
| L4 端到端 | `e2e/orchestrator.js` 起假供应商 + 隔离根，跑 `e2e/driver.js` 全套断言（单 agent / 协作 / 代拟 / 工具 / 回档） | — | — | — |

## 八、给 AI 的工作方法

1. 跑入口：`node start.js -test`（或 `node run-tests.js`）。
2. 读汇总与 `target/test-report.json`：分清 `fail`（要修代码或测试）、`env-skip`（本机禁令，不是缺陷）、`gap`（要补的测试）。
3. 认领一条 `gaps.yaml`：按 `how` 实现，按 `accept` 判定。
4. 关账：删除该条目；再跑一遍，直到 `TEST-REPORT-OK` 且目标平台的 `gaps.yaml` 为空。
5. 禁令：不许把"跑不了"写成通过；不许在测试与文档里写机器路径；不许改成功标记的含义；不许让跳过不带原因。
