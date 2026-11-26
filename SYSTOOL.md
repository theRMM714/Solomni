# 系统工具、流程段与角色（SYSTOOL）

> 定位：系统侧契约的门户：系统工具是什么、这一轮谁能用哪些、准不准用、要改时读哪一份。
> 效力：索引｜核对：2026-11-24

## 真相源

| 要找什么 | 读哪一份 |
| --- | --- |
| 一条动作**是什么**：id / 说明 / 参数契约 / 能否并发；按文件域 / 核心操作 / 共享区版本化工作区 / 登记处动作 / 会话动作 / 代理工具 / 协作动词分段（**谁能拿到它不在这张表里**，由工具来源各自交 id） | `systools/tools.yaml` |
| 系统工具与模块工具的分别：**系统工具**由核心实现（声明在上表的工具总表），**模块工具**由模块自己的外部命令实现；模块工具是动态动作（`module.<模块id>.<工具名>`），清单来自各模块 | `modules/*/module.yaml`；[MODULE_SPEC.md](MODULE_SPEC.md) |
| 这个**身份有什么**：哪个角色（核心 / 成员）+ 提示词 + 它在这一段自己交的机制动作 id | `src/units/business/session_model/domain/mechanism.rs` 的 `ROLES` 与 `LOOPS` 的 `cast` |
| 工作区布局**固定占用的目录名**（agent 实例名不得占用） | `systools/names.yaml` |
| 一个**会话类型**的 id 与显示名、按什么顺序跑哪些流程、是否整块交给核心（`auto`）、agent 能不能建它（`agent_visible`）、挂哪几个**工具来源**（`sources`）、带哪些**系统工具**（`system_tools`）、（没有流程的类型）坐哪个**静止段** | `systools/profiles.yaml` |
| 一个**流程段**的角色位：哪些角色出演、各要几员、各交哪些机制动作、抑制哪几组能力 | `src/units/business/session_model/domain/mechanism.rs` 的 `LOOPS`（`flow: true` 的才是流程） |
| 面里有的工具**这一刻准不准用** | 授权端口 `src/units/business/agentloop/domain/policy.rs`（默认实现按流程段的 `deny` 判） |
| 表里没有的会话类型为什么建不出来、agent 能建哪些类型 | 形状只由类型表定（`src/units/business/work/service/create.rs`）；agent 只看 `agent_visible`（`catalog_agents` 的 `scope=types` 也照此过滤） |
| 已确认但尚未实施的系统工具与验收条件，**不限席位**（非当前工具表、非测试缺口账） | [systool_gaps.yaml](systool_gaps.yaml) |
| 角色位或会话类型引用了表里不存在的 id 为什么硬失败 | 装配期 `loop_check::problems(&ToolBook)` 与 `SystemTools::problems()`，由 `src/tests/detail/prompt.rs` 锁死 |

## 读哪一份

| 要找什么 | 读哪一份 |
| --- | --- |
| 工具面怎么合成、每回合给谁、越权怎么判 | [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md) 的「可用工具面」「越权校验」 |
| 核心操作为什么必须走工具调用、载荷形状 | [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md) 的「核心操作必须走工具调用」 |
| 提示词按席位怎么分配 | [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md) 的「提示词按席位分配」 |
| 路径模型（占位符 → 本环境写法、越界拒绝、编码、`@` 引用） | [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md) 的「路径模型」 |
| 回报与验收的语义（`submit_report` / `node_verdict` / `checklist` / `verdict`） | [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md) 的「可用工具面」；链的流程见 [docs/collab/task-chain.md](docs/collab/task-chain.md) |
| 围栏与能力等级（守门进程、平台后端、降级） | [ARCHITECTURE.md](ARCHITECTURE.md) 的「跨平台机制」 |
| 围栏落点判据、真机验收与探针 | [docs/kernel/README.md](docs/kernel/README.md) 的「围栏落点：必要与可选」；探针见 [TESTING.md](TESTING.md) |
| 模块工具怎么声明 | [MODULE_SPEC.md](MODULE_SPEC.md) |

## 联动

| 要找什么 | 读哪一份 |
| --- | --- |
| 工具与机制席位的**细则** | [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md) |
| 模块侧契约 | [MODULE_SPEC.md](MODULE_SPEC.md) |
| 会话、围栏与并发模型 | [ARCHITECTURE.md](ARCHITECTURE.md) |
| 产品行为（讨论 / 审查关卡 / 任务链 / 验收） | [PRODUCT.md](PRODUCT.md) |

## 改动时必须同步

- `systools/tools.yaml` → 加 / 改一个系统工具：唯一真相在这里，同次补实现、让该发它的**来源**交出它的 id（或把它加进角色位的机制动作 / 会话类型的系统工具）与测试
- `src/units/business/session_model/domain/mechanism.rs` → 改角色 / 流程段（`ROLES` / `LOOPS`）：同次同步 `prompts/roles/<角色>`（提示词与机制动作同处声明）与 `systools/tools.yaml`
- `systools/names.yaml` → 加 / 改保留目录名：同次同步落盘布局文档 [docs/session/session-model.md](docs/session/session-model.md)
- `systools/profiles.yaml` → 改会话类型 / 流程顺序：同次确认流程段已在 `src/units/business/session_model/domain/mechanism.rs` 的 `LOOPS` 注册（角色位与人数在那边），并同步机制册 `prompts/shared/mechanisms.yaml` 的条目（装配期校验，见 [docs/session/session-model.md](docs/session/session-model.md)）
- `docs/tools/tools-and-roles.md` → 改路径模型 / 围栏口径：同次同步 [ARCHITECTURE.md](ARCHITECTURE.md)
- `src/tests/detail/prompt.rs` → 悬空引用由装配期 `loop_check::problems()` 挡下，改表后保持它锁死
