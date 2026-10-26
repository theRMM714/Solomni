# Solomni

> **English:** [README_EN.md](README_EN.md) —— the same overview in English.

Solomni 是一个**跑在自己机器上的 agent 运行环境（AgentOS）**：一个可执行程序 + 一个 `modules/` 文件夹
就是整个产品。没有安装程序、没有服务端、没有账号。

## 为什么需要它

- **工具与工作流按契约接入。** 一个文件夹 + 一份 `module.yaml`（能力摘要 + 工具用法 + 工具声明）就是模块，启动命令由模块自己声明。
  适配只落在模块自己那边，不引 SDK、不动本仓代码结构。**契约之外双方互不依赖**：拿掉本产品，模块照原样跑；移走模块，产品照常运行。
  同一个能力，人走界面用、agent 走契约用，共用同一份声明（契约见 [MODULE_SPEC.md](MODULE_SPEC.md)）。
- **多个 agent 一起干一件活。** 每个 agent 有自己的沙箱（`session/<工作名>/<agent 实例名>/`），它同时是共享区的**工作副本**；
  共享区 `work/` 是**主副本、对 agent 只读**。要用的内容先 `work_pull` 拉进沙箱，改完 `work_commit` 提交回共享区。
  提交按**文件级三方比较**判定：冲突整个拒绝并逐条点名，不会静默覆盖；冲突后用 `work_versions` 看三方、`work_resolve` 显式选边——文本可人工合并后用 `take=local`，图片 / 视频 / 二进制只能选边，不做自动合并。
  文本、图片、视频与其它二进制都按字节处理；任一提交点都能还原。
  跨 agent 流动只有共享区与进入上下文的转录两条明路；"谁只写哪一块"由**会话权限**落实（见 [docs/permission/README.md](docs/permission/README.md)）。
- **过程可审计。** 转录**只追加**；删除 / 恢复是显式动作，会真的重写流水。任意一行可「回档到此处」：
  **留档**（追加标记、折叠之前的行，字节全留）/ **删除**（真的截断）/ **恢复**（在标记处截断，撤掉留档期间的新工作）。
  行 id 单调、永不复用；回放可复现，**重建与实时逐条一致**。共享区按提交记录上的 `(agent, 行)` 锚一起回到那一刻
  （见 [docs/session/session-model.md](docs/session/session-model.md)）。

## 由什么组成

| 东西 | 是什么 |
| --- | --- |
| **核心** | 产品里唯一的程序（Rust）。提供机制：会话与转录、任务编排与验收、工具执行与围栏、登记处、回档与压缩 |
| **模块** | 一个文件夹 `modules/<id>/`：一份 `module.yaml`（能力摘要 + 工具用法 + 工具声明，可声明常驻服务与隐秘字段）+ 工具本体。任何语言、任何能跑起来的东西 |
| **agent** | 名字 + 它具备的模块（可以为空：只用内置文件工具）+ 模型 + 自己的沙箱。模块被装进哪个 agent，就以那个 agent 的名义说话 |
| **使用者** | 人与 agent。人走界面（终端 / 本机网页），agent 走模块契约——同一批模块，两个门 |

## 当前能力

- **三种形态**：单 agent / 小组协作 / 代理（决定权整块交给核心）。形态、流程与用户旅程见 [PRODUCT.md](PRODUCT.md)。
- **模块即插即用**：`modules/` 里放一个带合法 `module.yaml` 的文件夹就出现，移走就消失——不注册、不编译、不重启（契约见 [MODULE_SPEC.md](MODULE_SPEC.md)）。
- **会话与历史**：一个工作一个目录 `session/<工作名>/`；回档 / 留档 / 删除 / 恢复、转录回放、上下文压缩、共享区版本化（`work_pull` / `work_commit` / `work_status` / `work_versions` / `work_resolve` / `work_lost`）。
- **三重隔离**：内置工具越界拒绝 → 工具进程走守门进程（环境白名单 / 进程树 / 超时杀整棵树）→ 平台围栏（Windows AppContainer、Linux Landlock、macOS seatbelt）；装不上就如实报告能力等级。
- **两档执行**：本机档（默认）与虚拟机档（模块工具在 guest 里跑）。虚拟机档按宿主 × 引擎 × guest × 通道 × 共享五轴可插；
  Linux（QEMU/KVM + virtiofs，缺 virtiofsd 时退 9p）与 Windows（QEMU/WHPX + SMB guestfwd）已真机验收；Windows 缺 smbd 时虚拟机档不可用（先补齐，不降级）；copy 只是开发/应急通道，不参与用户档。macOS 未验（`VERIFIED_HOSTS` 只放开 Linux / Windows；缺 QEMU 取包与 Apple vz 引擎，见 [tests/macos/gaps.yaml](tests/macos/gaps.yaml)）；
  虚拟机档在 Linux / Windows 上不可用时，网页与命令行都能按同一份清单补齐依赖（见下）。细节见 [docs/vm/README.md](docs/vm/README.md)，运行包契约见 [RUNTIME_SPEC.md](RUNTIME_SPEC.md)。
- **登记处**：供应商 / 模型 / agent / 设置四份 YAML，全在 `.home/`，由界面管理；通道密钥只在本机登记处与核心的出站调用里（边界见 [REGISTRY_SPEC.md](REGISTRY_SPEC.md)）。
- **两种交互面**：终端转录中心（默认）与本地网页（`-webUI`，只绑 `127.0.0.1`，默认 3081）。
- **一条裁决通道**：核心关卡、工具级确认与工具执行层的问题都用同一张「消息 + 选项」卡；必须阻塞，选项 id 是契约，卡片与回答随会话落盘。行为见 [PRODUCT.md](PRODUCT.md) 与 [docs/session/session-model.md](docs/session/session-model.md)。

## 安装与运行

前置：**Node**（跑启动器）与 **Rust**（构建核心）。环境分两层：**运行环境**（只想跑产品）与**开发环境**（改代码 / 跑门禁）。

### 运行环境

```bash
node setup-runtime.js    # 准备运行环境：项目内 platform/<os>/ 与 .tools/；有可用 Rust 则只读借用
node setup-toolchains.js # 只补基础工具链（setup-runtime 默认会做；下载前要 --yes）
node setup-toolchains.js --check # 只读校验：缺什么列什么，缺则非零退出
node start.js            # 任意平台；Windows 也可双击 start.bat，macOS / Linux 也可 ./start.sh
node start.js -webUI     # 直接进本地网页（127.0.0.1:3081）
node start.js --provision                # 可选：虚拟机档只读状态（JSON：QEMU / guest 镜像 / agent 与运行包）
node start.js --provision --ensure --yes # 可选（Linux / Windows）：把 QEMU 与 Debian guest 装进项目内 .vm/（下载前要 --yes）
# 网页里也能直接补（Linux / Windows）：虚拟机档不可用、或选中的虚拟根未装齐时，配置面板 / 创建向导给的「补齐虚拟机依赖」就是这条（不必开终端）
```

- **首次启动**：没有 `.home/settings.yaml` 时，Windows 会先问一次是否授权容器围栏（写目录 ACL）；
  选是就写盘并整体重启；选否不写设置、进入启动门（本次无围栏 / 重设 / 退出）。`-webUI` 也要先过这关，
  不会“带参数就直接开网页”。基础工具链缺失时，`start.js` 还会在首次启动问一次是否装进项目内；
  之后每次启动只报告缺失与补全命令，不再追问。
- 工具链与缓存收敛在**项目内**：检测到可用的 Rust 就**只读借用**，缓存与产物仍落 `platform/<os>/`；没有或有问题的才先征求同意装进项目内，不动系统；**无交互终端时不做安装**，打印手动步骤后退出。
- 模块工具/构建的**基础工具链**（Python / Node / C-C++）是运行前提，装进 `.tools/toolchains/`，启动时前置 PATH。`setup-runtime.js` 默认安装；`start.js` 首启询问、之后每次启动报告项目内 / 宿主回退 / 缺失。宿主回退只在 Windows 围栏能授权时可用（按**真身安装根**判：已有足够权限项、目录属主是当前用户、或当前用户对该目录有 WRITE_DAC；命令名经链接进来时链接本身不授）；否则如实报原因并要求先装项目内。C# 等额外能力可由用户把适用于本机目标的包放进 `runtimes/`（包 `bin/` 进本次执行的 PATH；缺包如实报告）。
  C/C++ 在 Windows 用 MinGW-w64、Linux / macOS 用 Zig；macOS 链接原生框架仍需要系统的 Xcode Command Line Tools / SDK。
  借用系统工具链时不读系统 `~/.cargo/config.toml`（镜像 / 凭据），依赖会重新下到项目内——这是「不写项目外」的取舍；门禁还会快照项目外缓存根，写出去即失败。

### 开发环境

```bash
node setup-dev.js        # 装齐开发环境：开发统一用**项目内**工具链，产物只落 platform/<os>/ 与 .tools/
node setup-dev.js --check  # 只读校验：缺什么列什么，缺则非零退出
node run-tests.js        # 质量门禁 + 全量测试（见 TESTING.md）
node run-tests.js --skip-e2e  # 迭代时按门禁判一次（质量 job 模式，只省 L4）
node dev.js test|fmt|clippy   # 本机开发回路：改一层验一层；不要直接调 cargo（见 docs/testing/execution-ci.md）
```

- 开发环境的**条目清单**是 [dev-tools.js](dev-tools.js)（唯一真相）：`setup-dev.js` 按它装，`run-tests.js` 按它查 / `env-skip`。
  含 Rust 组件（rustfmt / clippy / llvm-tools-preview）、交叉 target（Windows + macOS）、项目内工具（cargo-audit / cargo-deny / cargo-llvm-cov）。
- 与运行环境分开的理由：只想跑产品的人不需要这些；门禁在只有运行环境的机器上仍能跑，缺的项如实 `env-skip`。
- 环境是独立一层（[env.js](env.js)）：路径约定、工具链探测、环境拼装与安装都在这里；`node env.js` 看解析结果，`node env.js --print-env` 给机器读。
  `setup-runtime.js` / `setup-dev.js` 只编排；`node setup-runtime.js` 与 `node env.js setup` 等价。
  Windows 首次运行会征得同意后把可携 MinGW（`dlltool.exe`，`windows-sys` 的 raw-dylib 需要）装进项目内 `.tools/`；
  已有完整 binutils 的机器用 `SOLOMNI_MINGW_BIN=<bin 目录>` 指过去即可，不必再下 260 MB。
- **外部连接台账**：`links.yaml` 是项目自用、可能变的外部 URL 的唯一真相（当前：工具链引导源、winlibs 上游发布包的钉死 URL、自托管依赖 Release 前缀、guest 镜像源与 SMB helper 资产）；
  改一处全仓生效。**下载源的校验值与那条链接成对**写在同一份台账里（`<链接键>.sha256`），自托管 release 资产的校验值写在 `dependencies.yaml` 的条目里。
  用户覆盖走 `.home/settings.yaml` 与环境变量，不改仓库里的台账。
- **没有可用模型就不建会话**：先登记供应商与模型并设定核心默认；缺了如实拒绝并指明缺什么，不回落假模型。
- 常用参数：`--root <目录>`（换产品根）、`--web-port <端口>`、`--release`。
- 终端提示符下的命令：启动时打印的菜单就是全量清单（`src/presentation/cli/mod.rs` 的 `print_menu`）；回档走 `rewind`，供应商 / 模型 / 核心默认模型分别走 `provider` / `model` / `core`。

### 配供应商与模型

在界面里加供应商（端点 + 密钥）与模型，并给核心选一个默认模型；密钥只写进 `.home/providers.yaml`，不回显。

## 模块契约

模块 = 一个文件夹 + 一份 `module.yaml`，**不依赖本项目的任何代码**；核心也不依赖模块的实现——双方只依赖这份契约（完整字段与规则见 [MODULE_SPEC.md](MODULE_SPEC.md)）：

```yaml
id: research                 # 全局唯一，= 文件夹名
brief: 调研与对比选型。        # 能力简述：给核心选人看
system: 用法：scan 必填 root 与 out……  # 工具用法：参数 / 路径 / 输入输出 / 错误（能力写在 brief）
tools:                       # 可选：模块工具表
  read_txt:
    command: python tools/read_txt.py   # 工作目录 = 本模块根目录
```

- 模块工具只有一条调用约定：**启动命令 + 参数 + stdin/stdout 回执**；长驻 MCP 服务写进 `services`。
- 工具进程可达该 agent 的私有沙箱与它使用的模块目录；共享区主副本要先 `work_pull` / `work_commit`（冲突用 `work_versions` / `work_resolve`）。模块目录（含 `userdata/`）对 agent 的文件工具只读：agent 只有模块使用权，模块数据由模块自己的工具 / 服务进程维护。
- 能跑通就是合法模块；跑不通时核心如实报错，不猜、不兜底。

### 跑一遍演示

演示是**真机测试**：走真实模型，会真的投喂资料、调工具、核产物。所以脚本开工前先核对条件——
前两个演示要三个模块在清单里、登记处有供应商与一个可用的模型（用 `SOLOMNI_DEMO_MODEL` 指定，或用核心默认）；indexer 的产物在创建会话前自动补编（也可先手动 `solomni --build=indexer`）；
**MCP 演示**要 `mcp-toolbox` 在清单里、登记处有供应商与可用模型（它自带 `services` 声明，不需要预先构建）；
**代理演示**要登记处有供应商、模型与**核心默认模型**（挑人是核心的事，它可以临时组装零模块的 agent）。
**虚拟机档演示**要 `harvest` 在清单里、登记处有可用模型，且选定的虚拟根在本机装齐、画像里声明有 python / node / cc / gcc（`guest.yaml` 的 `runtimes`——Linux / Windows 都由 `solomni --provision --ensure --yes` 的运行期编排起一次 guest 自证后写入；缺包时核心会在创建那一刻拒绝）。
**条件不齐就打印 `DEMO-SKIPPED` 与怎么补、以退出码 2 收场：不跑，也不用内置演示通道凑一遍。**

```bash
node start.js -webUI
node demo/run-demo.mjs         # 组合式：一个 agent 装三个模块（python / node / C++）
node demo/run-demo-collab.mjs  # 小组协作：三个 agent 各持一份能力协商，走完整条流程
node demo/run-demo-mcp.mjs     # MCP：单 agent 只装 mcp-toolbox，拉起常驻服务并真调它的操作
node demo/run-demo-proxy.mjs   # 代理：只跟核心说话，它自己挑人、建子工作、被叫醒；停止即全停
node demo/run-demo-vm.mjs      # 虚拟机档：单 agent 在 guest 里跑 harvest，产物经实时共享直接落宿主
```

C++ 模块的产物不入库：创建会话前会按 `module.yaml` 的 `build:` 段在目标环境自动补编一次（失败则创建被拒，理由见 [modules/indexer/README.md](modules/indexer/README.md)）；也可以手动 `solomni --build=indexer`。

## 还没接入

未完成项、怎么补与验收标准只有一处权威：全局看 [tests/gaps.yaml](tests/gaps.yaml)，平台专属看 [tests/macos/gaps.yaml](tests/macos/gaps.yaml)；核心代理的系统工具规划在 [systool_gaps.yaml](systool_gaps.yaml)。

## 它不是什么

- 不是模型或供应商：通道与密钥是产品资源，模型可以任意替换。
- 不是模块调度器：`tools` 每次任务起一次性进程、跑完即走；`services` 声明的常驻服务由核心按用户授权拉起，随会话租约回收。
- 不是服务端：只绑 `127.0.0.1`，没有账号。

**质量**：`node run-tests.js` 是唯一入口——T0（格式 / 编译 / clippy / 依赖重复 / 结构审查 / 交叉类型检查 / 项目外写 / 供应链，零容忍；供应链工具缺失时记 `env-skip`）+ 单元 +
跨平台集成 + 前端冒烟 + 端到端；平台围栏的真机探针由三平台 CI 跑（见 [TESTING.md](TESTING.md)）。

## 文档指引

| 我想… | 读这一份 |
| --- | --- |
| 看项目概括与快速开始 | 本文（English: [README_EN.md](README_EN.md)） |
| 看这个项目要做成什么（方向与承诺） | [PHILOSOPHY.md](PHILOSOPHY.md) |
| 用产品 / 看用户旅程与交互面 | [PRODUCT.md](PRODUCT.md) |
| 写一个模块（YAML 契约与模块工具） | [MODULE_SPEC.md](MODULE_SPEC.md) |
| 系统工具、角色与路径模型（回报与验收也在工具里） | [SYSTOOL.md](SYSTOOL.md) |
| 看未完成项、怎么补与验收标准 | [tests/gaps.yaml](tests/gaps.yaml)（全局）与 [tests/macos/gaps.yaml](tests/macos/gaps.yaml)（平台） |
| 核心代理的待实现系统工具清单与方案 | [systool_gaps.yaml](systool_gaps.yaml)（规划，不代表当前能力） |
| 做运行包（`package.yaml`） | [RUNTIME_SPEC.md](RUNTIME_SPEC.md) |
| 管理供应商 / 模型 / agent / 设置（含密钥边界） | [REGISTRY_SPEC.md](REGISTRY_SPEC.md) |
| 改代码（分层、端口、日志、提示词、落盘契约） | [ARCHITECTURE.md](ARCHITECTURE.md) |
| 写测试与验收（层级、替身、门禁、缺口账、CI） | [TESTING.md](TESTING.md) |
| 看开发流程、PR 与开工闸门 | [docs/workflow/README.md](docs/workflow/README.md) |
| 看文档分层、术语与引用约定 | [docs/conventions.md](docs/conventions.md) |
| 看全部单元与横向细则 | [docs/README.md](docs/README.md) |
| 仓库协作规则与文档路由 | [AGENTS.md](AGENTS.md) |

> **两层结构**：仓库根是**门户**（定位 + 引用），`docs/` 是**细则**——同一事实只有一份权威。
> 完整路由见 [AGENTS.md](AGENTS.md) 的「文档路由」。

Apache License 2.0 · 见 [LICENSE](LICENSE)
