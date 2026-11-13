# 计划与流程

> 定位：本轮的待办队列、合并顺序、设计与验收流程；一份文档只记录计划，不承载规则。
> 效力：计划（非规范）｜核对：2026-11-13｜依据：`docs/workflow/README.md`

## 一、当前队列

| 顺序 | PR | 分支 | 内容 | 状态 |
| --- | --- | --- | --- | --- |
| 1 | #78 | `task/shell-facts-receipt` | shell 回执结构化事实落盘（`ShellFacts`），回放与实况同源 | Ready，CI 全绿，待合并 |
| 2 | #79 | `task/fence-prep-timing` | 围栏准备分根计时、进度提示、台账锁收窄、可信基座授权归位 | Draft |
| 3 | #80 | `task/portable-setup` | 便携启动：PATH shim、`--setup` 维护旗、首次授权后围栏预授与启动对账 | Draft |
| 4 | #81 | `task/plan-and-flow` | 本文件 | Draft，文档门禁已过，可随时合并 |

合并顺序固定为 #78 → #79 → #80（#81 可随时）。#79 / #80 都改 `docs/kernel/README.md`、`docs/permission/README.md`，先合并者在 main 上定稿，后者 merge 前 rebase 解冲突。

## 二、开工、阶段与收口流程

1. `git fetch origin main`；从 `origin/main` 切 `task/<简述>`。
2. 空提交占位（`chore: 开工 <任务>`）→ push → `gh pr create --draft`，正文按 `.github/pull_request_template.md` 填全。
3. 开工自检四项全为「是」才写实现；一个阶段 = 一个提交（代码、测试、文档同一提交）。
4. 每次 push 更新 PR 的「实施清单」「阶段记录」「验收证据」。
5. Draft 期不跑 CI；用户确认后标 Ready，触发三平台验收。
6. 收口本地门禁全绿、PR 正文改为最终状态；**Ready 与合并由用户执行**（`AGENTS.md` 第七节）。

| 门禁 | 命令 / 判据 |
| --- | --- |
| 格式 | `cargo fmt --all --check` |
| 静态检查 | `cargo clippy --all-targets --all-features -- -D warnings` |
| 单元与集成 | `cargo test` |
| 文档 / 结构 / 端到端 | `node run-tests.js`（收口全量；Draft 期至少 `--docs`） |
| 变更行覆盖 | `tests/coverage.mjs`：product ≥ 90%，security 100%（CI Linux 判定） |
| 文档同步 | 门户、细则、缺口账 `tests/gaps.yaml` 与实现同提交 |

## 三、PR4：围栏写入收敛

目标：把「首次 shell 等 327 秒」压到可接受量级，并让每会话不再重付 ACL 写成本。

- **祖先链**：用 `--fence-live` 探针在容器里逐级验证 `FILE_READ_ATTRIBUTES` 是否真的必要（目录判在、起子进程、读 `.tools`）。不必要 → `AncestorStat::WithinRoot`，省掉根外祖先写入（本机实测仓库外一级祖先 193 秒）；必要 → 改非传播写入（`SetFileSecurityW`）并用合成树断言子对象 DACL 不变。
- **会话目录先行**：`src/capabilities/workspace/detail/fs_workspace.rs` 的 `prepare` 先建空目录，写容器 SID 的 `(OI)(CI)`，再允许填充；首条 shell 只读命中，每会话零写。
- **工具链根**：`.tools` / `runtimes` 在空目录阶段预授递归只读，provision 出的新文件自动继承，避免既有大树重算。
- **产品根**：树规模低于阈值才静默写 `(OI)(CI)`；超阈值只提示并显式执行（`--setup=fence`），给出估算。
- **对账**：`machine_plan` 不再给根外祖先链落点；用户删除主会话时保留逐条撤销路径。
- **注释勘误**：`src/kernel/detail/confine/windows/acl.rs` 的祖先链注释与 `:522-547` 实际落点不一致，同 PR 修正。
- 文档同步：`docs/kernel/README.md`、`docs/permission/README.md`。

## 四、PR5：外部工作区与授权体验

- **成本预估**：绑定外部目录前后给出「约 N 个文件，首次约 X 秒」；`--setup=fence-calibrate` 在目标卷合成三档小树实测 `.home/fence-costs.json`，并按 DACL 状态分档（protected 与自动继承实测差 16 倍）。
- **用户选择**：`workspace_grant: none | once | always`——不授权（容器读不到）、授权一次（进台账）、始终授权；默认不做隐式全量写。
- **身份复用**：外部工作区授权按工作区身份复用，容器名去掉 session 分量，避免每个新会话重付 `ro_tree` 递归写。
- **后台化**：首次授权与删除会话的撤销都移到后台低优先级执行（orca 的一次性标记思路），前台只报告进度。
- **VM 档**：不写 ACL，隔离由挂载根 + VM 边界保证；补两个探针——junction / 符号链接不得越出挂载根、只读挂载不得在 guest 内被 remount 成可写。
- 缺口账：外部工作区 0 成本路径（path 型边界 / broker）记 `tests/gaps.yaml`。

## 五、设计原则

- **空目录先写 ACE 再填充**：自建目录（会话、work、agent 沙箱、`.tools`、`runtimes`）一律如此，成本接近零。
- **绝不向已存在的大树事后注入可继承 ACE**：Windows 写目录 DACL 会按继承把 ACE 物化到整棵子树（O(子树)），这是反模式；外部已有目录的 O(n) 只能显式确认。
- **系统盘根永不写**：盘根一级永不作为落点；中间祖先按 PR4 的探针结论收敛。
- **三类授权主体**：可信基座（`.tools` / `runtimes`）一次机器级只读；产品根一次 `(OI)(CI)` 只读属性；受保护数据按 reach 授容器 SID。
- **O(1) 优先**：能换成 path / 挂载边界的场景不逐文件改 ACL（VM 档已是这种形态）。
- **同事实一处权威**：规则写门户与 `docs/` 细则，本文件只引用不复制。

## 六、风险与待拍板

- 本机已真实写入机器级 ACL（仓库外一级祖先属性、产品根 `(OI)(CI)`、`.tools` / `runtimes` 递归只读）；撤除依赖 `--fence-clean`。
- 产品根 `(OI)(CI)` 一次性成本随对象数增长（本机 227k 对象、HDD 实测 56 秒）；阈值与提示文案需 PR4 定。
- 机器级 ACE 按设计不进对账回收，长期累积只能显式清理。
- `--fence-clean` 在大仓库上较慢，e2e 里需控制超时。
- 外部工作区成本属于 AppContainer 继承（copy）语义的固有代价；除非改 path 型边界或走 VM，否则无法消除。
