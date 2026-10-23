# 登记处契约（REGISTRY_SPEC）

> 本文只讲**登记处**：`.home/` 下四份 yaml 长什么样、谁读谁写、缺字段与非法输入怎么办、密钥的边界在哪。
> 模块契约见 [MODULE_SPEC.md](MODULE_SPEC.md)，运行包契约见 [RUNTIME_SPEC.md](RUNTIME_SPEC.md)，
> 分层与端口见 [ARCHITECTURE.md](ARCHITECTURE.md)，产品行为见 [PRODUCT.md](PRODUCT.md)。

## 设计立场

**登记处是产品级资源，不是模块资源；密钥归这里，模型选择是会话级决定。**

- 四份 yaml 都由用户经界面（终端或本地网页的设置）管理，落在 `.home/`——用户私有区、git 忽略。
- 模块与工具**只持 id 引用**，永不接触通道与密钥（见 [MODULE_SPEC.md](MODULE_SPEC.md)「供应商与密钥」）。
- 登记处是**内存事实的落盘形态**：结构定义在 `capabilities/registry/domain/`（`providers.rs` 与 `agents.rs`），
  读写机制在 `capabilities/registry/detail/yaml_settings.rs`（`SettingsStore` 端口）。
- 核心只校验，不纠正：解析失败 = 如实报错（装配失败），不静默兜底。

## 一、文件与职责

| 文件 | 内容 | 缺文件时 |
| --- | --- | --- |
| `providers.yaml` | 供应商通道：端点 + 密钥 | 空登记处 |
| `models.yaml` | 模型清单 + 核心默认模型 | 空登记处、无默认模型 |
| `agents.yaml` | 用户配置的具名 agent | 空登记处 |
| `settings.yaml` | 基本设置（开关与默认档位） | 全部取默认值 |

四份文件**分开读写**：`load` 逐份读，缺哪份就用哪份的默认；`save` 会同时写四份。
yaml 非法（解析失败）= 装配失败并指出是哪一份。

## 二、providers.yaml

```yaml
providers:
  <供应商 id>:
    base_url: https://...      # 端点（必填，非空）
    api_key: sk-...            # 密钥（必填，非空）
```

- 供应商 id 是模型引用它的键；`id` 与 `base_url` 不能为空。
- 更新供应商时 `api_key` 留空 = **保留原密钥**（界面从不回显密钥）；没有旧值则报错。
- 删除供应商时若仍有模型引用它 = **拒绝**并列出引用者，不静默级联删除。
- 密钥只落这一份文件（unix 下 0600；drvfs/NTFS 上 chmod 无效属平台限制，如实告知不隐瞒）。

## 三、models.yaml

```yaml
models:
  <模型 id>:
    name: 展示名                # 给人看（必填）
    api_model: 实际模型串        # 发给供应商的那个（必填）
    provider: <供应商 id>        # 必填，必须已存在
    note: ""                    # 能力说明（可选）
    tools: envelope             # 工具调用形态（可选；缺省 envelope）。只认这两个值，别的一律拒收
core: <模型 id>                 # 核心默认模型（可选）
```

- `provider` 必须指向 `providers.yaml` 里的一个 id，否则拒绝新增。
- `tools` 是**该通道用什么协议调工具**，只认两个值：
  - `envelope`（缺省）：模型在正文里写手写信封，核心解析——**任何供应商都能用**，也是缺省值；
  - `native`：用供应商原生的工具调用（参数走结构化槽位）——**需要该通道确实支持 function calling**。
- **探测**（把"支不支持"变成事实，而不是靠猜）：命令行跑 `solomni --probe-tools <模型 id>`，
  适配层发**两条最小请求**对比——不带工具声明的、带工具声明的：
  - 不带就失败 → 如实报"通道本身没打通"（**不**当成"不支持工具调用"）；
  - 带工具声明被拒（附供应商原话）→ 判**不支持**，并把 `tools` 写回 `envelope`；
  - 返回了工具调用 → 判**支持**，并把 `tools` 写回 `native`；
  - 带了声明但这次没发起调用 → 判**无法判定**，只报事实、**不改**登记处（由你决定填哪个）。
  只有确凿的结论才会写回 `models.yaml`；无法判定时绝不替用户拍板。
  - **回放形状探测**（工具调用历史怎么发回供应商才收）：命令行跑 `solomni --probe-replay <模型 id>`。
    适配层把同一件事的几种写法各发一次——基线是**现在线上真在用的**文本回放，其余是 `assistant` 带 `tool_calls` +
    `role=tool` 结果的几种可判定变体（正文空串 / null / 有正文 / 结果消息带 name）——逐项报两件事：
    **供应商收了没有**（HTTP 层），以及**模型真的读到了没有**（回答里带回工具结果里那个编号才算读懂；
    收了不等于看懂了）。它**不改登记处**：只报事实，采不采用由人定。
    界面上：模型登记里那两行按钮（**测工具调用** / **测回放形状**）走的就是这两个探测
    （`model.act` 的 `probe` / `probe-replay`），结论逐项显示。
  **两种形态互斥**：一个通道走 native 就只认原生调用，正文里的信封不会被当工具执行
  （否则模型在正文里讲解参数就会被误当成调用），但会如实记一条失败的工具行。
  声明 native 而通道实际不支持时**如实报错**，不静默降级成信封（那会让模型在不知道信封格式的情况下空转）。
- 删除模型时若它是核心默认模型 = 拒绝（先改默认再删）。
- `core` 未设定或解析失败 = 核心通道回落演示通道，并如实告知（不静默）。
- 「模型 → 供应商」的绑定只对本产品可见；会话只记模型 id 引用。

## 四、agents.yaml

```yaml
agents:
  <agent 名>:
    modules: [research, reviewer]   # 成员模块（可以为空：零模块 agent 只用内置文件工具）
    model: <模型 id>                # 默认模型（可选；缺省 = 用核心默认）
    note: ""                        # 一句话说明（可选）
```

- agent 只是「用户编排选择的存档」，不是能力注册表——能力仍在 `modules/` 公地（见 [PHILOSOPHY.md](PHILOSOPHY.md)）。
- 同一个模块可以在一次工作里被多个 agent 共用（各自独立沙箱与调用）；每个 agent 的模块列表按出现顺序去重，`meta.modules` 是去重后的扁平清单。
- agent 名会成为会话内的沙箱目录名，校验比 id 严（见「六、名字与 id」）。

## 五、settings.yaml

```yaml
streaming: true        # 流式传输：向供应商请求逐片返回（默认 true）
show_reasoning: true   # 思维链显示（默认 true；界面上永远默认折叠、点击才展开）
tier: host             # 默认执行档位：host（本机）| vm（虚拟机）（默认 host）
sandbox:               # 全局沙箱策略：会话不可填；保存后整体重启
  policy:
    network: false     # 全局网络开关：会话 / 直接模块工具 / 常驻服务 / 构建共用
    enforce:           # 哪些围栏必须强制（require）；best_effort 表示不可用时降级并如实报告
      filesystem: require
      network: require
      process_tree: require
      env_whitelist: require
    on_unavailable: ask # ask = 启动时询问是否本进程降级（不落盘）；refuse = 直接拒绝
    windows:
      write_acl: false # 是否允许 Windows 容器围栏写本机目录 ACL
    host_read_roots: [] # 用户显式授权的主机绝对路径只读根（默认空）
    deny: []           # 全局黑名单（工作区相对路径）：硬底线，会话只能追加、不能删除
  defaults:            # 会话权限的全局默认；会话 meta 可覆盖
    allow_read: []     # 读白名单（相对工作区根；空 = 整棵工作区可读）
    allow_write: []    # 提交白名单（相对工作区根；空 = 整棵工作区可提交）
    granularity: ask   # 决定粒度：ask = ask 表里的工具要用户放行；full = 任何工具都不设确认
    ask: []            # granularity=ask 时要用户放行的工具名（模块工具写 module.tool）
qemu_path: ""          # 虚拟机档用的 QEMU 可执行文件路径（默认空 = 判据每次取用时采用项目内补齐的可运行包；否则兜底看 PATH。显式登记优先）
virtiofsd_path: ""     # 虚拟机档共享后端要用的外部程序路径（如 virtiofsd；默认同 qemu_path：登记优先，否则用项目内补齐的那个）
smbd_path: ""          # 虚拟机档 smb 共享档要用的外部 helper 路径（默认空 = 判据每次取用时采用项目内补齐的 helper；否则兜底看 PATH）
max_guests: 2          # 虚拟机档同时最多几台 guest（0 = 不限制；每台固定 -m 2048 -smp 2）
llm_timeout_secs: 300  # 单次模型调用的总预算（秒），全局通用（讨论/执行/验收/单 agent）
compact_at_percent: 70 # 上下文用到模型窗口的百分之多少就自动压一次（0 = 关）
discuss_remind_cap: 3  # 讨论里一轮内对同一个成员最多提醒几次（提醒 = "你还没用动词表态"）
```

- `sandbox.policy` 是**全局硬约束**：会话、子会话、agent 都不能覆盖；改它要保存后整体重启，
  让所有 guest / 常驻服务 / 工具进程按新策略重建。配置写错（拼错键、未知键、相对的主机只读根、非法路径）启动即报 `settings.yaml 配置非法`。
- `sandbox.defaults` 是**会话权限的全局默认**；会话级覆盖写在 `meta.yaml` 的 `sandbox.permissions`
  （见 [docs/session/session-model.md](docs/session/session-model.md)）。`sandbox.policy.deny` 是硬底线，
  会话只能追加黑名单，不能清空。语义只有一份权威：[docs/permission/README.md](docs/permission/README.md)。
- **网络是全局开关**（`sandbox.policy.network`）：会话工具、直接模块工具、常驻服务、模块构建共用；
  provisioning 首启与核心模型出站不受它影响。
- `on_unavailable=ask` 时，启动预检发现 `require` 的机制不可用会询问是否**本进程降级**；
  回答不落盘，重启再问；没人答 / 没有前端 / `refuse` = fail-closed。`SOLOMNI_SANDBOX_DEGRADED=1/0`
  只供测试与 CI 在进程级注入（`1` 仅在 `ask` 下生效）。
- `sandbox.policy.windows.write_acl` 可被环境变量 `SOLOMNI_FENCE_WRITE=1/0` 临时覆盖（优先级高于设置，测试与 CI 用得到）；
  它只影响 Windows 的目录 ACL。
- `sandbox.policy.host_read_roots` 是**用户显式授权的主机绝对路径只读根**（默认空）：工具进程对这些目录只读可达，
  不继承写；授权落在用户自己的目录上。撤权与 `write_acl` 走同一份台账（`--fence-clean` 一并撤净；
  启动期按归属对账回收陈旧授权，见 [ARCHITECTURE.md](ARCHITECTURE.md) 六）。
- `llm_timeout_secs` 是**单次模型调用的总预算**，全局通用：连接之外，等响应头 / 读响应体 / 整体都用它。
  **为什么是一个预算**：非流式下供应商要等整段生成完才发响应头，单独设一个小的「头超时」会把几十秒的正常长回复误判成不通。
  预算用尽 = **中断**这一轮并如实告知（用户可以点「继续」重试），不是把会话作废。
- **模型调用次数没有上限**：工具循环跑到模型不再发起调用为止（执行席与讨论席一样），
  用户随时可以点「停止」。`discuss_remind_cap` 只是**提醒次数**：提醒到顶就如实记一行"未回应"放过它，整轮继续。

## 六、名字与 id

- 供应商 id / 模型 id：登记处内的映射键，非空且唯一（yaml 映射天然保证唯一）。
- **agent 名**会变成目录名，因此校验更严：非空、首尾无空白、不含 `\ / : * ? " < > |` 与控制字符、
  不是 `.` 与 `..`、不是 Windows 保留名（`CON` / `PRN` / `AUX` / `NUL` / `COM1..9` / `LPT1..9`）。
- 同一工作内 agent 重名 = 核心自动追加 `-2`、`-3`（用户不改名时的兜底），并先让用户裁决是否改名。

## 七、安全边界

- 通道密钥只在两处存在：`providers.yaml` 与核心发起的出站调用。
  **永不**进入提示词、转录、日志、模块工作区。
- 模块隐秘字段的值只存在 `.home/secrets.yaml`（`capabilities/secrets` 持有）；核心按模块作用域经环境变量注入该模块自己的工具 / 服务进程——
  **值不进**提示词、转录、日志、模块工作区与命令行。字段的声明与注入名在 `module.yaml` 的 `secrets:`（见 [MODULE_SPEC.md](MODULE_SPEC.md)）。
- 界面与报告只呈现「供应商 id + 端点」与「模型 id + 展示名」，**永不携带密钥**。
- 出站错误信息经脱敏后才出适配层；日志与转录里出现的永远是 id，不是密钥。
