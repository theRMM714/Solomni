# Solomni —— 环境 / 依赖说明

本文件说明如何把一个全新 clone 的仓库，通过一条命令装好开发依赖并运行。
跨平台（Windows / WSL / macOS）。bootstrap 用 Node 实现（Node 是 DSH 运行时，必然存在）。

## 前置要求
- Node.js >= 18（跑 bin/setup.mjs）。验证：node --version
- 装 Flutter 时，Linux 侧请先装桌面构建依赖：
    sudo apt install -y clang cmake ninja-build pkg-config libgtk-3-dev

## 一键安装（clone 后跑一次，可重复跑）
在仓库根目录执行：

    node bin/setup.mjs            # 只装 Dart SDK + pub get（纯 Dart 核心开发）
    node bin/setup.mjs --flutter  # 再加装 Flutter SDK（做桌面 UI 模块）

它会做：
1. 若仓库根缺少 SDK，则按当前操作系统 / 架构下载并解压：
   - Dart SDK    -> dart-sdk/           （zip 内自带一层 dart-sdk/）
   - Flutter SDK -> flutter/            （--flutter 时）
2. 建好仓库内缓存目录：.pub-cache / .dart-home / .tmp
3. 对 assistant/ 下每个含 pubspec.yaml 的包运行 dart pub get
   （--flutter 时对 apps/modules/ui_flutter 运行 flutter pub get）

这些 SDK / 缓存目录都已在 .gitignore 排除，绝不进 git。

## 之后怎么开发（无需手动改环境）
用仓库内包装命令，它们内部自动把 PUB_CACHE / TMP 指向仓库内：

    Windows:      bin\dart.bat    bin\flutter.bat
    WSL / Linux:  ./bin/dart       ./bin/flutter

示例（在对应目录下用相对路径调用 wrapper）：

    # 跑核心撮合 smoke test
    cd assistant/packages/core
    ../../../bin/dart tool/smoke_test.dart

    # 跑桌面协作演示（CLI 渲染器）
    cd assistant/apps/desktop_demo
    ../../../bin/dart bin/main.dart

说明：wrapper 会先设置 PUB_CACHE/TMP 再调用本地 SDK，因此你永远不需要手动改环境变量或路径；
换到 WSL 上同理，行为完全一致。

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
- 工具链 / SDK / 缓存 / 会话日志（session.jsonl）都 gitignore，两边各自本地生成。
- Windows 与 WSL 各自 clone、各自跑一次 setup，环境行为完全一致；代码只通过 git 桥同步。
- .dart_tool/、build/ 是构建产物，各机器 pub get 时本地重建，不提交。
