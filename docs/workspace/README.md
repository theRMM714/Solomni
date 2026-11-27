# workspace（工作区）

> 定位：工作区的布局与组装——在哪儿、怎么铺、这一席拿到哪份工作面。
> 效力：规范｜核对：2026-11-27｜依据：`src/units/business/workspace/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；共享区版本化在 [`docs/versions`](../versions/README.md)；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：按会话铺局（`ensure_layout` / `discard_layout`）、共享区落文件（上传）、文件视图与用量、根（共享区根 / 会话树根 / 各 agent 私沙根）、根会话的外部工作区绑定投影（`.work-root`）与根策略。
- 管：**工作面清单的唯一构造点**（`src/units/business/workspace/domain/seats.rs` 的 `seat_benches`）：调用方只交「每席要什么」（agent 名、模块目录、生效权限面、工作台声明链），本单元算出每席的私有根、模块目录、模块私有区与共享区归属，产出 `workbench::api::Benches`；缺私有根就地报「工作区没有给出 agent <名> 的工作面路径」。代理会话与普通会话走同一条路。
- 管：**不版本化的工作台策略 `plain`**（`src/units/business/workspace/service/plain.rs`）：能力位为空、共享区可写、不发任何 `work_*`、不给回合钩子——一席直接在共享那棵树上写；与 `versions` 的 `versioned` 同属 `workbench::api::BenchPolicy`，会话只在声明里记策略 id（缺省 `versioned`，见 `src/units/business/session_model/domain/workspace.rs` 的 `DEFAULT_POLICY`）。
- 管：**共享区归属的求值**（`src/units/business/workspace/domain/scope.rs` 的 `scope_from`）：全局默认 → 根覆盖 → 逐层收窄 → 全局硬黑名单，顺序错了就会放宽本该收紧的面；会话只在 `SessionMeta.workspace.params` 里存自己那层声明（`edit_of` / `edit_params` 是读写它的唯一入口）。
- 不管：**版本化**——提交记录、三条线、推送合并、差异与解决、回档 / 留档 / 回收、`work_*` 十一个工具的处理器与 `versioned` 策略，全部在 [`docs/versions`](../versions/README.md)；本单元只回答「在哪儿」与不版本化那一档，不持版本库状态。
- 不管：**哪个策略接进来**——策略表由组合根拼（`workbench::api::Policies::new`，首个即缺省），本单元只提供 `plain_policy`。
- 不管：**建工作**——工作名、通道、guest 与两段事务都在 `work`（`prepare_work` / `commit_work` / `abort_work`），本单元只按已有会话把目录铺出来，不产生会话也不决定工作名；不管模块清单与 `module.yaml`（在 `modules`，本单元只消费它给的模块事实）、不执行工具（`tools`）、不写会话流水（`session`）、不决定谁能用哪些工具（`tools` 的总表与会话能力的流程段与角色）。

## 入站契约与状态归属

- 入站面：`api::Workspace`，十法——`bind`（把根会话的工作区绑定刷成 `.work-root` 布局投影；`None` = 产品内布局）/ `ensure_layout`（按会话铺目录：会话目录、版本库 `.work` 与过滤种子、`work/`、各 agent 沙箱）/ `discard_layout`（撤销一次铺局：只删本产品建出的东西——空沙箱 / 空 `work/`、布局投影与版本库 `.work`；建会话失败时用）/ `work_has` / `files` / `usage` / `roots` / `is_external` / `shared_lock_path`（跨会话写闸的锁文件路径，锁由 `versions` 取）/ `write_work`（共享区落文件，上传用）。
- 装配侧另有 `api::seat_benches` / `api::scope_from`（工作面清单与共享区归属的唯一派生点，入参只收每席计划与会话声明链）；`api::ref_roots` 把 `Benches` 转成提示词侧的引用根。
- 状态归属：目录布局（`data/session/<工作名>/` 下的 `work/`、各 agent 沙箱、版本库 `.work` 的目录与过滤种子、外部绑定投影 `.work-root`）由本单元的落盘适配 `FsWorkspace` 持有；版本库里的提交记录、拉取基线、待恢复事务与跨进程锁属于 `versions`。
- 工作台声明（策略 id + 不透明参数）随会话 meta 走，本单元不认识会话元信息的形状：声明到参数的映射由 `turn` 做，本单元只收已经算好的参数（`Bench` / `SeatPlan`）。
- 用量统计（`usage`）在内存里按 `session + agents` 缓存 2 秒（前端 `@` 菜单与删除确认会短时间重复取；写路径不做逐写失效），统计时**不跟随**目录链接 / 联接——链接本身按一个文件计，避免重复计数与成环（`src/units/business/workspace/detail/fs_workspace.rs`）。
- 外部工作区：根会话可把共享区根绑到产品目录之外；CLI 启动的会话默认绑**启动目录**（`--no-workspace` 关闭；`ambient` 与 `resolve` 共用一条阶梯，但启动目录不探可写、判定不过就回退产品内 `work/` 并打印原因）。外部根与产品根的互不包含按**落点身份**（卷 + 文件 id）判，同一物理目录的 UNC / 盘符两种写法不会被当成两处；跨会话写闸也用它做键（`src/units/business/workspace/service/roots.rs`、`src/units/business/workspace/detail/fs_workspace.rs`）。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`exec_model`、`permission`、`prompt`、`workbench`
- 谁在用我：`work`、`turn`、`versions`、`turn_state`、`collab`、`runtime`、`main`（拼策略表）

## 改动本单元时必须同步

- `src/units/business/turn/turn/service/facts.rs` → 改 `seat_benches` 的入参形状要同步这里的唯一取事实处（`rebuild.rs` 的代理会话同一份计划）。
- `tests/specs/workspace.yaml` → 用例声明的文件清单与用例名。
- `docs/versions/README.md` → 共享区版本化的口径搬走后，两页不许各说一半。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
