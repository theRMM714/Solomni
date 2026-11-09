# 能力化架构升级 · 施工计划

> 临时施工文档（`tests/gates/docs/root-router.js` 白名单条目 `plan.md`），不对外。每完成一个阶段就把状态改成「已销账」并写一行证据；全部完成后删除本文件并移除白名单条目。
>
> 替代上一轮《上下文新鲜度与全局版本化 · 施工计划》（内容见提交 `7b368caa` 的 `plan.md`）。

## 0. 状态

| # | 分支 | 内容 | 状态 |
| --- | --- | --- | --- |
| S0 | `task/arch-upgrade` | 三条结构判据 + `[big]` 大文件回执 + 空基线 | 已销账（证据见 §4） |
| S1 | `task/builds-ports` | builds：抽端口、拆 service | 已销账（证据见 §4） |
| S2 | `task/storage-ports` | storage：抽端口、拆 service | 已销账（证据见 §4） |
| S3 | `task/workspace-split` | workspace：只拆文件，不抽端口 | 已销账（证据见 §4） |
| S4 | `task/provision-ports` | provision：六个端口 + 拆 service + 测试外迁 | 已销账（证据见 §4） |
| S5 | `task/conductor-split` | conductor：拆 work/turn、方法族声明依赖面 | 未开始 |
| S6 | （待定） | proxy 方法族是否升格为能力（评估，可取消） | 未开始 |

顺序依赖：S0 提供判据与基线，S1–S5 每个 PR 只做「减基线」；S6 依赖 S5。

## 1. 已拍板（不再重议）

- **D1 跨能力直连 `api`，不加防腐层。** 文档 §4 的 port+adapter 降为触发条件：对方可能远程化、需要在不改编排时替换实现、契约要按调用方裁剪——三条都不成立时不建。ARCHITECTURE 同次补一句「未来远程化时启用」。
- **D2 行数不进失败门禁。** 行长是气味，依赖是规则：气味进 `[big]` 回执（只报不拦，形状照 `run-tests.js` 的 `[slow]`），规则进失败判据。唯一硬线：**新建文件 >1200 行直接失败**。
- **D3 门禁 PR 独立先落地**（S0），之后每个能力 PR 的 diff 里「基线少一条」就是可验证的达标证据。
- **D4 起步能力选 builds**（741 行、有 IO 无端口、风险低），作为模板跑通四件套。
- **D5 conductor 准入与升格**：`conductor` 只收「无自有持久真相 + 需要 ≥2 个能力 `api`」的编排；有落盘真相的一律是能力。「被 3+ 能力复用」先试一主多从，只有会成环或造 hub 才升格；升格需成本门（api 面 + ports + unit-map + 测试分区 + 门禁条目 + 文档），并有降格条件（调用方掉到 1 个且契约不再版本化 → 并回）。

## 2. S0 判据规格（实现以本节为准）

实现位置：`tests/gates/structure/capability.js`（复用 `dependency.js` 的 `stripTestModules`；注册为 `structure/capability-internal`），基线 `tests/structure-baseline.json`（与 `dependency-baseline.json` 同构：每条判据一个数组，条目过期即失败；另加 `bigFiles` 快照仅供回执显示增量与判定「新建」）。

| 判据 | 命中条件 | 豁免 |
| --- | --- | --- |
| `serviceToDetail` | `src/capabilities/<c>/service/**` 出现 `crate::capabilities::<c>::detail` 或 `crate::kernel::detail` | 存量进基线 |
| `serviceIo` | 同上范围出现 `std::fs`、`std::process::Command`、`ureq`、`reqwest`、`std::net`（`std::path` 放行） | 存量进基线 |
| `siblingCycles` | 同能力 `service/` 下文件级 import 图有环 | 存量进基线 |

`[big]` 回执（**不进 exit code**）：

- 统计范围：`service/**`、`domain/**`、`detail/**`、`api.rs`。
- >800 行的文件按行数降序列出，带斜杠形式：`行数 (+相较基线的增量) io=N impl=M 目的槽: 能/不能`。
- 新建文件 >1200 行 → 失败（`fail`，不是 `quality-fail`）。
- 文案固定：**行数不是失败条件。若本次新增了独立职责，优先拆子模块或抽端口；仅为数字拆文件会更乱。**

## 3. 每个阶段的固定套路（四件套）

1. **拆**：`service.rs` → `service/<子业务>.rs`；子业务用自由函数 + 必要的私有状态，**不为一件事造一个 struct**（那是「接口爆炸」）。
2. **文档**：同次更新 `docs/<单元>/unit-map.md` 与 `ARCHITECTURE.md` 相关条目（R8）。
3. **注释**：`node run-hygiene.js --tighten`（拆文件会改 `comment-baseline.json` 的键）。
4. **门禁**：`node run-tests.js` 全绿 + 从 `tests/structure-baseline.json` 销掉对应条目，PR 正文贴「回执前后对比 + 基线剩余条数」。

## 4. 各阶段销账条件

- **S0**：三条判据对故意构造的违规能红（已验：新增 `use std::fs` → serviceIo 红；基线留空条目 → 「已过期」红；新建 1201 行文件 → 硬线红）；存量全部进基线后全绿（现 0 问题 / 15 条 `[big]`）；`[big]` 在汇总尾出现；`node run-tests.js --docs` 绿（本文件在根路由白名单内）。
- **S1 builds**：`service.rs` ≤800 行（741 → 726）；`std::fs` 经端口（`create_dir_all` 移进 `BuildStore::prepare_home`）；`serviceIo` 与 `serviceToDetail` 各销 1 条（剩 7 / 4）；端口契约测试在 `src/tests/builds.rs`（4 条，L1 886 passed）；`node run-tests.js` 与 `--docs` 全绿；顺手修掉用量脚本的 `[object Object]` 字节列。
- **S2 storage**：`service.rs` 876 → 473 行；盘点 / 回收 / 存活三个端口（`storage/ports.rs`，适配器 `detail::{survey,reclaim,guests}`）接住 `std::fs`、`qemu-img`、回收锁与 `ProcessBook`；`serviceToDetail` 销 2 条、`serviceIo` 销 1 条（剩 2 / 6）；端口契约测试 6 条在 `src/tests/storage.rs`（L1 892 passed）；顺手修掉显式清理 `freed_bytes` 恒 0（旧实现删完才盘点）；`node run-tests.js` 与 `--docs` 全绿，`[big]` 从 15 条降到 14 条。
- **S3 workspace**：2533 行拆到 service.rs 633 + service/{commit 492, sync 393, history 268, resolve 252, lifecycle 504, facade 194}，全在 800 行内；`siblingCycles` 保持为空；`[big]` 从 14 条降到 13 条（`workspace/service.rs` 退出软线，快照改为 633）；`Workspace` 的 trait impl 收进 `service/facade.rs`（Rust 不允许 impl 分块），29 个方法逐条转发；L1 仍 892 passed；`node run-tests.js` 与 `--docs` 全绿，`run-hygiene --tighten` 后 3493 处。
- **S4 provision**：`service.rs` 3727 → 566 行、`guest.rs` 617 → 357 行，生产段全部进 800 行软线；六个端口（`provision/ports.rs`：`Fetcher` / `HostFs` / `Ledger` / `Installers` / `GuestImage` / `Procs`，适配器 `detail::{fetch,host,ledger,install,guest_image,procs}`）接住 `std::fs`、`Command`、`ureq`、`ext4` 工具链与进程台账，`service` 只留编排（`run` / `ensure` / `status` / `reset_guest` + 选择与校验的自由函数）；`serviceToDetail` 销 1 条（剩 1）、`serviceIo` 销 3 条（剩 3）、`[big]` 从 13 条降到 12 条（`provision/service.rs` 退出软线，快照改为 566）；1519 行内联测试迁 `src/tests/provision.rs`（46 条，含 4 条端口契约）；L1 896 passed；`node run-tests.js` 与 `--docs` 全绿，`run-hygiene --tighten` 通过；CI 首轮 Linux 红（16 处 E0616：`GuestAttr` 拆到 `detail/guest_image/image.rs` 后字段仍私有，`#[cfg(target_os = "linux")]` 用例本机编不到），字段提到 `pub(crate)` 后 `gh workflow run test.yml --ref task/provision-ports` 重跑 11 个 job 全绿（`6983eb6a`）。
- **S5 conductor**：`work.rs` 拆 `work/{lifecycle,files,config}`；`turn.rs` 的 4 件事拆回各自编排位置；五个方法族文件头的「看得见父模块私有字段」改成「只用到 a/b/c 三个依赖」；`[big]` 里 conductor 两文件降到 800 以下。
- **S6**：按 D5 判据给 proxy 方法族出结论（升格 / 不升格 + 理由），结论写进 ARCHITECTURE，不升格也要留理由。

## 5. 不做清单

- 不改目录名 `application→service` / `adapters→detail`（19 个能力、5 个目录的机械重命名，零行为收益，违背 R8 成本）。
- 不给 `permission`/`slate`/`taskchain` 加 `ports.rs`（纯逻辑，R3 明确不抽象）。
- 不做跨能力防腐层（D1）。
- 不在本项目使用「独立部署」类判据（纯 Rust 单二进制，无部署单元）。
- 不重写 collab（已垂直切 9 文件、单向无环）。

## 6. 风险

- 拆 provision 要与真实 guest（qemu/ext4/debugfs）联调，S4 可能被拉长；S1–S3 先交付以免全线阻塞。
- `[big]` 回执若无人读会退化成装饰；对策是 PR 正文必须贴一次，且 `[big]` 出现在每次 `run-tests.js` 的汇总尾。
- 基线只减不增；若某阶段必须临时增加豁免，PR 里单独说明并限期销账。
- 平台条件编译的用例（`#[cfg(target_os = "linux")]` 等）在本机编不到：拆模块改动可见性时按这些块专门审读，Draft 阶段要平台结论用 `gh workflow run test.yml --ref <分支>`（`workflow_dispatch` 不受 Draft 跳过影响）。
