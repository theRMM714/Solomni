# testing（横向：测试与质量）

> 定位：`docs/testing/` 的目录索引：测试与质量是横向流程、不属于任何业务单元，判据门户在 `TESTING.md`，本页不复述判据。
> 效力：索引｜核对：2026-11-04

## 本目录

| 要找什么 | 读哪一份 |
| --- | --- |
| 测试分层、放哪、允许与禁止什么、怎么判定 | [levels.md](levels.md) |
| 替身（Stub / Fake / Mock / Spy / Fixture）语义与端口矩阵 | [doubles.md](doubles.md) |
| 资源边界、副作用清理、质量门禁与 `allow` 清单 | [quality-isolation.md](quality-isolation.md) |
| 本地入口、报告读法、成功标记、CI 与报告发布 | [execution-ci.md](execution-ci.md) |
| 目录与命名、缺口账格式、工作方法与验收清单 | [gaps-acceptance.md](gaps-acceptance.md) |
| 模块作者要交什么测试证据 | [module-delivery.md](module-delivery.md) |

## 缺口账

| 要找什么 | 读哪一份 |
| --- | --- |
| 全局与平台缺口：长期目标与尚未实施的产品 / 机制缺口 | `tests/gaps.yaml`、`tests/cross-platform/gaps.yaml`、`tests/<平台>/gaps.yaml` |
| 缺口账的格式、目录与命名、验收清单 | [gaps-acceptance.md](gaps-acceptance.md) |
| 缺口账与测试门禁、报告的关系 | [TESTING.md](../../TESTING.md) |

## 改动时必须同步

- `docs/testing/README.md` → 新增或改名的测试细则页：先在本页路由表登记
- `TESTING.md` → 判据改动：同次改本页与 `docs/testing/` 下对应细则
- `tests/gaps.yaml` → 记或销缺口：同次改 [gaps-acceptance.md](gaps-acceptance.md) 的清单
- `docs/README.md` → 本页增删细则入口时同次改它的「横向：测试与质量」分区
