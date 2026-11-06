# 登记处契约（REGISTRY_SPEC）
> 定位：`.home/` 下四份 yaml 的格式与字段、缺字段与非法输入怎么办、密钥边界在哪；不回答实现与探测行为。
> 效力：契约｜核对：2026-11-04｜依据：`src/capabilities/registry/domain/providers.rs`

## 结构
```text
.home/
  providers.yaml   # 供应商通道：端点 + 密钥
  models.yaml      # 模型清单 + 核心默认模型
  agents.yaml      # 用户配置的具名 agent
  settings.yaml    # 基本设置（开关与默认档位）
```
- 登记处是产品级资源，不是模块资源；四份 yaml 都由用户经界面（终端或本地网页的设置）管理，落在 `.home/`——用户私有区、git 忽略。
- 四份文件分开读写：`load` 逐份读，缺哪份就用哪份的默认；`save` 会同时写四份。
- yaml 非法（解析失败）= 装配失败并指出是哪一份；核心只校验，不纠正，不静默兜底。
- 模块与工具只持 id 引用，永不接触通道与密钥（见 `MODULE_SPEC.md`「隐秘字段」）。
- 会话只记模型 id 引用；「模型 → 供应商」的绑定只对本产品可见。

## 一、文件与职责
| 字段 | 类型 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- | --- |
| `providers.yaml` | 文件 | 否 | 空登记处 | 供应商通道：端点 + 密钥 |
| `models.yaml` | 文件 | 否 | 空登记处、无默认模型 | 模型清单 + 核心默认模型 |
| `agents.yaml` | 文件 | 否 | 空登记处 | 用户配置的具名 agent |
| `settings.yaml` | 文件 | 否 | 全部取默认值 | 基本设置（开关与默认档位） |
- 缺文件时按上表取默认；yaml 非法 = 装配失败并指出是哪一份。
- 写面（登记 / 移除 / 设核心默认 / 探测 / 改设置）以用户动作暴露，默认不开放给任何角色——供应商与密钥是产品级资源。

## 二、providers.yaml
| 字段 | 类型 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- | --- |
| `<供应商 id>` | 映射 | 否 | 空 | 供应商 id → 通道 |
| `base_url` | 字符串 | 是 | - | 端点（非空） |
| `api_key` | 字符串 | 是 | - | 密钥（非空） |
- 供应商 id 是模型引用它的键；`id` 与 `base_url` 不能为空。
- 更新供应商时 `api_key` 留空 = 保留原密钥（界面从不回显密钥）；没有旧值则报错。
- 删除供应商时若仍有模型引用它 = 拒绝并列出引用者，不静默级联删除。
- 密钥只落这一份文件（unix 下 0600；drvfs/NTFS 上 chmod 无效属平台限制，如实告知不隐瞒）。

## 三、models.yaml
| 字段 | 类型 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- | --- |
| `<模型 id>` | 映射 | 否 | 空 | 模型 id → 模型 |
| `name` | 字符串 | 是 | - | 展示名（给人看） |
| `api_model` | 字符串 | 是 | - | 发给供应商的实际模型串 |
| `provider` | 字符串 | 是 | - | 供应商 id，必须已存在 |
| `note` | 字符串 | 否 | 空 | 能力说明 |
| `tools` | 字符串 | 否 | `native` | 工具调用形态；只认 `envelope` / `native` |
| `thinking.default` | 字符串 | 否 | 无 | 默认思考档；必须是 `thinking.levels` 里的一个 |
| `thinking.levels` | 映射 | 否 | 无 | 档位 → 请求体片段；键必须在 `settings.yaml` 的 `thinking.levels` 里 |
| `core` | 字符串 | 否 | 无 | 核心默认模型 id |
- `provider` 必须指向 `providers.yaml` 里的一个 id，否则拒绝新增。
- `tools` 是**该通道用什么协议调工具**：`native`（缺省）用供应商原生的工具调用（参数走结构化槽位）——保存前必须实测通过；`envelope` 让模型在正文里写手写信封、核心解析——仅在模型表单里**显式选择**时使用，保存前不测试。
- 两种形态互斥：一个通道走 native 就只认原生调用，正文里的信封不会被当工具执行（否则模型在正文里讲解参数就会被误当成调用），但会如实记一条失败的工具行。
- **原生形态的保存闸门**：保存前实测「通道基线 + 带 tools 的最小调用 + 实际回放形状」；只有明确支持且回放形状被接受才落盘。明确不支持 / 无法判定 = **不保存**，只如实提示原因，不静默降级成信封、也不引导改用手写信封（用户可自己把表单改成信封再保存）。
- 信封形态是用户的显式选择：**不测试，直接保存**；通道是否可用由使用者自己负责。
- `thinking` 是出站请求的思考强度映射：字符串值 = `{"reasoning_effort": 值}` 简写；对象值 = 原样合并；空对象 = 这一档不发任何字段。
- `models.yaml` 的 `thinking.levels` 片段不得覆盖协议字段（`model` / `messages` / `stream` / `stream_options` / `tools` / `tool_choice`），保存时拒收。
- 解析优先级：会话选的档 > 模型 `default`；两者都没有 = 不发；会话档在模型里没登记 = 下一次生成如实报错（不静默降级）。
- 默认四档 `off / low / high / max` 只是目录缺省；每家供应商的取值不同（DeepSeek 用 `reasoning_effort: none` 关，GLM 用 `thinking.type=disabled`），所以映射放在模型上而不是代码里。
- 删除模型时若它是核心默认模型 = 拒绝（先改默认再删）。
- `core` 未设定或解析失败 = 建会话 / 核心操作如实拒绝并指明缺什么（不回落假模型）。
- 探测（`--probe-tools`）与回放探测（`--probe-replay`）的行为、结论判定与写回规则见 `docs/registry/README.md`。

## 四、agents.yaml
| 字段 | 类型 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- | --- |
| `<agent 名>` | 映射 | 否 | 空 | agent 名 → 存档 |
| `modules` | 字符串数组 | 否 | `[]` | 成员模块；可以为空（零模块 agent 只用内置文件工具） |
| `model` | 字符串 | 否 | 无 | 默认模型；缺省 = 用核心默认 |
| `note` | 字符串 | 否 | 空 | 一句话说明（给人看） |
| `chara` | 字符串 | 否 | 空 | 人设 / 身份（给模型看，可多行） |
- agent 只是「用户编排选择的存档」，不是能力注册表——能力仍在 `modules/` 公地（见 `PHILOSOPHY.md`）。
- `note` 与 `chara` 是两件事：前者给人看（列表 / 推荐理由），后者进这个 agent 的身份提示词；`chara` 为空时不产生人设块。
- 建会话时把 agent 的组合（名字 / 模块 / 模型 / 人设）取一份 copy 落进 `meta.yaml` 的 `agents`；之后改登记处不影响已开会话，会话配置里改的也只是这份 copy。
- 同一个模块可以在一次工作里被多个 agent 共用（各自独立沙箱与调用）；每个 agent 的模块列表按出现顺序去重，`meta.modules` 是去重后的扁平清单。
- agent 名会成为会话内的沙箱目录名，校验比 id 严（见「六、名字与 id」）。

## 五、settings.yaml
| 字段 | 类型 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- | --- |
| `streaming` | 布尔 | 否 | `true` | 流式传输：向供应商请求逐片返回 |
| `show_reasoning` | 布尔 | 否 | `true` | 思维链显示（界面上永远默认折叠、点击才展开） |
| `thinking.levels` | 数组 | 否 | `off / low / high / max` | 全局思考强度目录；标签顺序即界面顺序 |
| `tier` | 字符串 | 否 | `host` | 默认执行档位：`host`（本机）｜`vm`（虚拟机） |
| `sandbox.policy.network` | 布尔 | 否 | `false` | 全局网络开关：会话 / 直接模块工具 / 常驻服务 / 构建共用 |
| `sandbox.policy.enforce.filesystem` | 字符串 | 否 | `require` | 文件系统围栏：`require` / `best_effort` |
| `sandbox.policy.enforce.network` | 字符串 | 否 | `require` | 网络围栏：`require` / `best_effort` |
| `sandbox.policy.enforce.process_tree` | 字符串 | 否 | `require` | 进程树围栏：`require` / `best_effort` |
| `sandbox.policy.enforce.env_whitelist` | 字符串 | 否 | `require` | 环境白名单：`require` / `best_effort` |
| `sandbox.policy.on_unavailable` | 字符串 | 否 | `ask` | `ask` = 启动门询问是否本进程降级（不落盘）；`refuse` = 直接拒绝 |
| `sandbox.policy.windows.write_acl` | 布尔 | 否 | `false` | 是否允许 Windows 容器围栏写本机目录 ACL（首次启动会引导） |
| `sandbox.policy.host_read_roots` | 字符串数组 | 否 | `[]` | 用户显式授权的主机绝对路径只读根 |
| `sandbox.policy.deny` | 字符串数组 | 否 | `[]` | 全局黑名单（工作区相对路径）：硬底线，会话只能追加 |
| `sandbox.defaults.profile` | 字符串 | 否 | `workspace` | 默认档位：`view` / `workspace` / `external` / `all_reach` |
| `sandbox.defaults.shared.read` | 字符串／数组 | 否 | `all` | 共享区读范围三态：`all` / `none` / 路径数组 |
| `sandbox.defaults.shared.write` | 字符串／数组 | 否 | `all` | 共享区写范围三态 |
| `sandbox.defaults.shared.deny` | 字符串数组 | 否 | `[]` | 默认追加黑名单；全局 `policy.deny` 仍是硬底线 |
| `sandbox.defaults.tools` | 映射 | 否 | `{}` | 逐工具决定：工具名 → `allow` / `ask` / `deny` |
| `qemu_path` | 字符串 | 否 | 空 | 虚拟机档 QEMU 可执行文件路径；空 = 取项目内补齐的那个，否则看 PATH |
| `virtiofsd_path` | 字符串 | 否 | 空 | 虚拟机档共享后端的外部程序路径；口径同 `qemu_path` |
| `smbd_path` | 字符串 | 否 | 空 | 虚拟机档 smb 共享档的 helper 路径；口径同 `qemu_path` |
| `max_guests` | 整数 | 否 | `2` | 虚拟机档同时最多几台 guest（0 = 不限制；每台固定 `-m 2048 -smp 2`） |
| `storage.periodic_gc` | 布尔 | 否 | `true` | 启用全局周期回收；构建前 headroom 始终启用 |
| `storage.gc_interval_minutes` | 整数 | 否 | `15` | 全局自动回收间隔（分钟，至少 15） |
| `storage.cache_cap_gib` | 整数 | 否 | `20` | 可回收缓存总预算（GiB）；0 = 按磁盘 10%，夹 10–50 GiB |
| `storage.cache_keep_days` | 整数 | 否 | `30` | 超过多久未使用可回收（天）；0 = 不按龄 |
| `storage.build_volume_gib` | 整数 | 否 | `16` | 新构建缓存卷的虚拟大小（GiB）；现有卷不自动缩 |
| `storage.min_free_gib` | 整数 | 否 | `2` | 宿主剩余低于它先回收；回收后仍低才拒绝构建 |
| `storage.log_keep_days` | 整数 | 否 | `14` | 日志保留天数（另保留最近 50 份） |
| `llm_timeout_secs` | 整数 | 否 | `300` | 单次模型调用的总预算（秒），全局通用 |
| `compact_at_percent` | 整数 | 否 | `70` | 上下文用到模型窗口的百分之多少就自动压一次（0 = 关） |
| `compact_keep_percent` | 整数 | 否 | `15` | 压缩时至少保留最近多少内容（占模型窗口 %；0 = 只留摘要） |
| `compact_model` | 字符串 | 否 | 空 | 压缩用模型 id；空 = 会话同模型 |
| `discuss_remind_cap` | 整数 | 否 | `3` | 讨论里一轮内对同一个成员最多提醒几次 |
- `thinking.levels` 是全局思考等级标签目录：默认四档，用户可追加任意标签（例如 `xhigh`、`高`、`极高 档`）；标签可以任意字符，但不能为空、不能有首尾空白或控制字符，不能重复。
- 删掉仍被模型 `thinking.levels` 引用的档位 = 拒绝并列出模型；模型默认档不在新目录里 = 同样拒绝。
- 会话里切档是会话级事实（写在 `session/` 下那张 `meta.yaml` 的 `thinking`，工作根持有、子会话共享）；状态栏与配置页只列这组模型都登记了的档，没选过档时直接显示共同默认档（见 `docs/session/session-model.md`）。
- `storage` 只约束可回收缓存（构建缓存卷 / 下载缓存 / 过期可写层 / 日志），不约束会话、虚拟根、模块产物；到上限先回收一次，不拒绝构建；只有宿主剩余低于 `min_free_gib` 且回收后仍低才拒绝；字段范围保存时校验。
- `periodic_gc=false` 只关闭全局周期回收，构建前 headroom 始终保留；`gc_interval_minutes` 控制全局周期回收，默认且最小 15 分钟，改设置最多 30 秒生效、不重启。
- `compact_at_percent` / `compact_keep_percent` 都按模型窗口算：前者是自动压缩阈值（0 = 关），后者是压缩时至少保留的最近内容（0 = 只留摘要）；压缩只改"发给模型什么"，转录完整保留；压缩记录本身是一条带 `kind = compacted` 的转录行，删除 / 留档 / 恢复走普通行操作；本轮新注入的用户消息 / 派发任务不会被压进摘要（见 `docs/session/session-model.md`）。
- `compact_model` 是全局压缩用模型：空 = 与会话同模型；非空要在 `models.yaml` 里已登记；它只在压缩触发时解析（不按回合、不刷新会话）；被它引用的模型不能直接删除。
- `sandbox.policy` 是全局硬约束：会话、子会话、agent 都不能覆盖；改它要保存后整体重启，让所有 guest / 常驻服务 / 工具进程按新策略重建；配置写错（拼错键、未知键、相对的主机只读根、非法路径）启动即报 `settings.yaml 配置非法`。
- `sandbox.defaults` 是会话权限的全局默认；会话级权限层写在 `meta.yaml` 的 `sandbox.ceiling` / `sandbox.narrowing`；`sandbox.policy.deny` 是硬底线，会话只能追加黑名单，不能清空；profile 与工具声明只决定「能到哪、要不要问」，不能突破 policy 的 network / host_read_roots / deny / 强制等级上限。
- 权限语义只有一份权威：`docs/permission/README.md`。
- 网络是全局开关（`sandbox.policy.network`）：会话工具、直接模块工具、常驻服务、模块构建共用；provisioning 首启与核心模型出站不受它影响。
- 启动门：默认 CLI 与 `-webUI` 都先过环境检查；`sandbox.policy` 的要求没满足时，只能本次无围栏（不落盘）/ 重新设置 / 退出（`settings.yaml` 的 `on_unavailable`），不会起 CLI 或 Web；没有可回答的前端、或 `refuse` 且不重设 = 退出。
- 首次启动（`.home/` 下还没有 `settings.yaml`）在 Windows 上先问一次是否授权 `windows.write_acl`：选是就写盘并以退出码 75 整体重启；选否不写任何设置，直接进入启动门；这样默认不写本机权限项，又不会让首次启动停在"不可用"状态。
- 重设只修能修的那一项：Windows 未授权 → 写 `write_acl: true`；本机能力缺失 → 明确询问是否把 `enforce` 四项降为 `best_effort`（持久接受无强制），默认否。
- Windows 配置一致性：`require` 文件系统 / 网络围栏 + `windows.write_acl: false` 是非法组合——保存时直接拒绝；已存在的 `settings.yaml` 加载时报 `settings.yaml 配置非法`；首启没有文件不拦（交给引导）；要无围栏请把对应 `enforce` 降为 `best_effort`。
- `on_unavailable=ask` 时，启动门发现 `require` 的机制不可用会询问是否本进程降级；回答不落盘，重启再问；没人答 / 没有前端 / `refuse` = fail-closed；`SOLOMNI_SANDBOX_DEGRADED=1/0` 只供测试与 CI 在进程级注入（`1` 仅在 `ask` 下生效）。
- `sandbox.policy.windows.write_acl` 可被环境变量 `SOLOMNI_FENCE_WRITE=1/0` 临时覆盖（优先级高于设置，预检、启动报告与工具进程用同一个值）；它只影响 Windows 的目录 ACL；环境变量生效时不再做首次授权引导，重设也不会写无效的设置，而是提示改环境变量。
- `sandbox.policy.host_read_roots` 是用户显式授权的主机绝对路径只读根（默认空）：工具进程对这些目录只读可达，不继承写；授权落在用户自己的目录上；撤权与 `write_acl` 走同一份台账（`--fence-clean` 一并撤净；启动期按归属对账回收陈旧授权，见 `ARCHITECTURE.md`）。
- `llm_timeout_secs` 是单次模型调用的总预算，全局通用：连接之外，等响应头 / 读响应体 / 整体都用它；非流式下供应商要等整段生成完才发响应头，单独设一个小的「头超时」会把几十秒的正常长回复误判成不通；预算用尽 = 中断这一轮并如实告知（用户可以点「继续」重试），不是把会话作废。
- 模型调用次数没有上限：工具循环跑到模型不再发起调用为止（执行席与讨论席一样），用户随时可以点「停止」；`discuss_remind_cap` 只是提醒次数：提醒到顶就如实记一行"未回应"放过它，整轮继续。

## 六、名字与 id
| 字段 | 类型 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- | --- |
| `<供应商 id>` | 字符串 | 是 | - | 登记处内的映射键，非空且唯一 |
| `<模型 id>` | 字符串 | 是 | - | 登记处内的映射键，非空且唯一 |
| `<agent 名>` | 字符串 | 是 | - | 会变成目录名，校验更严（见下） |
- 供应商 id / 模型 id：登记处内的映射键，非空且唯一（yaml 映射天然保证唯一）。
- agent 名会变成目录名，因此校验更严：非空、首尾无空白、不含 `\ / : * ? " < > |` 与控制字符、不是 `.` 与 `..`、不是 Windows 保留名（`CON` / `PRN` / `AUX` / `NUL` / `COM1..9` / `LPT1..9`）。
- 同一工作内 agent 重名 = 核心自动追加 `-2`、`-3`（用户不改名时的兜底），并先让用户裁决是否改名。

## 示例
- `providers.yaml` 与 `models.yaml` 的最小可用组合：
```yaml
# providers.yaml
providers:
  main:
    base_url: https://api.example.com/v1
    api_key: sk-...
```
```yaml
# models.yaml
models:
  fast:
    name: 快速模型
    api_model: example-fast
    provider: main
    tools: native
    thinking:
      default: low
      levels:
        off:  { thinking: { type: disabled } }   # 对象 = 原样合并进请求体
        low:  low                                # 字符串 = {"reasoning_effort": 值} 简写
core: fast
```
- `agents.yaml` 的一个条目：
```yaml
agents:
  researcher:
    modules: [research]
    model: fast
    note: 资料整理
    chara: |-
      严谨的资料整理者：先核实，再下结论。
```

## 校验与拒收
- `providers.yaml`：`<供应商 id>`、`base_url`、`api_key` 不能为空；更新时 `api_key` 留空且没有旧值 = 报错；删除仍被模型引用的供应商 = 拒绝并列出引用者。
- `models.yaml`：`name` / `api_model` / `provider` 必填；`provider` 不指向已存在的供应商 = 拒绝新增；`tools` 不是 `envelope` / `native` = 拒收；原生形态保存前实测不通过 = 拒绝新增 / 更新；`thinking` 片段覆盖协议字段 = 保存时拒收；删除核心默认模型 = 拒绝；`core` 未设定或解析失败 = 建会话 / 核心操作如实拒绝。
- `agents.yaml`：agent 名不满足「六、名字与 id」的校验 = 拒收；同一工作内重名按兜底规则处理。
- `settings.yaml`：`thinking.levels` 的标签为空、有首尾空白 / 控制字符或重复 = 拒收；删掉仍被引用的档位、模型默认档不在目录里 = 拒绝；拼错键 / 未知键 / 相对的主机只读根 / 非法路径 = 启动即报 `settings.yaml 配置非法`；`require` 文件系统 / 网络围栏与 `windows.write_acl: false` 并用 = 保存拒绝；`storage` 字段范围保存时校验。
- 密钥边界：通道密钥只在 `providers.yaml` 与核心发起的出站调用里存在，永不进入提示词、转录、日志、模块工作区。
- 密钥边界：模块隐秘字段的值只存在 `.home/` 的隐秘登记处；核心按模块作用域经环境变量注入该模块自己的工具 / 服务进程——值不进提示词、转录、日志、模块工作区与命令行；字段的声明与注入名在 `module.yaml` 的 `secrets:`。
- 密钥边界：界面与报告只呈现「供应商 id + 端点」与「模型 id + 展示名」，永不携带密钥。
- 密钥边界：出站错误信息经脱敏后才出适配层；日志与转录里出现的永远是 id，不是密钥。
- 核心只校验，不纠正：解析失败 = 如实报错（装配失败），不静默兜底。

## 路由
- 探测 / 回放探测的行为、结论判定与写回 → `docs/registry/README.md`
- 模块如何声明隐秘字段 → `MODULE_SPEC.md`
- 会话怎么记模型、思考档与压缩 → `docs/session/session-model.md`
- 沙箱、权限档位与围栏语义 → `docs/permission/README.md`
- 构建缓存与存储回收 → `docs/builds/README.md`、`docs/storage/README.md`
- 虚拟机档的依赖路径登记与就绪检测 → `docs/vm/README.md`
- 分层与依赖方向 → `ARCHITECTURE.md`
