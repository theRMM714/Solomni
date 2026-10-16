# workspace（工作区）

> 沙箱寻址与共享区版本化。
> 本目录是该单元的唯一细则入口：本页 → [`unit-map.md`](unit-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：工作区与沙箱寻址（含越界判定）、建工作与上传、文件视图，以及**共享区版本化工作区**（内容寻址的提交记录、各 agent 的拉取基线、文件级三方比较与按提交点还原）。

**不管**：模块清单与 `module.yaml`（在 `modules`，本能力只消费它给的模块事实）、不执行工具（`tools`）、不写会话流水（`session`）、不决定谁能用哪些工具（`tools` 的角色表）。

## 二、入站契约与状态归属

`api::Workspace`（prepare / work_has / files / roots / **work_pull / work_commit / work_commit_user / work_status / work_restore / work_head / work_rewind_to / work_restore_point / work_discard_after** …）。提交记录带 `CommitAnchor(agent, line)`，回档据此按行精确定位。出站端口 `Workdirs` / **`WorkStore`**（版本库落盘：文件原语 + 内容寻址对象 + 提交记录 + 拉取基线 + 删提交/清 head）**只由 `service.rs` 持有**（R12）。
模块事实（含 `has_userdata`）由 `modules` 提供：沙箱把它收成 `modules_with_userdata` 一起注入，围栏派生据此决定派不派这条落点——domain 层不读盘。

沙箱还带**会话 id**（`Sandbox.session`）：围栏把它当**会话租约**（`FenceSpec.lease`）带进授权台账，同名 agent 的多个并发会话释放时互不误撤（机制侧见 [docs/tools/README.md](../tools/README.md) 五）。

虚拟机档的沙箱还带**路径翻译器**（`Sandbox.path_codec`，本机档为 None = 恒等）：系统工具寻址、模块工具在 guest 里
执行、以及用户 `@` 引用的改写共用同一座桥，三处给的是同一种写法（见 [docs/tools/tools-and-roles.md](../tools/tools-and-roles.md) 的「路径模型」）。
沙箱清单 `Sandboxes` 另存同一份 `codec`：协作的 `@` 改写发生在**名单确认之前**（那时 `list` 还是空的），
由 `Sandboxes::ref_roots()` 取用。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`permission`、`prompt`
- 谁在用我：`collab`、`conductor`、`session`、`tools`

## 四、改动本单元时必须同步

- 用例测试在 `src/tests/workspace.rs`；运行包与执行档位的契约不在本单元（见 [docs/vm/README.md](../vm/README.md)）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |