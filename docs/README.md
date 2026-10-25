# 文档索引

> 本文件是 `docs/` 的细则索引；根门户是 [`AGENTS.md`](../AGENTS.md)。先读门户，再按单元或事项进这里。
> 每个业务与机制单元一个目录：目录里的 `README.md` 是唯一入口，`unit-map.md` 是逐文件职责（T0 与磁盘双向比对）。

## 单元

| 单元 | 入口 | 该单元还有 |
| --- | --- | --- |
| `conductor`（协调业务） | [README.md](conductor/README.md) | [unit-map.md](conductor/unit-map.md) |
| `collab`（协作会话） | [README.md](collab/README.md) | [unit-map.md](collab/unit-map.md)、[task-chain.md](collab/task-chain.md) |
| `session`（会话） | [README.md](session/README.md) | [unit-map.md](session/unit-map.md)、[session-model.md](session/session-model.md) |
| `tools`（工具与围栏） | [README.md](tools/README.md) | [unit-map.md](tools/unit-map.md)、[tools-and-roles.md](tools/tools-and-roles.md) |
| `permission`（权限） | [README.md](permission/README.md) | [unit-map.md](permission/unit-map.md) |
| `llm`（模型通道与协议） | [README.md](llm/README.md) | [unit-map.md](llm/unit-map.md) |
| `registry`（登记处） | [README.md](registry/README.md) | [unit-map.md](registry/unit-map.md) |
| `modules`（模块公地） | [README.md](modules/README.md) | [unit-map.md](modules/unit-map.md) |
| `packages`（运行包） | [README.md](packages/README.md) | [unit-map.md](packages/unit-map.md) |
| `builds`（模块构建） | [README.md](builds/README.md) | [unit-map.md](builds/unit-map.md) |
| `vm`（虚拟机档） | [README.md](vm/README.md) | [unit-map.md](vm/unit-map.md) |
| `workspace`（工作区） | [README.md](workspace/README.md) | [unit-map.md](workspace/unit-map.md) |
| `provision`（依赖补齐） | [README.md](provision/README.md) | [unit-map.md](provision/unit-map.md) |
| `prompt`（提示词册） | [README.md](prompt/README.md) | [unit-map.md](prompt/unit-map.md)、[prompts.md](prompt/prompts.md) |
| `slate`（名单） | [README.md](slate/README.md) | [unit-map.md](slate/unit-map.md) |
| `residents`（常驻服务） | [README.md](residents/README.md) | [unit-map.md](residents/unit-map.md) |
| `secrets`（隐秘字段） | [README.md](secrets/README.md) | [unit-map.md](secrets/unit-map.md) |
| `taskchain`（任务链） | [README.md](taskchain/README.md) | [unit-map.md](taskchain/unit-map.md) |
| `kernel`（机制型业务） | [README.md](kernel/README.md) | [unit-map.md](kernel/unit-map.md) |
| 入口层（组合根 / 探针 / 守门进程） | [README.md](entry/README.md) | [unit-map.md](entry/unit-map.md) |
| 前端 / 交付机制（`cli` 与 `web`） | [README.md](presentation/README.md) | [contracts.md](presentation/contracts.md)、`cli/`、`web/` |

## 横向：测试与质量

门户是 [`TESTING.md`](../TESTING.md)；`docs/testing/` 是细则。

| 要做的事 | 读这一份 |
| --- | --- |
| 判断测试属于哪层、放哪、怎么判定 | [levels.md](testing/levels.md) |
| 写或改替身（Stub / Fake / Mock / Spy / Fixture）、验收 Fake | [doubles.md](testing/doubles.md) |
| 新增端口或替身、核对真实适配器覆盖范围 | [doubles.md](testing/doubles.md)（端口矩阵在 §三） |
| 声明测试资源边界、清理副作用、质量门禁 | [quality-isolation.md](testing/quality-isolation.md) |
| 跑本地入口、读报告、认成功标记、CI 与报告发布 | [execution-ci.md](testing/execution-ci.md) |
| 记缺口、目录与命名、按验收清单收口 | [gaps-acceptance.md](testing/gaps-acceptance.md) |
| 交付模块（模块作者要交什么测试证据） | [module-delivery.md](testing/module-delivery.md) |

## 流程与约定

| 要做的事 | 读这一份 |
| --- | --- |
| 任务分类、六步流程、开工 / 实施 / 收口、Git / PR | [workflow/README.md](workflow/README.md) |
| 文档分层、同步表、术语、引用格式 | [conventions.md](conventions.md) |

新增单元或文档时，先在本页登记入口，再从根门户链到它；细则内容按 [conventions.md](conventions.md) 的「改文档必须同步」处理。
