# testing（横向：测试与质量）

> 这一格是**横向流程**，不属于任何一个业务单元。
> **门户与路由在 [TESTING.md](../../TESTING.md)**——本页只做目录索引，不复述任何判据。

## 本目录

| 文件 | 内容 |
| --- | --- |
| [`levels.md`](levels.md) | 测试分层、放哪、允许与禁止什么、怎么判定 |
| [`doubles.md`](doubles.md) | 替身（Stub / Fake / Mock / Spy / Fixture）语义与端口矩阵 |
| [`quality-isolation.md`](quality-isolation.md) | 资源边界、副作用清理、质量门禁与 `allow` 清单 |
| [`execution-ci.md`](execution-ci.md) | 本地入口、报告读法、成功标记、CI 与报告发布 |
| [`gaps-acceptance.md`](gaps-acceptance.md) | 目录与命名、缺口账格式、验收清单 |
| [`module-delivery.md`](module-delivery.md) | 模块作者要交什么测试证据 |

## 缺口账在哪

- 全局与平台：`tests/gaps.yaml`（长期目标与尚未实施的产品/机制缺口）、`tests/cross-platform/gaps.yaml`、`tests/<平台>/gaps.yaml`。