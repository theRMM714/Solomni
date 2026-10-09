# entry（入口层）

> 组合根、机器可读探针与围栏守门进程。
> 本目录是该单元的唯一细则入口：本页 → [`module-map.md`](module-map.md)（逐文件职责，机器比对）→ 其它细则。
> 分层与依赖方向见 [ARCHITECTURE.md](../../ARCHITECTURE.md) §一，业务边界判据见 §九；测试规范见 [TESTING.md](../../TESTING.md)。

## 一、管什么 / 不管什么

**管**：入口层共用机制（产品根规范化）+ 组合根（`main.rs`：`new` 出所有适配器并注入）+ 机器可读探针（`diagnostics/`）+ 围栏守门进程（`guard/`，**第二个程序入口**）。

**不管**：除装配与探针外无业务；**任何能力都不许依赖它**（门禁判定）。

## 二、入站契约与状态归属

探针命令行（`--doctor` / `--https-check` / `--print-routes` / `--print-fence-env` / `--fence-verify`，多数**恒退出 0**——判定归调用方）；围栏收尾与按条处置（`--fence-clean` 整体收尾，`--fence-reconcile` 按归属对账回收陈旧授权（启动期也自动跑一次），`--fence-ledger` 列清单 / `--fence-restore` 还原一条 / `--fence-revoke` 撤一条 / `--fence-profile-rm` 删一个 profile，都是隐藏模式、用户经文档知道）；守门进程协议（`--fence-run`，内部协议，模块作者不接触）。

## 三、依赖图位置（由源码的 `::api` 引用推导）

- 经 `::api` 用到：（无——依赖图最底层或纯领域）
- 谁在用我：（无）

## 四、改动本单元时必须同步

- 新增端口 / 适配器 → 改 `main.rs` 装配；新增探针 → 同步 `docs/testing/execution-ci.md`；路由目录的机器比对在 `src/tests/routes.rs` ↔ `docs/presentation/contracts.md`。
- 测试缺口：本单元**不是业务能力**，没有独立的缺口账；缺口记在 `tests/gaps.yaml`；格式见 [docs/testing/gaps-acceptance.md](../../docs/testing/gaps-acceptance.md) §十二。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`module-map.md`](module-map.md) | 逐文件职责（T0 与磁盘双向比对） |
