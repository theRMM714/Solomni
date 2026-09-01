# Solomni -- 环境 / 依赖说明

本文件说明如何把一个全新 clone 的仓库，通过一条命令装好开发依赖并运行。
跨平台（Windows / WSL / macOS）。bootstrap 用 Node 实现（Node 是 DSH 运行时，必然存在）。

## 前置要求
- Node.js >= 18（跑 bin/setup.mjs）。验证：node --version
- git（Flutter 工具强依赖；WSL/无 root 环境的用户级安装见文末附录）
- 装 Flutter 时，Linux 侧请先装桌面构建依赖（跑桌面窗口需要）：
    sudo apt install -y clang cmake ninja-build pkg-config libgtk-3-dev    # Debian/Ubuntu
    sudo pacman -S --needed clang cmake ninja pkg-config gtk3             # Arch

## 平台后缀目录（多平台共仓库互不覆盖）

SDK 与缓存按平台分目录，Windows 与 WSL 共用同一个仓库时互不覆盖、切换零成本：

    dart-sdk-windows/   dart-sdk-linux/   dart-sdk-macos/     # Dart SDK
    flutter-windows/    flutter-linux/    flutter-macos/      # Flutter SDK
    .pub-cache-<平台>/  .dart-home-<平台>/ .tmp-<平台>/        # 各平台缓存

旧版无后缀目录（dart-sdk/、flutter/、.pub-cache/…）会被 setup 自动迁移，无需手动处理。

## 一键安装（clone 后跑一次，可重复跑）
在仓库根目录执行：

    node bin/setup.mjs            # 只装 Dart SDK + pub get（纯 Dart 核心开发）
    node bin/setup.mjs --flutter  # 再加装 Flutter SDK（做桌面 UI 模块）

它会做：
1. 若当前平台缺 SDK，则按当前操作系统 / 架构下载并解压：
   - Dart SDK    -> dart-sdk-<platform>/dart-sdk/     （zip 内自带一层 dart-sdk/）
   - Flutter SDK -> flutter-<platform>/flutter/       （--flutter 时）
2. 建好该平台缓存目录：.pub-cache-<platform> / .dart-home-<platform> / .tmp-<platform>
3. 对 assistant/ 下每个含 pubspec.yaml 的包运行 dart pub get
   （--flutter 时对 Flutter 依赖包改用 flutter pub get）

这些 SDK / 缓存目录都已在 .gitignore 排除，绝不进 git。

## 之后怎么开发（无需手动改环境）
用仓库内包装命令，它们内部自动把 PUB_CACHE / TMP / HOME 指向仓库内：

    Windows:      bin\dart.bat    bin\flutter.bat
    WSL / Linux:  ./bin/dart      ./bin/flutter

示例（在对应目录下用相对路径调用 wrapper）：

    # 跑核心撮合 smoke test
    cd assistant/packages/core
    ../../../bin/dart tool/smoke_test.dart

    # 跑画布模型逻辑测试（ui_canvas）
    cd assistant/packages/ui_canvas
    ../../../bin/dart tool/canvas_test.dart

    # 跑产品验收与终端 REPL（CLI 交互）
    cd assistant/apps/solomni
    ../../../bin/dart tool/smoke.dart
    ../../../bin/dart bin/main.dart
    # 或者用启动器（等价，免去 cd）：
    ./bin/solomni

    # Flutter UI 模块
    cd assistant/apps/modules/ui_flutter
    ../../../../bin/flutter pub get
    ../../../../bin/flutter analyze

说明：wrapper 会先设置 PUB_CACHE/TMP/HOME 再调用本地 SDK，因此你永远不需要手动改
环境变量或路径；换平台同理，行为完全一致。

## 启动器

    Windows:      bin\solomni.bat [参数]
    WSL / Linux:  ./bin/solomni [参数]

等价于 `cd assistant/apps/solomni && dart bin/main.dart`，只是免去手敲长路径；
参数与退出码原样透传。启动器不含任何业务——模块的依赖与环境由各模块自己的
`dev`/`dev.bat` 契约脚本打理（见 assistant/MODULE_DEV_GUIDE.md）。

## 绝对路径纪律（跨平台可移植性）

仓库保持零 hosted 依赖（llm_gateway 用 dart:io HttpClient 直调，core 用 tool/ 自定义
测试规约），因此除 ui_flutter 外**所有包的 .dart_tool/package_config.json 都是纯相对
路径**——Windows 与 WSL 共用一个仓库目录时，这些包两侧通用，切侧零成本。

唯一例外 ui_flutter：`sdk: flutter` 依赖由 Flutter 工具链解析成本地 SDK 绝对路径
（工具链写死 .dart_tool 位置，无法相对化）。其 `dev`/`dev.bat` 启动脚本会在拉起前
**自检异侧配置并自动 pub get**，两侧切换对 `-gui` 启动无感；只有手动跑
`flutter analyze`/`flutter test` 时切侧才需要先 `flutter pub get` 一次。

## 版本 / 镜像覆盖
默认用国内 flutter-io 镜像，版本锁定为与当前开发一致的：
- Dart SDK:    3.12.2
- Flutter SDK: 3.38.2

如需换版本或官方源，用环境变量覆盖：

    DART_VERSION=3.12.2 FLUTTER_VERSION=3.38.2
    DART_BASE=https://storage.googleapis.com/dart-archive/channels/stable/release/
    FLUTTER_BASE=https://storage.googleapis.com/flutter_infra_release/releases/stable/

- Windows PowerShell： $env:DART_VERSION='3.12.2'; node bin\setup.mjs
- bash：              DART_VERSION=3.12.2 node bin/setup.mjs

## 为什么这样设计（与 git 的关系）
- git 只同步源码（assistant/ + 文档 + bin/）。
- 工具链 / SDK / 缓存 / 会话日志（session.jsonl）都 gitignore，各平台本地生成。
- Windows 与 WSL 可共用同一仓库目录（平台后缀目录互不覆盖），
  也可以各自 clone、各自跑一次 setup，行为完全一致；代码只通过 git 桥同步。
- .dart_tool/、build/ 是构建产物，各机器 pub get 时本地重建，不提交。

## 附录：WSL（Arch）无 root 装 git（Flutter 工具强依赖 git）

有 sudo 的直接：sudo pacman -S --needed git
（建议连同桌面构建工具链一次装齐：sudo pacman -S --needed git clang cmake ninja pkg-config gtk3）

无 sudo 时，下载 Arch 官方 git 包解压到用户目录（系统库 glibc/libcurl/libpcre2 等已内建，无需依赖）：

    mkdir -p ~/.local/opt && cd ~/.local/opt
    curl -LO "https://mirrors.tuna.tsinghua.edu.cn/archlinux/extra/os/x86_64/git-2.55.0-1-x86_64.pkg.tar.zst"
    tar --zstd -xf git-*-x86_64.pkg.tar.zst

解压后 git 位于 ~/.local/opt/usr/bin/git。仓库内 bin/flutter、bin/dart 会自动探测
该路径（无需改 PATH）；在自己的终端手动用 git 则加一行到 ~/.bashrc：

    export PATH="$HOME/.local/opt/usr/bin:$PATH"

版本号可到 https://mirrors.tuna.tsinghua.edu.cn/archlinux/extra/os/x86_64/ 查最新 git-*。
