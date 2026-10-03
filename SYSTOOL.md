# 系统工具与角色（SYSTOOL）

> 系统侧契约的**门户**：系统工具是什么、这一轮谁能用哪些、路径怎么给、回报与验收为什么是工具。
> 与模块工具的分别只有一处：**系统工具由核心实现**（声明在 `systools/tools.yaml`），
> **模块工具由模块自己的外部命令实现**（声明在 `module.yaml` 的 `tools`，见 [MODULE_SPEC.md](MODULE_SPEC.md)）。
> 两张表的字段、工具面合成与越权校验的唯一细则：[docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md)。

## 一、真相源

| 表 | 回答什么 | 位置 |
| --- | --- | --- |
| **工具总表** | 工具**是什么**：id / 说明 / 参数契约 / 能否并发 / 能力 | `systools/tools.yaml` |
| **角色表** | 这个**身份有什么**：引用的系统工具 id + 提示词 | `systools/roles.yaml` |
| **名字表** | 工作区布局**固定占用的目录名**（agent 实例名与它们撞路径，因此不得占用） | `systools/names.yaml` |
| **规划清单** | 已确认但尚未实施的核心代理系统工具与验收条件（非当前工具表、非测试缺口账） | [systool_gaps.yaml](systool_gaps.yaml) |

工具与角色的名单在代码里没有第二份：角色引用了表里不存在的 id = **结构审查硬失败**（不靠人看）。

## 二、三类系统工具

- **文件域**：`read` / `write` / `edit` / `patch` / `list` / `search`——碰文件系统，所以照样受沙箱与围栏约束；
  名字**保留**，模块的 `tools` 不得占用。
- **核心操作**：`plan`（方案 + 任务链）/ `node_verdict`（逐节点验收）/ `checklist`（总验收）/ `verdict`（裁决是否明确）/
  `slate`（名单）/ `submit_report`（执行席回报）/ `compact`（压缩上下文）——
  **凡会驱动核心的产出都必须是一次工具调用**，核心只从**工具参数**取载荷；正文里手写的同形 JSON 不作数。
- **代理工具**（`core_proxy` 专属，代理模式的前置契约）：`catalog_agents`（只读清单）/ `create_session`（建单 agent 或
  多 agent 代理会话）/ `send_session_message`（代用户转达）/ `observe_session`（只读观察）/ `control_session`（暂停、恢复、
  停止、关闭）——工具逻辑已落地；真实会话宿主与核心代理生成循环尚未落地（见 `src/capabilities/conductor/testgaps.yaml`）。
- **协作动词**：`say` / `agree` / `leave` / `ask`——讨论阶段的表态，是**声明式工具**：两套通道产出同语义，
  没有表态的纯正文 = 这一轮**没表态**（不降级成普通发言）。

## 三、这一轮能用哪些

工具面按**身份**逐回合合成，**总表不进提示词**：

```text
本回合可用工具 = 系统工具表 ∩ 该角色表  +  该成员所属模块的工具（角色表 module_tools: true 时）
```

讨论席只做核实（读类 + 讨论动词）；执行席才动手，且**按身份分两种**：
用户建的单 agent 工作用 `solo`（文件域 + 它自己的模块工具），协作的节点子会话用 `executor`（多一个回报工具 `submit_report`）；
核心代理回合用 `core_proxy`（五项代理工具 + 只读核实，`module_tools: false`）。
列出来的就是这一刻真能调的；真去调没拿到的会被**如实拒绝并落一条工具行**（不静默）。

## 四、路径模型：AI 只看到真实绝对路径

仓库里（代码、提示词册、`module.yaml`、文档）一律只用**占位符**，运行时由核心取真实目录替换进提示词——
仓库永不出现机器路径；系统工具与模块自带的外部工具**用同一套路径语言**：

| 占位符 | 运行时替换为 | 可达范围 |
| --- | --- | --- |
| `{{work_root}}` | 本次工作共享区 `session/<工作名>/work/` 的绝对路径 | 本工作内的 agent |
| `{{sandbox_root}}` | 该 agent 私有沙箱 `session/<工作名>/<agent实例名>/` 的绝对路径 | 只有它自己 |
| `{{module_roots}}` | 该 agent 各模块目录的绝对路径（一行一个） | 只有该模块所属的 agent |

- **越界即拒绝**：路径必须是列出的真实根**之下**的绝对路径；相对路径、`..` 跳出、不在任何根之内一律拒绝，
  并把允许的根列回去（如实报错，不纠正）。
- **编码**：读严格 UTF-8（非法字节按替换字符呈现并如实标注——**本程序不猜编码**）；写一律 UTF-8。
- **用户也能引用**：输入框里用 `@` 挑文件（`@work:相对路径` / `@sandbox:<agent>/相对路径`，**给人用**），
  核心替换成真实绝对路径之后才进转录与上下文；引用别人的私有沙箱时如实说明无权读取，且不泄漏对方的真实路径。

## 五、围栏与能力等级

系统工具与模块工具**走同一条执行路径**：进程统一从**守门进程**里跑（环境白名单、进程树围栏、超时连根杀、按平台的文件系统围栏）。
可达范围 = 本次工作的共享区 + 该 agent 私有沙箱 + 它自己的模块目录，**这些目录与它们的直接父目录可以判断存在性**
（`exists` / `stat` 得到真实结果），所以"产物目录不存在就先建"这类写法在围栏里也能正常工作。
机制装不上就**如实报告能力等级**，不假装有（三级隔离见 [PRODUCT.md](PRODUCT.md)）。

## 六、回报与验收（都由工具承载）

- **执行席回报**：`submit_report`（summary / changes / open）——它就是这一轮的**最终答复**，不是正文里的 JSON 块。
  理由：回报会驱动核心判节点完成，属于核心操作，必须带 schema 校验并进工具台账。
- **节点验收**：`node_verdict`（逐节点 `ok` / `note`）——`ok=false` 的节点**退回待办**，
  用户点「继续」后**只重派它们**（同阶段其余节点保持已通过，不整阶段重来）。
- **总验收**：`checklist`（逐条 `pass` / `fail`）——`fail` **必须**填 `rework` = 要返工的**节点 id**；
  填了表里没有的 id 或漏填 = 这次判定不能用，核心**要求重填**。
- **裁决**：`verdict.clear = true` 才照用户说的开工 / 放行；含糊的回应不算明确。

## 七、改动时必须同步

- 加 / 改一个系统工具 → `systools/tools.yaml`（唯一真相）+ 它的实现 + 角色表按需引用 + 测试；
- 改角色 → `systools/roles.yaml` + `prompts/roles/<角色>`（**提示词与工具面同处声明**，分开必然漂）；
- 加 / 改保留目录名 → `systools/names.yaml`（唯一真相）+ 同步落盘布局文档（[PRODUCT.md](PRODUCT.md) 的「工作的落盘与沙箱」与 [docs/session/session-model.md](docs/session/session-model.md)）；
- 改路径模型或围栏口径 → 本文 + [ARCHITECTURE.md](ARCHITECTURE.md)（§六 状态与落盘）+ [docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md)；
- 悬空引用（角色引用表里没有的 id）由 `node run-tests.js` 的结构审查挡下。

## 八、联动

- 工具与角色的**细则**：[docs/tools/tools-and-roles.md](docs/tools/tools-and-roles.md)
- 模块侧契约（怎么声明一个模块工具）：[MODULE_SPEC.md](MODULE_SPEC.md)
- 会话、围栏与并发模型：[ARCHITECTURE.md](ARCHITECTURE.md)；
- 产品行为（讨论 / 审查关卡 / 任务链 / 验收）：[PRODUCT.md](PRODUCT.md)
