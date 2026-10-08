# 系统工具与角色（SYSTOOL）

> 系统侧契约的**门户**：只回答"系统工具是什么、这一轮谁能用哪些、要改时读哪一份"。
> **细则**在 [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md)。与模块工具的分别只有一处：
> 系统工具由核心实现（声明在 `systools/tools.yaml`），模块工具由模块自己的外部命令实现（见 [MODULE_SPEC.md](MODULE_SPEC.md)）。

## 一、真相源

| 表 | 回答什么 | 位置 |
| --- | --- | --- |
| **动作表**（工具总表） | 一条动作**是什么**：id / 说明 / 参数契约 / 能否并发 / 能力 / **callers（谁能调它）**；按文件域 / 核心操作 / 会话动作 / 代理工具 / 协作动词分段。**模块工具是动态动作**（`module.<模块id>.<工具名>`），清单来自各模块 | `systools/tools.yaml` + `modules/*/module.yaml` |
| **角色表** | 这个**身份有什么**：引用的系统工具 id + 提示词 + 是否给模块工具 | `systools/roles.yaml` |
| **名字表** | 工作区布局**固定占用的目录名**（agent 实例名不得占用） | `systools/names.yaml` |
| **规划清单** | 已确认但尚未实施的系统工具与验收条件，**不限角色**（非当前工具表、非测试缺口账） | [systool_gaps.yaml](systool_gaps.yaml) |

工具与角色的名单在代码里没有第二份：角色引用了表里不存在的 id = **结构审查硬失败**（不靠人看）。

## 二、读哪一份

| 要了解 | 读这一份 |
| --- | --- |
| 工具面怎么合成、每回合给谁、越权怎么判 | [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md) 的「可用工具面」「越权校验」 |
| 核心操作为什么必须走工具调用、载荷形状 | 同上的「核心操作必须走工具调用」 |
| 提示词按角色怎么分配 | 同上的「提示词按角色分配」 |
| 路径模型（占位符 → 真实绝对路径、越界拒绝、编码、`@` 引用） | 同上的「路径模型」 |
| 回报与验收的语义（`submit_report` / `node_verdict` / `checklist` / `verdict`） | 同上的「可用工具面」；链的流程见 [docs/collab/task-chain.md](docs/collab/task-chain.md) |
| 围栏与能力等级（守门进程、平台后端、降级） | [ARCHITECTURE.md](ARCHITECTURE.md) 的「跨平台机制」；探针见 [TESTING.md](TESTING.md) |
| 模块工具怎么声明 | [MODULE_SPEC.md](MODULE_SPEC.md) |

## 三、改动时必须同步

- 加 / 改一个系统工具 → `systools/tools.yaml`（唯一真相）+ 实现 + 角色表按需引用 + 测试；
- 改角色 → `systools/roles.yaml` + `prompts/roles/<角色>`（提示词与工具面同处声明，分开必然漂）；
- 加 / 改保留目录名 → `systools/names.yaml` + 落盘布局文档（[docs/session/session-model.md](docs/session/session-model.md)）；
- 改路径模型 / 围栏口径 → [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md) 与 [ARCHITECTURE.md](ARCHITECTURE.md)；
- 悬空引用由 `node run-tests.js` 的结构审查挡下。

## 四、联动

- 工具与角色的**细则**：[docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md)
- 模块侧契约：[MODULE_SPEC.md](MODULE_SPEC.md)；会话、围栏与并发模型：[ARCHITECTURE.md](ARCHITECTURE.md)
- 产品行为（讨论 / 审查关卡 / 任务链 / 验收）：[PRODUCT.md](PRODUCT.md)
