# builds（模块构建）
> 定位：模块 `build:` 声明在目标环境里的执行、产物复验、构建缓存与失败归因。
> 效力：规范｜核对：2026-11-04｜依据：`src/capabilities/builds/api.rs`

## 管什么 / 不管什么
- 管：一次"确保产物可用"的用例（入口在场即跳过、缺失才编、编完复验）、构建期能力选包、源指纹缓存与 `.vm/build-state/` 落点、构建缓存卷与目标环境。
- 不管：`module.yaml` 的 `build:` 字段契约（`MODULE_SPEC.md`）、运行包契约与选包（`packages`）、guest 与盘怎么起（`vm`）、会话生命周期（`session`）。

## 入站契约与状态归属
- 入站面：`api::Builds`（`ensure_many` 一批模块、`build` 单个模块）+ `api::Target`（`Host` / `Guest(虚拟根名)`）+ `api::Built`（模块 / 目标 / 是否跳过 / 输出）。
- 调用方：`conductor` 在建 / 启用会话前调 `ensure_many` 补编；`diagnostics` 的 `--build` 与体检是 `build` 的薄壳。
- 状态归属：构建状态落 `.vm/build-state/<模块>.json`，由本单元读写（`target` + `fingerprint`）；构建缓存卷落 `.vm/build-cache/<虚拟根>.qcow2`，回收归 `storage`。
- 构建是系统动作，不申请容器身份；围栏、guest 与进程由 `vm` / `kernel` 提供，本单元只给规格。

## 依赖图位置（由源码的 `::api` 引用推导）
- 经 `::api` 用到：`kernel`、`modules`、`packages`、`provision`、`storage`、`vm`、`workspace`
- 谁在用我：`conductor`（建 / 启用会话前补编）、`diagnostics`（`--build` 与体检）、`main`（组合根装配）

## 一、两个调用方与跳过判据
- 创建会话 / 启用历史会话前（自动）：本次名单里的模块，若声明了 `build:` 且 `build-state.target` 不是目标环境（或入口不在场），就先在目标环境编一次；编成继续建 / 启用，编不成则创建或启用失败并如实报原因（不留下半成品、不落构建状态）。
- `solomni --build=<模块 id> [--guest=<虚拟根>]`（手动）：入口在场也按源指纹决定是否重编；给作者与运维的显式入口。
- 入口在场且状态与目标一致 → 跳过（创建时零成本；手动 `--build` 仍看指纹）。
- 入口在场但没有构建状态（随仓自带产物）→ 跳过；无法从状态判断目标。
- 入口在场但状态是别的目标（如宿主产物进 guest）→ 在目标环境重编，编完复验入口。
- 入口不在场、模块声明了 `build:` → 在目标环境编一次，编完复验入口。
- 入口不在场、模块没声明 `build:` → 拒绝，指到「自带产物或按模块 README 构建」。
- 构建期能力缺失 → 拒绝，点出缺哪个能力 + 几条出路。
- 构建命令失败 / 编完入口仍不在场 → 拒绝，错误里带目标环境与命令输出；不落状态（下次重试）。

## 二、入口判据与成功口径
- "入口在场"与创建会话时的入口判据同一把尺子：名字按目标平台的壳解析（Windows 补 `.exe` / `.cmd` …），跨平台还看文件形式（ELF / Mach-O / PE / 脚本头）。
- 同平台同架构但 CPU 特性不同（宿主 native 产物进 qemu64 guest）入口判据看不出来——所以缓存命中必须同时看 `.vm/build-state/<模块>.json` 里的 `target`。
- 产物是唯一判据：入口在目标环境里在场且能跑才算编成。
- 命令退出码只当快速失败信号——非零即失败；零退出但入口不在场同样失败（此时错误里明说"脚本可能吞掉了失败"，并附命令输出）。
- 为什么不信退出码：构建脚本自己会判失败，但脚本写错（例如 Windows 的 `if errorlevel 1` 对崩溃码不成立）就会把编译器的崩溃报成成功；产品据此复验产物，所以误判只会让归因不准，不会让失败被当成成功。

## 三、目标环境
- 本机档：在宿主上编；围栏由产品给，只写模块目录与 `.vm/build-state/`，只读放行项目内基础工具链的安装根（编译器要读自己的 libexec / 动态库）；出站网络 = 全局开关与模块 `build.net` 的交集，平台落点见 `docs/vm/README.md`。
- 构建是系统动作，不申请容器身份（与 `--build` 同一条执行路）：Windows 上按无容器路径跑，可达范围仍由围栏规格给出；容器里的工具临时区由围栏准备（见 `docs/kernel/README.md`），编译器可以在容器里落临时文件，构建本身不走这条路。
- 虚拟机档：在指定虚拟根里编；起一台一次性 guest（`isolated`，用完销毁），一批模块共用这一台。
- 模块目录读写挂进去只当数据面（源与产物回写的落点）；构建在 guest 本地的缓存卷工作区里发生（`.vm/build-cache/<虚拟根>.qcow2` → `/var/lib/solomni/build`；源复制进来，`HOME` / `TMPDIR` 也在卷上），成功后才按差异回写宿主那份模块目录（`userdata/` 不参与）。
- 卷与模块各有一份环境标识（镜像 / runtimes、`needs` + 实际提供者），变了就清对应范围；卷挂不上会重建文件系统再挂；同一虚拟根一把构建锁（不并写）；构建完 `fstrim` 把空块还给宿主；批次补编共用一台 guest 与一块卷。
- 缓存卷工作区里的 `chmod` / 符号链接 / 锁是 guest 原生语义：权限位与链接行为按卷内文件系统生效，不套用宿主规则。
- 编译能力由 guest 自带（`guest.yaml` 的 `runtimes`）或运行包库提供；需要 QEMU 与可用虚拟根，缺了如实拒绝。
- 宿主工具链不进这条链。

## 四、构建期能力
- `build.needs` 里的名字是目标环境 PATH 上的可执行名（如 `gcc`）。
- 本机档：项目内基础工具链（Python / Node / C-C++）优先，其次宿主 PATH 回退，再其次是 `runtimes/` 里的包；缺了如实拒绝——「声明不构成保证」。
- 虚拟机档：guest 自带的运行能力优先，其次包库（prefix 的 `bin` 进 PATH、system 的载荷叠写进系统路径）。
- 选包规则与运行能力同一套，见 `RUNTIME_SPEC.md` 与 `docs/vm/README.md`。
- 项目内基础工具链由产品 provision 补齐（`node start.js --provision --ensure=toolchain --yes`，kind 组别名只补缺项；装完由 Rust 侧按清单重写 `.tools/toolchains/active.json`；`env.js` 装构建期 Zig cc 时也会合并登记 `cc`）；`start.js` 只指路（见 `README.md` 的「安装与运行」）。

## 五、缓存与回收
- 状态落 `.vm/build-state/<模块>.json`：`target`（`host` / 虚拟根名）+ `fingerprint`；本机档与各虚拟根共用一份，靠 `target` 区分。
- 源指纹 = 构建命令 + 能力名 + 能力的实际提供者 + 网络开关 + 模块内全部源文件（相对路径与内容）；跳过 `build/`（产物本身）与 `userdata/`（跨任务私有状态，不是源）。
- 指纹一致且入口都在场才跳过重编。
- 产物写入模块目录（`build/` 在 `.gitignore` 里）；agent 看得见产物，看不见工具链。
- 卷大小：新卷虚拟大小取 `settings.yaml` 的 `storage.build_volume_gib`（默认 16 GiB 稀疏）；实际占用看 qemu-img 的 actual-size。
- 起 guest 前由 `storage` 做 `ensure_headroom`（到预算 / 宿主余量不足先回收一次，当前目标卷受保护）；构建失败也清临时目录并 `fstrim`；epoch 清空卷后也 trim。
- 模块工作区有 `USED` 时间戳，超过 `storage.cache_keep_days` 且不在本批构建名单里的会在下次挂载时回收；按模块清理先写 `.purge`，下次构建前执行。

## 六、跨平台构建脚本与会话内入口
- 构建命令写成模块内的脚本：unix 放 `tools/build`（要执行位），Windows 放 `tools/build.cmd`；两个文件并排，各自的壳自己找得到（Windows 的 cmd 按 PATHEXT，与 `npm` 的 shim 同一条规矩）。
- `.cmd` 只写 ASCII：cmd 按控制台代码页解释批处理字节，非 ASCII 的 `rem` 注释会把下一行整行吞掉，构建直接失败；`chcp 65001` 在管道 stdio 下也救不了。
- 失败判定用 `||`，不要用 `if errorlevel`：`g++ … || exit /b 1`；`if errorlevel 1` 的语义是「errorlevel ≥ 1」，编译器崩溃时退出码是负数（如 `0xC0000409`），判断为假、脚本以 0 退出，产品只能靠入口复验发现——归因会变得含糊。
- 说明文字写进模块 README，`--build` 与门禁都按这条判（结构审查的 platform 门禁 `tests/gates/structure/platform.js` 扫 `.cmd` / `.bat` 的非 ASCII 与 BOM）。
- 会话里要执行的相对入口（`tools.*.command` 的首 token）复制到 `/usr/local/lib/solomni/modules/<模块>/<入口>` 并给执行位，cwd 仍是共享的模块根（数据与 userdata 不动）——共享盘（cifs 的合成权限位）不承载执行位，脚本一律用解释器跑（`bash x.sh`）。

## 改动本单元时必须同步
- `MODULE_SPEC.md` → `build:` 字段契约；`RUNTIME_SPEC.md` → 运行包契约与 `kind` 装配。
- `src/tests/builds.rs` → 构建用例；`src/tests/diagnostics.rs` → `--build` 与体检。
- `docs/vm/README.md` → guest 起停、缓存卷挂载与共享档；`docs/storage/README.md` → 缓存回收与预算。
- `docs/kernel/README.md` → 容器临时区与长驻子进程的账。

## 本目录
| 文件 | 内容 |
| --- | --- |
| [`unit-map.md`](unit-map.md) | 逐文件职责（T0 与磁盘双向比对） |
