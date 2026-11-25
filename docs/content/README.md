# content（内容盘资源）

> 定位：`data/modules`（模块私有区）与 `data/runtimes`（运行包）这块盘的唯一主人——落点怎么算、私有区谁来建、安装/卸载的形状。
> 效力：规范｜核对：2026-11-25｜依据：`src/units/content/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：**落点**——`data/modules`、模块 id → 私有区（`data/modules/<id>`）、私有区里的 `userdata/`、`data/runtimes`。
- 管：**私有区的确保存在**（`ensure_userdata`，幂等）。这是产品里唯一的"载入即写盘"，以前写在 `catalog` 的扫描器里、且落在源码树；现在归本单元，落点在 `data/` 下。
- 管：**模块 id 的路径段安全化**（`safe_segment`：只留 ascii 字母数字与 `-_`，丢弃 `.`，其余替 `_`）——兜底防穿越，不改变 `catalog` 的 id 契约。
- 不管：**模块清单与校验**——`module.yaml` 的读取、id/版本/能力/工具/服务/隐秘字段的拒收理由都在 `catalog`。
- 不管：**回收与盘点**——`storage` 从本单元取落点，删不删、什么时候删是它的事。
- 不管：**模块源码**——`modules/<id>` 是随包只读内容，归安装根；本单元只管它在 `data/` 下的私有区。
- 不管：**安装/卸载的实现**——本期只有形状与缺口（见 §三）。

## 入站契约与状态归属

- 入站面：`api`——`paths(install) -> ContentPaths`（落点派生，纯函数）、`Content`（`area` / `ensure_userdata` / `runtimes`）、`ContentInstall`（安装/卸载形状，本期未实现）。
- 出站端口：`ContentFs` **只由 `service.rs` 持有**（R12）；真实实现 `detail::FsContentFs` 只由组合根构造。
- 状态归属：**无内存状态**——派生器只持产品安装根，其余每次重算；`ensure_userdata` 的结果是文件系统事实，不缓存。
- 契约方向：`catalog` 的模块扫描器（`catalog::modules::detail::FsModules`）依赖 `Content`，本单元**不依赖任何业务 DTO**（`Module` 归 `catalog`）——资源单元不反向依赖业务。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`（`layout` 的介质名）
- 谁在用我：`catalog`（模块扫描与私有区事实）、`work`（guest 的模块根挂载）、`entry`（组合根装配）

## 一、盘上有什么

- 落点拼法只有一处：`src/units/content/domain/paths.rs`；新增一类内容落点 = 在那里加一个方法，别在消费方 `join`。

| 落点 | 路径 | 内容归谁解释 | 可写 |
| --- | --- | --- | --- |
| 模块私有区根 | `data/modules` | `catalog`（id 与清单） | 是 |
| 模块私有区 | `data/modules/<id>` | 本单元（只给落点） | 是 |
| 跨任务私有数据 | `data/modules/<id>/userdata` | 模块自己（作为进程 HOME） | 是 |
| 模块构建产物 | `modules/<id>/build`（安装根，**不在数据根**） | `builds` / 模块自身构建 | 是（由 `.gitignore` 忽略） |
| 运行包投放口 | `data/runtimes` | `catalog::packages`（`package.yaml`） | 是（用户投放） |

- 模块源码在 **安装根**的 `modules/<id>`（随包只读），与私有区严格分离：`userdata/` 已完全不落源码树。构建产物例外——`builds` 把模块目录挂成可写、cwd 就在里面，产物回写到 `modules/<id>/build`；那是**源码树里的本地产物**，由 `.gitignore` 忽略，不搬进数据根（数据根只放数据）。
- 私有区与源码的对应只经 `Content::area(id)` 表达；业务层要拿"模块能写哪儿"，拿的是 `Module.private`（`catalog` 扫描时从本单元问来的），而不是自己拼路径。

## 二、私有区为什么归这里，而不是模块源码目录

- 源码随包分发、可能装在只读位置（Windows 的 `Program Files`、容器的只读挂载）；把用户数据写进源码树，轻则污染版本库、重则直接失败。
- 以前的事实是**错的**：扫描器往 `modules/<id>/userdata` 里 `create_dir_all`，而盘上的数据早就搬到了 `data/modules/<id>`——模块进程因此拿不到自己的 HOME。
- 现在只有一处判断"私有区可用不可用"：`ensure_userdata` 的返回值，转成 `Module.private`（`Some` = 可用，`None` = 建不出来）。策略层据此决定派不派这条模块，`tools` 的围栏据此决定授不授写权限。

## 三、待办（本阶段没做，别当成已完成）

- **安装 / 卸载没有实现**：`ContentInstall` 的三个方法一律返回 `Err`，缺口 id 为 `content-install` / `content-remove`（登记在 `tests/gaps.yaml`）。`Installed` / `InstallPlan` 两个 DTO 先把形状定下来，等实现时再填。
- **`build/` 不进数据根**（已定）：构建的 cwd 就在模块目录，产物回写在 `modules/<id>/build`，由 `.gitignore` 固定忽略（见 §一 末条）。数据根只放 `userdata/`；`storage` 的模块产物盘点从 `catalog::modules::api::source_dir` 读源码树，不读数据根。
- **模块私有区的配额 / 回收**：`storage` 现在只展示模块目录里 `build/` 的占用（`CAT_OUTPUTS`，不自动回收）；`userdata/` 永不自动回收。

## 改动本单元时必须同步

- 新增 / 改动落点 → 本页 §一 的表格与 [state.md](../architecture/state.md) 的盘面契约。
- 改动端口形状（`ContentFs`）→ [docs/testing/doubles.md](../testing/doubles.md) 的端口矩阵。
- 改动私有区契约 → [MODULE_SPEC.md](../../MODULE_SPEC.md) 的模块目录说明与 [docs/modules/README.md](../modules/README.md)。
- 安装/卸载缺口实现后 → 删掉本页 §三 的第一条与 `tests/gaps.yaml` 的两条缺口。
- 盘面布局改动的唯一权威是 `src/kernel/domain/layout.rs`；本页不重复常量。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
