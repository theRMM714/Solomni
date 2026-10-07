# tools（工具与围栏）

> 系统工具、角色表与一次工具执行的围栏。
> 系统侧的门户是 [SYSTOOL.md](../../SYSTOOL.md)（入口与真相源表）；本目录是它的细则。
> 本目录是该单元的唯一细则入口：本页 → [`module-map.md`](module-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：系统工具总表与角色表（`systools/`）、内置文件工具（read/write/edit/patch/search）的执行编排与纯规则、工具参数契约、补丁通道、外部工具进程的执行、**围栏策略与平台实现**、授权记录与撤销（写 ACL 前先落台账；写后核对**覆盖标准、对象与回调 ACE**，身份 = ACE 类型 + SID + 权限位 + 对象 GUID，布局认不出的如实计数；失败回滚；收尾按台账还原或精确撤销，并在产品根内回收孤儿授权）。

**不管**：不放领域语义（角色是系统的身份，不是业务概念）；不选模型；不碰会话流水。

## 二、入站契约与状态归属

`api::Tools`（按角色发放工具面 / 总表 / 自检）+ `api::ToolExec`（跑模块与内置工具、释放围栏）。出站端口 `SysIo` / `ToolRunner` / `FenceHost` / `SystoolsSource` **只由 `service/mod.rs` 持有**（R12）。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：`kernel`、`llm`、`prompt`、`workspace`
- 谁在用我：`collab`、`conductor`、`session`、`slate`

## 四、改动本单元时必须同步

- 平台专属代码本地不编译——`FenceSpec` 字面量必须写全字段（跨平台字面量门禁）；改围栏 → `tests/<平台>/` 探针；改工具表 → `systools/` 与本目录 `tools-and-roles.md`。
- 围栏的写后核对、回滚与台账在 `src/capabilities/tools/detail/confine/windows/`：改落点形状（`GrantTarget`）或台账字段时，`prepare_fence` / `release_fence` / `clean` / `sweep_orphan_aces` 要一起改。
- ACE 的读法只有一处：`acl.rs` 的 `ace_parts` 按 ACE 头算 SID 偏移（标准 ACE 与回调 ACE 从第 8 字节起，对象 ACE 再加 Flags(4) 与在场的 GUID）。改覆盖的 ACE 类型或身份形状时，写后核对、诊断转储与 `windows/tests.rs` 的对象 ACE 往返探针要一起看。
- 容器里**列目录**的判据：同一个授权叶子上，cmd 的 `dir` 与 PowerShell 的 `Get-ChildItem` 会被拒，
  而 cmd 的 `for` 枚举与 python 的 `os.listdir` / `os.scandir` 正常（真机机制矩阵实测：叶子 DACL
  给容器的是 Modify，写读往返也通）。所以"列自己的产物"用 `for` 枚举钉住
  （`container_roundtrip_sees_leaf_but_not_parent_content` 的第四步），`dir` 不是可依赖的列举手段。
- 命令里的解释器有**基线**：`node <文件>` 的命令由围栏注入 `NODE_OPTIONS=--preserve-symlinks --preserve-symlinks-main`
  ——node 的 `fs.realpathSync` 先 lstat 盘卷根、再逐级 lstat 祖先前缀，而这两类落点按设计都不在可达范围
  （卷根属主是系统、非管理员改不动；祖先链只靠令牌的「按名穿过」特权，管不到显式 lstat），少了它进程在脚本执行前就 EPERM 死。
  只在本平台注入（Landlock 不管 stat、seatbelt 已给祖先放行 `file-read-metadata`）；判据与 `interpreter_dirs` 共用
  同一份 PATH 解析（`command_programs_in`），而白名单随命令而变——`--print-fence-env` 与探针都按同一条命令问。
- 授权面按**注入的事实**派生：`<module>/userdata/` 有没有，由 workspace 扫描读出并随沙箱注入（domain 不读盘）；没有就不进 `rw`，`standalone` 的缺省工作目录退回模块根。`prepare_fence` 也会跳过不存在的落点、不判整次失败。
- 核心代理系统工具的待实现规划清单：`systool_gaps.yaml`（仓库根；不是当前工具表，也不替代测试缺口账）。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`module-map.md`](module-map.md) | 逐文件职责（T0 与磁盘双向比对） |
| [`tools-and-roles.md`](tools-and-roles.md) | 系统工具、角色（身份）与「谁能用哪些工具」 |