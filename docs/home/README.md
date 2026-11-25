# home（登记处资源）

> 定位：`data/.home` 这块盘的唯一主人——哪几份登记处文档能存在、怎么读、怎么写（原子、私密）。
> 效力：规范｜核对：2026-11-25｜依据：`src/units/home/api.rs`
> 本目录是单元唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）；分层与依赖方向见 [layers.md](../architecture/layers.md)，业务边界判据见 [boundaries.md](../architecture/boundaries.md)，测试规范见 [TESTING.md](../../TESTING.md)。

## 管什么 / 不管什么

- 管：登记处文档的**清单**（`Doc`：providers / models / settings / agents / secrets / modules）、**读取**（文件不存在 = `None`，其余错误如实报）、**整份写回**（建目录 → 临时文件 → rename，读者读不到半份）、**私密权限**（除 `modules.yaml` 外一律 0600）。
- 不管：文档的**内容形态**——yaml 结构与解码留在各业务单元（`catalog` 的 `registry` / `secrets` / `modules`），本单元只搬运原样文本。
- 不管：路径本身（`data/.home` 由 `kernel::domain::layout` 给，组合根传进来）。
- 不管：校验规则（如沙箱授权自洽）——那是 `catalog::registry::domain` 的判断。

## 入站契约与状态归属

- 入站面：`api`（`Home` + `Doc`）。
- 出站端口：`HomeFiles` **只由 `service.rs` 持有**（R12）；真实实现 `detail::FsHome` 只由组合根构造。
- 状态归属：**无内存状态**——每次读写都直达盘；并发读写由文件系统与 rename 原子性保证，本单元不加锁（登记处的串行由调用方与单实例门共同保证）。
- 契约方向：`catalog` 的三个持久化适配器（`registry::detail::YamlSettingsStore`、`secrets::detail::YamlSecrets`、`modules::detail::YamlModuleState`）依赖 `Home`，本单元**不依赖任何业务 DTO**——资源单元不反向依赖业务，这是本单元只搬运文本的原因。

## 依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`
- 谁在用我：`catalog`、`entry`（组合根装配）

## 一、盘上有什么

| 文档 | 文件 | 私密 | 内容归谁解释 |
| --- | --- | --- | --- |
| `Doc::Providers` | `providers.yaml` | 是 | `catalog::registry` |
| `Doc::Models` | `models.yaml` | 是 | `catalog::registry` |
| `Doc::Settings` | `settings.yaml` | 是 | `catalog::registry` |
| `Doc::Agents` | `agents.yaml` | 是 | `catalog::registry` |
| `Doc::Secrets` | `secrets.yaml` | 是 | `catalog::secrets` |
| `Doc::Modules` | `modules.yaml` | 否 | `catalog::modules` |

- 新增一份登记处文档 = 在 `src/units/home/domain/doc.rs` 加一个 `Doc` 变体（文件名 + 是否私密），在消费方接上解码；路径与权限不必再各写一遍。
- 台账不在这里：`data/state/`（进程账、围栏授权、锁、首启回执）归 kernel 与 `system`，见 [state.md](../architecture/state.md)。

## 二、写入必须原子

- 顺序固定：`create_dir_all` → 写 `.<名>.tmp-<pid>` → 私密则收紧权限 → `rename`。读者永远看到完整的一份，崩溃只留临时文件。
- 失败如实报（`建登记处目录失败` / `写登记处失败`），不吞错、不静默回退默认值——「配置读不到」与「没有配置」是两件事。

## 改动本单元时必须同步

- 新增 / 改动 `Doc` → 消费方解码（`catalog` 各子业务）与 [registry/README.md](../registry/README.md)、[secrets/README.md](../secrets/README.md)、[modules/README.md](../modules/README.md) 的落盘路径描述。
- 改动端口形状 → [docs/testing/doubles.md](../testing/doubles.md) 的端口矩阵。
- 盘面布局改动的唯一权威是 `src/kernel/domain/layout.rs` 与 [state.md](../architecture/state.md)；本页不重复常量。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
