# 系统工具与角色（SYSTOOL）

> 定位：系统侧契约的门户：系统工具是什么、这一轮谁能用哪些、要改时读哪一份。
> 效力：索引｜核对：2026-11-04

## 真相源

| 要找什么 | 读哪一份 |
| --- | --- |
| 一条动作**是什么**：id / 说明 / 参数契约 / 能否并发 / 能力 / **callers（谁能调它）**；按文件域 / 核心操作 / 共享区版本化工作区 / 登记处动作 / 会话动作 / 代理工具 / 协作动词分段 | `systools/tools.yaml` |
| 系统工具与模块工具的分别：**系统工具**由核心实现（声明在上表的工具总表），**模块工具**由模块自己的外部命令实现；模块工具是动态动作（`module.<模块id>.<工具名>`），清单来自各模块 | `modules/*/module.yaml`；[MODULE_SPEC.md](MODULE_SPEC.md) |
| 这个**身份有什么**：引用的系统工具 id + 提示词 + 是否给模块工具 | `systools/roles.yaml` |
| 工作区布局**固定占用的目录名**（agent 实例名不得占用） | `systools/names.yaml` |
| 已确认但尚未实施的系统工具与验收条件，**不限角色**（非当前工具表、非测试缺口账） | [systool_gaps.yaml](systool_gaps.yaml) |
| 角色引用了表里不存在的 id 为什么硬失败、工具与角色的名单为什么在代码里没有第二份 | 装配期 `SystemTools::problems()`，由 `src/tests/detail/prompt.rs` 锁死 |

## 读哪一份

| 要找什么 | 读哪一份 |
| --- | --- |
| 工具面怎么合成、每回合给谁、越权怎么判 | [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md) 的「可用工具面」「越权校验」 |
| 核心操作为什么必须走工具调用、载荷形状 | [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md) 的「核心操作必须走工具调用」 |
| 提示词按角色怎么分配 | [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md) 的「提示词按角色分配」 |
| 路径模型（占位符 → 本环境写法、越界拒绝、编码、`@` 引用） | [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md) 的「路径模型」 |
| 回报与验收的语义（`submit_report` / `node_verdict` / `checklist` / `verdict`） | [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md) 的「可用工具面」；链的流程见 [docs/collab/task-chain.md](docs/collab/task-chain.md) |
| 围栏与能力等级（守门进程、平台后端、降级） | [ARCHITECTURE.md](ARCHITECTURE.md) 的「跨平台机制」 |
| 围栏落点判据、真机验收与探针 | [docs/kernel/README.md](docs/kernel/README.md) 的「围栏落点：必要与可选」；探针见 [TESTING.md](TESTING.md) |
| 模块工具怎么声明 | [MODULE_SPEC.md](MODULE_SPEC.md) |

## 联动

| 要找什么 | 读哪一份 |
| --- | --- |
| 工具与角色的**细则** | [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md) |
| 模块侧契约 | [MODULE_SPEC.md](MODULE_SPEC.md) |
| 会话、围栏与并发模型 | [ARCHITECTURE.md](ARCHITECTURE.md) |
| 产品行为（讨论 / 审查关卡 / 任务链 / 验收） | [PRODUCT.md](PRODUCT.md) |

## 改动时必须同步

- `systools/tools.yaml` → 加 / 改一个系统工具：唯一真相在这里，同次补实现、角色表按需引用与测试
- `systools/roles.yaml` → 改角色：同次同步 `prompts/roles/<角色>`（提示词与工具面同处声明，分开必然漂）
- `systools/names.yaml` → 加 / 改保留目录名：同次同步落盘布局文档 [docs/session/session-model.md](docs/session/session-model.md)
- `docs/tools/tools-and-roles.md` → 改路径模型 / 围栏口径：同次同步 [ARCHITECTURE.md](ARCHITECTURE.md)
- `src/tests/detail/prompt.rs` → 悬空引用由装配期 `SystemTools::problems()` 挡下，改表后保持它锁死
