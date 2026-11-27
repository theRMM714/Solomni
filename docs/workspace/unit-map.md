# 单元地图 · workspace

> 定位：workspace（工作区）逐文件职责的唯一权威；由 T0 结构审查与磁盘双向比对。
> 效力：契约｜核对：2026-11-27｜依据：`src/units/business/workspace/api.rs`

> 本文是 **workspace** 逐文件职责的唯一权威；由 `node run-tests.js` 的 T0 结构审查与磁盘**双向比对**
> （表里每个路径必须存在；`src/` 下每个 `.rs` 都必须出现在某一张单元地图里）。
> 分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)；单元划分与文档路由见 [AGENTS.md](../../AGENTS.md)。

| 文件 | 职责 |
| --- | --- |
| `src/units/business/workspace/api.rs` | 入站面：`Workspace`（寻址 / 铺局 / 文件视图 / 用量 / 根）、沙箱纯数据、共享区归属与过滤规则的类型，以及外部工作区根策略（`product_root` / `policy` / `resolve` / `ambient` / `recheck`）的导出；版本化能力不在这一面（在 `versions::api`，其它能力只准用这里） |
| `src/units/business/workspace/service.rs` | 门面：`Workdirs` 只在这里（R12）；`WorkspaceService` 只做「在哪儿」的转发——布局、文件视图、用量、根、外部根判定、跨会话写闸路径、共享区落文件 |
| `src/units/business/workspace/service/roots.rs` | 外部工作区根的策略：按执行侧（host / guest）判网络路径 / 共享根 / 文件系统根 / UTF-8 与产品根的互包含（文本 + 落点身份），并做目录与可写探测；创建走 `resolve`，CLI 启动目录走 `ambient`（同一条阶梯、不探可写），加载复检走 `recheck`，三种用法共用同一份 `policy` 与 `check_bounds` |
| `src/units/business/workspace/service/facade.rs` | `Workspace` 的唯一 trait 实现：十个用例全部直接委托 `Workdirs`（Rust 不允许 trait impl 分块） |
| `src/units/business/workspace/ports.rs` | 出站端口：`Workdirs`（目录布局、外部根判定、跨会话写闸路径、共享区落文件） |
| `src/units/business/workspace/domain/seats.rs` | 沙箱清单的唯一构造点 `seat_sandboxes`：每席计划（agent 名 / 模块目录 / 生效权限面 / 工作台声明链）→ 私有根、模块目录、模块私有区与共享区归属；缺私有根当场点名报错 |
| `src/units/business/workspace/domain/workspace.rs` | 工作区与沙箱的纯数据与寻址（含越界判定）；`Sandbox.filter_rules` 指向版本库目录里的 `.workignore`——检索面（grep）与内容面共用这一份规则文件 |
| `src/units/business/workspace/domain/path_filter.rs` | 工作区过滤的唯一名单与匹配规则：`.workignore` 的文件名与播种内容、把规则文本编译成与 git 同引擎的匹配器（`Rules`）、按相对路径判定忽略（命中的目录连带整棵子树）；内容面与检索面都从这里取规则，不另立名单 |
| `src/units/business/workspace/domain/scope.rs` | 策略参数的唯一解释处：`edit_of`（参数 → 一层覆盖）/ `edit_params`（反向）/ `scope_from`（全局默认 → 根覆盖 → 逐层收窄 → 全局硬黑名单，链为空报错）；归属规则与值类型在 `permission` 的 `shared_scope` |
| `src/units/business/workspace/detail/fs_workspace.rs` | `Workdirs` 的落盘实现：`data/session/<工作名>/` 下的 `work/` 与各 agent 沙箱；准备会话时在版本库目录播种 `.workignore`（已存在不覆盖）；外部工作区绑定投影 `.work-root` 与共享区根解析；跨会话写闸的锁文件路径按落点身份做键（锁本身由调用方取）；用量统计带 2 秒内存缓存且不跟随目录链接 |
| `src/units/business/workspace/detail/walk.rs` | 目录遍历的实现：递归列文件、逐层按名字排序、给了 `.workignore` 规则就整棵剪掉被忽略的子树、按条数上限截断；`fs_workspace` 的清单用它（`src/units/business/versions/detail/walk.rs` 另有一份，跨单元 detail 不可见） |