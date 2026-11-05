# 运行包契约（RUNTIME_SPEC）
> 定位：运行包作者要遵守的契约：一个包长什么样、清单怎么写、两种 kind 怎么装配、什么被拒收。
> 效力：契约｜核对：2026-11-04｜依据：`src/capabilities/packages/domain/manifest.rs`

## 结构
```text
runtimes/<包文件夹>/
  package.yaml
  ...包内容（二进制、库、脚本，按 kind 组织）
  host-<os>-<arch>/    # 可选：只给本机档的载荷
  guest-<os>-<arch>/   # 可选：只给虚拟机档的载荷
  <os>-<arch>/         # 可选：两档共用的载荷（os ∈ linux/macos/windows，arch ∈ x86_64/aarch64）
```
- 一个包 = 一个文件夹 + 一份 `package.yaml`；放入 `runtimes/` 即出现，移出即消失，没有注册仪式（该目录在 `.gitignore` 里：包内容多为二进制，只提交源码与文档）。
- 目标由发起方给（本机会话 / 直接动作 / 常驻服务 / `--build` = 宿主；虚拟机档 = 选定的 guest）；包不声明自己服务哪一档。
- 本机档取 `host-<os>-<arch>/`（优先）或 `<os>-<arch>/`，把载荷根当作包根，`<根>/bin` 进本次执行的 PATH。
- 虚拟机档取 `guest-<os>-<arch>/`（优先）或 `<os>-<arch>/`，把载荷根挂到 `/<prefix>`。
- 没有平台子目录 = 两档通用；只有一档的载荷 = 另一档不可用，诊断如实报"目标不匹配"，不挂错的那份。
- 模块只声明能力名（`module.yaml` 的 `runtimes: [python]`）；精确版本由用户在会话的执行档位里定（`meta.yaml` 的 `exec.pins`）；库里同一能力有多个版本而用户没定版 = 核心如实报"需定版"，不替用户挑。
- 缺包不是崩溃，但也不能装看不见：本机档脚本跑不了就如实报错；虚拟机档在「开始」前如实拒绝（没有工具用户完不成工作），不静默降级。

## 一、包清单（`package.yaml`）
| 字段 | 类型 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- | --- |
| `id` | 字符串 | 是 | - | 能力名（= 包 id）；同 id 不同版本可并存 |
| `version` | 字符串 | 是 | - | 精确版本（不含空白与路径分隔符） |
| `kind` | 字符串 | 否 | `prefix` | `prefix`（独立前缀）｜`system`（写进系统路径，叠层） |
| `prefix` | 字符串 | `kind = prefix` 时是 | - | 独立前缀（书写形式：相对、`/` 分隔） |
| `provides_paths` | 字符串数组 | `kind = system` 时是 | `[]` | 它会写进去的系统路径 |
| `requires` | 字符串数组 | 否 | `[]` | 本包自己需要的能力名（传递依赖由核心求闭包） |
| `license` | 字符串数组 | 否 | `[]` | 再分发要随包带上 |
| `note` | 字符串 | 否 | 空 | 说明 |
| `files` | 数组 | 否 | `[]` | 包内文件指纹；见下三行 |
| `files[].path` | 字符串 | `files` 非空时是 | - | 包内相对路径 |
| `files[].sha256` | 字符串 | `files` 非空时是 | - | 内容指纹 |
| `files[].bytes` | 整数 | `files` 非空时是 | - | 字节数 |
- 能力名就是包 id：一个包提供它自己的 id 这一个能力名（别名尚未支持）；写法：小写字母或数字开头，其余小写字母、数字、`-` `_` `.`，长度 1..=64。
- `files` 里的 sha256 用于可复现，不用于信任：用户自造的包没有可信基准，本程序不据此判断包是否安全。
- `files` 随包库扫描一起解析并保留，供呈现与后续装配使用，但不参与装配判断，也不做校验（指纹对不上不会报错，也不会因此换版本）；包内容的组织方式由包作者决定，核心只认清单事实。
- 产品不预置任何包：库里有什么能力，完全取决于用户往 `runtimes/` 放了什么；`python` / `node` / `bash` / `cc`（C/C++ 工具链）只是常见写法的示例，没有特殊含义。
- 基础工具链不是运行包：产品把 Python / Node / C-C++ 的固定版本作为运行前提装进 `.tools/toolchains`（由 `--provision` 的 `kind: toolchain` 补，组别名只补缺项；登记进 `.tools/toolchains/active.json`），服务本机档；虚拟机档的同一组能力由 guest 镜像的 `guest.yaml` 声明，`vm` 的运行期编排负责安装并逐个自证（见 `docs/vm/README.md`）。

## 二、kind 与装配
| 字段 | 类型 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- | --- |
| `kind: prefix` | 枚举值 | 否 | `prefix` | 包整体挂到自己的前缀下、只读；本机档把载荷根当包根、`bin/` 进 PATH |
| `kind: system` | 枚举值 | 否 | - | 包会把文件写进 guest 系统路径（编译器要 `usr/include`、链接器搜索路径这类） |
- 本机档不叠写系统路径：`system` 类包在本机档报"不服务本机档"，不静默改成别的装法。
- 冲突判定只针对虚拟机档的 `prefix` / `system`：两条路径相等或一方是另一方的前缀目录都算落在同一处，叠加时互相覆盖，顺序消解不了。
- 装载顺序：先 `prefix` 类包（各自独立），再 `system` 类包（按 id 升序叠层）。
- 装配动作（起 guest 后软链 `bin`、按 `provides_paths` 叠写、失败不落"已叠"标记）见 `docs/vm/README.md`。

## 示例
- `kind: prefix`（缺省）的最小清单：
```yaml
id: python                  # 能力名；同 id 不同版本可并存
version: 3.12.4             # 精确版本（不含空白与路径分隔符）
kind: prefix                # 缺省 prefix
prefix: opt/rt/python3.12   # kind = prefix 时必填：独立前缀
requires: []                # 本包自己需要的能力名
license: [PSF-2.0]          # 再分发要随包带上
note: ""
files:                      # 包内文件指纹：用于可复现与呈现
  - path: bin/python3
    sha256: "..."
    bytes: 12345
```
- `kind: system` 的最小清单：
```yaml
id: gcc
version: 14.2.0
kind: system
provides_paths: [usr/include, usr/lib]   # kind = system 时必填
```

## 校验与拒收
- `package.yaml` 非法 = 拒收并说明原因，包库报告如实列出；核心只校验，不纠正。
- `id` 不合法（不是能力名写法）。
- `version` 为空，或含空白与路径分隔符。
- `kind` 不是 `prefix` / `system`。
- `kind = prefix` 缺 `prefix`，或 `prefix` 不是合法书写形式（相对、`/` 分隔、无空段、无 `.` 与 `..`、不含反斜杠），或 `prefix` 类包却声明了 `provides_paths`。
- `kind = system` 缺 `provides_paths`，或其中任一路径不是合法书写形式。
- `requires` 里的能力名不合法，或 `requires` 依赖自己。
- 同一（id, version）出现两次：只收先出现的那份，另一份拒收并说明原因。
- 虚拟机档两个包写进同一处系统路径（相等或互为前缀目录）= 冲突，装配前如实报出。
- 同一能力多版本而用户没定版 / 定版指定的版本不在库里 / 两个包写进同一处路径 = 「开始」那一刻如实拒绝，由用户解决选型后再开始。
- 本机没有虚拟机监视器 / guest 后端 / 引擎 / 共享档 = 虚拟机档不允许创建或改入，配置界面禁用并说明缺什么（用户环境问题，不是选型问题）。

## 路由
- 包库扫描、选包、诊断与 `PackagePlan` → `docs/packages/README.md`
- 装配计划、guest 装载、就绪检测与拒绝时机 → `docs/vm/README.md`
- 模块怎么声明 `runtimes` → `MODULE_SPEC.md`
- 登记处、会话定版与执行档位 → `REGISTRY_SPEC.md`、`docs/session/session-model.md`
- 依赖补齐（`--provision`）与虚拟根构造 → `docs/provision/README.md`
- 分层与依赖方向 → `ARCHITECTURE.md`
