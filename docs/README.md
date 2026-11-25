# 文档索引

> 定位：`docs/` 的细则索引：业务子业务与机制单元各一个目录（`README.md` 是入口、`unit-map.md` 是逐文件职责），业务单元按分层表归并到子业务，根门户是 `AGENTS.md`。
> 效力：索引｜核对：2026-11-17

## 单元

业务层两级：**单元**（归层与门禁的单位，见 [architecture/layers.md](architecture/layers.md) 的分层表）与单元内的**子业务**（细则目录与测试文件按子业务走）。第一列写成「单元：子业务」。

| 要找什么 | 读哪一份 |
| --- | --- |
| `runtime`（顶层编排） | [README.md](runtime/README.md)、[unit-map.md](runtime/unit-map.md) |
| `work`（工作用例） | [README.md](work/README.md)、[unit-map.md](work/unit-map.md) |
| `lifecycle`（会话历史生命周期） | [README.md](lifecycle/README.md)、[unit-map.md](lifecycle/unit-map.md) |
| `conversation`：`session`（会话） | [README.md](session/README.md)、[unit-map.md](session/unit-map.md)、[session-model.md](session/session-model.md) |
| `conversation`：`slate`（名单） | [README.md](slate/README.md)、[unit-map.md](slate/unit-map.md) |
| `collab`（协作会话） | [README.md](collab/README.md)、[unit-map.md](collab/unit-map.md)、[task-chain.md](collab/task-chain.md) |
| `proxy`（核心代理工具） | [README.md](proxy/README.md)、[unit-map.md](proxy/unit-map.md) |
| `turn`：`turn`（回合状态） | [README.md](turn/README.md)、[unit-map.md](turn/unit-map.md) |
| `turn`：`env`（环境事实） | [README.md](env/README.md)、[unit-map.md](env/unit-map.md) |
| `maintenance`：`builds`（模块构建） | [README.md](builds/README.md)、[unit-map.md](builds/unit-map.md) |
| `maintenance`：`storage`（存储空间） | [README.md](storage/README.md)、[unit-map.md](storage/unit-map.md) |
| `catalog`：`registry`（登记处） | [README.md](registry/README.md)、[unit-map.md](registry/unit-map.md) |
| `catalog`：`modules`（模块公地） | [README.md](modules/README.md)、[unit-map.md](modules/unit-map.md) |
| `catalog`：`packages`（运行包） | [README.md](packages/README.md)、[unit-map.md](packages/unit-map.md) |
| `catalog`：`secrets`（隐秘字段） | [README.md](secrets/README.md)、[unit-map.md](secrets/unit-map.md) |
| `tools`（工具与围栏） | [README.md](tools/README.md)、[unit-map.md](tools/unit-map.md)、[tools-and-roles.md](tools/tools-and-roles.md) |
| `permission`（权限） | [README.md](permission/README.md)、[unit-map.md](permission/unit-map.md) |
| `exec_model`（执行与作业模型） | [README.md](exec-model/README.md)、[unit-map.md](exec-model/unit-map.md) |
| `registry_model`（登记处读模型） | [README.md](registry-model/README.md)、[unit-map.md](registry-model/unit-map.md) |
| `agentloop`（单会话运行时） | [README.md](agentloop/README.md)、[unit-map.md](agentloop/unit-map.md) |
| `turn_state`（回合状态与裁决机制） | [README.md](turn-state/README.md)、[unit-map.md](turn-state/unit-map.md) |
| `session_model`（会话身份与声明） | [README.md](session-model/README.md)、[unit-map.md](session-model/unit-map.md) |
| `transcript`（转录线格式） | [README.md](transcript/README.md)、[unit-map.md](transcript/unit-map.md) |
| `llm`（模型通道与协议） | [README.md](llm/README.md)、[unit-map.md](llm/unit-map.md) |
| `vm`（虚拟机档） | [README.md](vm/README.md)、[unit-map.md](vm/unit-map.md) |
| `workspace`（工作区） | [README.md](workspace/README.md)、[unit-map.md](workspace/unit-map.md) |
| `provision`（依赖补齐） | [README.md](provision/README.md)、[unit-map.md](provision/unit-map.md) |
| `system`（系统维护与自检） | [README.md](system/README.md)、[unit-map.md](system/unit-map.md) |
| `prompt`（提示词册） | [README.md](prompt/README.md)、[unit-map.md](prompt/unit-map.md)、[prompts.md](prompt/prompts.md) |
| `residents`（常驻服务） | [README.md](residents/README.md)、[unit-map.md](residents/unit-map.md) |
| `taskchain`（任务链） | [README.md](taskchain/README.md)、[unit-map.md](taskchain/unit-map.md) |
| `kernel`（地基库，环 0） | [README.md](kernel/README.md)、[unit-map.md](kernel/unit-map.md) |
| 根（启动 / 组合根 / 探针 / 守门进程） | [README.md](entry/README.md)、[unit-map.md](entry/unit-map.md) |
| 前端 / 交付机制（`cli` 与 `web`） | [README.md](presentation/README.md)、[contracts.md](presentation/contracts.md)、[cli/README.md](presentation/cli/README.md)、[web/README.md](presentation/web/README.md) |
## 横向：测试与质量

| 要找什么 | 读哪一份 |
| --- | --- |
| 测试与质量的门户与细则总入口（门户是 `TESTING.md`、细则是 `docs/testing/`） | [TESTING.md](../TESTING.md)、[testing/README.md](testing/README.md) |
| 判断测试属于哪层、放哪、怎么判定 | [levels.md](testing/levels.md) |
| 写或改替身（Stub / Fake / Mock / Spy / Fixture）、验收 Fake | [doubles.md](testing/doubles.md) |
| 新增端口或替身、核对真实适配器覆盖范围（端口矩阵在 §三） | [doubles.md](testing/doubles.md) |
| 声明测试资源边界、清理副作用、质量门禁 | [quality-isolation.md](testing/quality-isolation.md) |
| 跑本地入口、读报告、认成功标记、CI 与报告发布 | [execution-ci.md](testing/execution-ci.md) |
| 记缺口、目录与命名、按验收清单收口 | [gaps-acceptance.md](testing/gaps-acceptance.md) |
| 交付模块（模块作者要交什么测试证据） | [module-delivery.md](testing/module-delivery.md) |

## 流程与约定

| 要找什么 | 读哪一份 |
| --- | --- |
| 任务受理、开工与共同约定（单 / 多 agent 选择、闸门） | [AGENTS.md](../AGENTS.md) 的「六、请求受理」 |
| 单 agent：实施、本机回路、收口 | [workflow/single-agent.md](workflow/single-agent.md) |
| 多 agent：开工、派发、worktree、逐片 squash 集成、收口 | [workflow/multi-agent.md](workflow/multi-agent.md) |
| 用户启动文档审计 / 治理（检查、修改、验收） | [workflow/docs-audit.md](workflow/docs-audit.md) |
| 架构细则入口（分层、端口、状态、边界判据、注释契约、Rust 规范） | [architecture/README.md](architecture/README.md) |
| 文档分层、同步表、术语、引用格式 | [conventions.md](conventions.md) |
| Rust 源码布局、格式与静态检查、unsafe、依赖 | [architecture/rust-dev.md](architecture/rust-dev.md) |

## 改动时必须同步

- `docs/README.md` → 新增单元或文档：先在本页「单元」表登记入口，再从根门户 [AGENTS.md](../AGENTS.md) 链到它
- `docs/conventions.md` → 文档分层、同步表、术语与引用格式的唯一口径；判据改动同次改 `AGENTS.md` 与本页
- `AGENTS.md` → 根门户的文档路由表与本页的「单元」表同次增删